//! Connection quality: latency spread, jitter and loss from a series of probes, measured
//! through Kova's own proxy port while connected and directly otherwise.
//!
//! The first request opens the connection (through the server when connected) and is
//! timed on its own. The rest go over that same connection, so each one is a single
//! round trip, the closest thing to ping that works through a proxy.

use std::time::{Duration, Instant};

use serde::Serialize;

use crate::Error;

/// What a series of probes found. Times are in milliseconds.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Quality {
    /// The first request, which also sets up the connection.
    pub connect: Option<u32>,
    /// Probes sent and probes answered.
    pub sent: u32,
    pub answered: u32,
    pub min: Option<u32>,
    pub median: Option<u32>,
    pub avg: Option<u32>,
    pub max: Option<u32>,
    /// Mean change between one answered probe and the next.
    pub jitter: Option<u32>,
    /// Share of probes that got no answer in time, 0 to 100.
    pub loss: f32,
    /// Measured through the proxy (false: directly).
    pub proxied: bool,
}

/// Sums up probe results in the order they were taken; `None` is a probe without an answer.
pub fn summarize(samples: &[Option<u32>]) -> Quality {
    let got: Vec<u32> = samples.iter().flatten().copied().collect();
    let mut sorted = got.clone();
    sorted.sort_unstable();
    let median = (!sorted.is_empty()).then(|| {
        let n = sorted.len();
        if n % 2 == 1 { sorted[n / 2] } else { (sorted[n / 2 - 1] + sorted[n / 2]).div_ceil(2) }
    });
    let jitter = (got.len() >= 2).then(|| {
        let total: u64 = got.windows(2).map(|w| u64::from(w[0].abs_diff(w[1]))).sum();
        (total as f64 / (got.len() - 1) as f64).round() as u32
    });
    Quality {
        sent: samples.len() as u32,
        answered: got.len() as u32,
        min: sorted.first().copied(),
        median,
        avg: (!got.is_empty()).then(|| (got.iter().map(|&d| u64::from(d)).sum::<u64>() as f64 / got.len() as f64).round() as u32),
        max: sorted.last().copied(),
        jitter,
        loss: if samples.is_empty() { 0.0 } else { (samples.len() - got.len()) as f32 * 100.0 / samples.len() as f32 },
        connect: None,
        proxied: false,
    }
}

fn client(proxy: Option<&str>, timeout: Duration) -> Result<reqwest::Client, Error> {
    let b = crate::fetch::builder().timeout(timeout).pool_idle_timeout(Duration::from_secs(30));
    let b = match proxy {
        Some(p) => b.proxy(reqwest::Proxy::all(p)?),
        None => b.no_proxy(),
    };
    Ok(b.build()?)
}

const fn ms(d: Duration) -> u32 {
    let m = d.as_millis();
    if m > u32::MAX as u128 { u32::MAX } else { m as u32 }
}

/// One kept-open connection for plain `http://` addresses: a CONNECT tunnel through the
/// proxy port (an HTTP proxy would otherwise open a new connection to the server for
/// every request), or a direct TCP connection.
struct Plain {
    host: String,
    port: u16,
    path: String,
    proxy: Option<(String, Option<String>)>,
    stream: Option<tokio::io::BufReader<tokio::net::TcpStream>>,
}

impl Plain {
    fn new(url: &reqwest::Url, proxy: Option<&str>) -> Option<Self> {
        let proxy = match proxy {
            Some(p) => {
                let p = reqwest::Url::parse(p).ok()?;
                let auth = (!p.username().is_empty()).then(|| {
                    use base64::Engine as _;
                    base64::engine::general_purpose::STANDARD.encode(format!("{}:{}", p.username(), p.password().unwrap_or_default()))
                });
                Some((format!("{}:{}", p.host_str()?, p.port_or_known_default()?), auth))
            }
            None => None,
        };
        let path = match url.query() {
            Some(q) => format!("{}?{q}", url.path()),
            None => url.path().to_string(),
        };
        Some(Self { host: url.host_str()?.to_string(), port: url.port_or_known_default()?, path, proxy, stream: None })
    }

    async fn open(&self) -> std::io::Result<tokio::io::BufReader<tokio::net::TcpStream>> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
        let target = format!("{}:{}", self.host, self.port);
        let Some((addr, auth)) = &self.proxy else {
            return Ok(tokio::io::BufReader::new(tokio::net::TcpStream::connect(&target).await?));
        };
        let mut s = tokio::io::BufReader::new(tokio::net::TcpStream::connect(addr).await?);
        let auth = auth.as_ref().map(|a| format!("Proxy-Authorization: Basic {a}\r\n")).unwrap_or_default();
        s.get_mut().write_all(format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\n{auth}\r\n").as_bytes()).await?;
        let mut line = String::new();
        s.read_line(&mut line).await?;
        if !line.split_whitespace().nth(1).is_some_and(|c| c.starts_with('2')) {
            return Err(std::io::Error::other(format!("proxy refused: {}", line.trim())));
        }
        loop {
            line.clear();
            if s.read_line(&mut line).await? == 0 || line == "\r\n" || line == "\n" {
                break;
            }
        }
        Ok(s)
    }

