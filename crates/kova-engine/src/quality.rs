//! Connection quality: latency spread, jitter and loss from a series of probes, measured
//! through Kova's own proxy port while connected and directly otherwise.

use std::time::{Duration, Instant};

use serde::Serialize;

use crate::Error;

/// What a series of probes found. Times are in milliseconds.
#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Quality {
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
        proxied: false,
    }
}

fn client(proxy: Option<&str>, timeout: Duration) -> Result<reqwest::Client, Error> {
    let b = crate::fetch::builder().timeout(timeout).pool_max_idle_per_host(0);
    let b = match proxy {
        Some(p) => b.proxy(reqwest::Proxy::all(p)?),
        None => b.no_proxy(),
    };
    Ok(b.build()?)
}

/// `count` requests to `url`, one after another, each on a new connection like a
/// latency test. A request that fails or takes over `timeout` counts as lost; when the
/// first three are all lost, it stops there.
pub async fn probe(url: &str, proxy: Option<&str>, count: u32, timeout: Duration) -> Result<Quality, Error> {
    let c = client(proxy, timeout)?;
    let mut samples = Vec::with_capacity(count as usize);
    for i in 0..count {
        if i > 0 {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        let start = Instant::now();
        // Kova's own port answers 502 to 504 when the server can't be reached: lost too.
        let ok = c.get(url).send().await.is_ok_and(|r| proxy.is_none() || !matches!(r.status().as_u16(), 502..=504));
        samples.push(ok.then(|| start.elapsed().as_millis().min(u128::from(u32::MAX)) as u32));
        // Three lost in a row from the start: the connection is down, not unsteady, and
        // waiting out the rest would only take long.
        if samples.len() == 3 && samples.iter().all(Option::is_none) {
            break;
        }
    }
    Ok(Quality { proxied: proxy.is_some(), ..summarize(&samples) })
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

    /// A server that answers 204 to every request, like the usual latency test address.
    async fn answering() -> String {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut c, _)) = l.accept().await {
                tokio::spawn(async move {
                    use tokio::io::{AsyncReadExt, AsyncWriteExt};
                    let mut buf = [0u8; 1024];
                    let _ = c.read(&mut buf).await;
                    let _ = c.write_all(b"HTTP/1.1 204 No Content
Connection: close

").await;
                });
            }
        });
        format!("http://{addr}/generate_204")
    }

    #[tokio::test]
    async fn probes_a_live_and_a_dead_address() {
        let q = probe(&answering().await, None, 3, Duration::from_secs(2)).await.unwrap();
        assert_eq!((q.sent, q.answered, q.loss, q.proxied), (3, 3, 0.0, false));
        assert!(q.jitter.is_some());
        // Nothing listens on port 9 (discard) here: every request is lost.
        let dead = probe("http://127.0.0.1:9/", None, 2, Duration::from_secs(1)).await.unwrap();
        assert_eq!((dead.answered, dead.loss), (0, 100.0));
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