    /// One request; reconnects first when the last answer closed the connection.
    async fn request(&mut self) -> std::io::Result<()> {
        use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};
        if self.stream.is_none() {
            self.stream = Some(self.open().await?);
        }
        let s = self.stream.as_mut().expect("opened above");
        let req = format!("GET {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: Kova\r\nAccept: */*\r\n\r\n", self.path, self.host);
        let res: std::io::Result<bool> = async {
            s.get_mut().write_all(req.as_bytes()).await?;
            let mut line = String::new();
            if s.read_line(&mut line).await? == 0 {
                return Err(std::io::ErrorKind::UnexpectedEof.into());
            }
            let status: u16 = line.split_whitespace().nth(1).and_then(|c| c.parse().ok()).ok_or(std::io::ErrorKind::InvalidData)?;
            let (mut length, mut keep) = (None::<u64>, true);
            loop {
                line.clear();
                if s.read_line(&mut line).await? == 0 || line == "\r\n" || line == "\n" {
                    break;
                }
                let l = line.to_ascii_lowercase();
                if let Some(v) = l.strip_prefix("content-length:") {
                    length = v.trim().parse().ok();
                } else if l.starts_with("connection:") && l.contains("close") {
                    keep = false;
                }
            }
            match (status, length) {
                (204 | 304, _) | (_, Some(0)) => {}
                (_, Some(n)) => {
                    let mut body = (&mut *s).take(n);
                    tokio::io::copy(&mut body, &mut tokio::io::sink()).await?;
                }
                // No length: the body runs to the end of the connection, so it can't be reused.
                (_, None) => keep = false,
            }
            Ok(keep)
        }
        .await;
        match res {
            Ok(true) => Ok(()),
            Ok(false) => {
                self.stream = None;
                Ok(())
            }
            Err(e) => {
                self.stream = None;
                Err(e)
            }
        }
    }
}

/// The first request to `url` sets up the connection and is timed on its own; then
/// `count` more go one after another over that same connection. A request that fails or
/// takes over `timeout` counts as lost (the next one reconnects); when the setup and the
/// first two all fail, it stops there.
pub async fn probe(url: &str, proxy: Option<&str>, count: u32, timeout: Duration) -> Result<Quality, Error> {
    let parsed = reqwest::Url::parse(url).map_err(|e| Error::Other(format!("test address: {e}")))?;
    let mut plain = if parsed.scheme() == "http" { Plain::new(&parsed, proxy) } else { None };
    let c = client(proxy, timeout)?;
    let mut once = async || -> Option<u32> {
        let start = Instant::now();
        let ok = match plain.as_mut() {
            Some(p) => tokio::time::timeout(timeout, p.request()).await.is_ok_and(|r| r.is_ok()),
            // Kova's own port answers 502 to 504 when the server can't be reached: lost too.
            None => match c.get(url).send().await {
                Ok(r) => {
                    let bad = proxy.is_some() && matches!(r.status().as_u16(), 502..=504);
                    r.bytes().await.is_ok() && !bad
                }
                Err(_) => false,
            },
        };
        if !ok && let Some(p) = plain.as_mut() {
            p.stream = None;
        }
        ok.then(|| ms(start.elapsed()))
    };
    let connect = once().await;
    let mut samples = Vec::with_capacity(count as usize);
    for _ in 0..count {
        tokio::time::sleep(Duration::from_millis(200)).await;
        samples.push(once().await);
        // Setup and two more lost: the connection is down, not unsteady.
        if connect.is_none() && samples.len() == 2 && samples.iter().all(Option::is_none) {
            break;
        }
    }
    Ok(Quality { connect, proxied: proxy.is_some(), ..summarize(&samples) })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sums_up_probes() {
        let q = summarize(&[Some(100), None, Some(120), Some(110), None]);
        assert_eq!((q.sent, q.answered), (5, 3));
        assert_eq!((q.min, q.median, q.avg, q.max), (Some(100), Some(110), Some(110), Some(120)));
        // Changes between answered probes: 20 and 10.
        assert_eq!(q.jitter, Some(15));
        assert_eq!(q.loss, 40.0);
    }

    /// A server that answers 204 to every request on a kept-open connection, like the
    /// usual latency test address, and counts the connections made to it.
    async fn answering() -> (String, std::sync::Arc<std::sync::atomic::AtomicUsize>) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        let conns = std::sync::Arc::new(AtomicUsize::new(0));
        let counted = conns.clone();
        tokio::spawn(async move {
            while let Ok((c, _)) = l.accept().await {
                counted.fetch_add(1, Ordering::SeqCst);
                tokio::spawn(async move {
                    use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
                    let mut c = tokio::io::BufReader::new(c);
                    let mut line = String::new();
                    loop {
                        line.clear();
                        match c.read_line(&mut line).await {
                            Ok(0) | Err(_) => return,
                            Ok(_) if line == "\r\n" => {
                                if c.get_mut().write_all(b"HTTP/1.1 204 No Content\r\n\r\n").await.is_err() {
                                    return;
                                }
                            }
                            Ok(_) => {}
                        }
                    }
                });
            }
        });
        (format!("http://{addr}/generate_204"), conns)
    }

    #[tokio::test]
    async fn probes_a_live_and_a_dead_address() {
        let (url, conns) = answering().await;
        let q = probe(&url, None, 3, Duration::from_secs(2)).await.unwrap();
        // The setup and all three requests used one connection.
        assert_eq!(conns.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert_eq!((q.sent, q.answered, q.loss, q.proxied), (3, 3, 0.0, false));
        assert!(q.connect.is_some() && q.jitter.is_some());
        // Nothing listens on port 9 (discard) here: every request is lost.
        let dead = probe("http://127.0.0.1:9/", None, 5, Duration::from_secs(1)).await.unwrap();
        assert_eq!((dead.connect, dead.sent, dead.answered, dead.loss), (None, 2, 0, 100.0));
    }

    #[test]
    fn nothing_answered() {
        let q = summarize(&[None, None]);
        assert_eq!((q.median, q.jitter, q.loss), (None, None, 100.0));
        let one = summarize(&[Some(50)]);
        assert_eq!((one.median, one.jitter, one.loss), (Some(50), None, 0.0));
        assert_eq!(summarize(&[Some(10), Some(21)]).median, Some(16));
    }
}
