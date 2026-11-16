//! "Add from another device": typing a subscription on a TV remote is painful, so Kova
//! serves a one-page form on the local network for a few minutes. The phone scans the
//! QR code, opens the page, pastes the subscription, and Kova adds it.
//!
//! The page lives at a random address on this device's network address only, accepts
//! one profile, and stops after that or after ten minutes.

use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::Arc;
use std::time::Duration;

use kova_engine::{Engine, Error};
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

/// The address other devices on the network reach this one at: the one the system would
/// use to go out. Connecting a UDP socket sends nothing.
pub fn lan_ip() -> Option<IpAddr> {
    let s = UdpSocket::bind("0.0.0.0:0").ok()?;
    s.connect("8.8.8.8:80").ok()?;
    let ip = s.local_addr().ok()?.ip();
    (!ip.is_loopback() && !ip.is_unspecified()).then_some(ip)
}

#[derive(Serialize, Clone)]
pub struct Receiving {
    pub url: String,
}

/// The most a request body may be: a large list of share links pasted as text is far less,
/// and a device on the network must not be able to fill the memory.
const MAX_BODY: usize = 256 * 1024;

/// How many requests are served at once (about 8 MB of bodies at most), and how many of
/// them one device may hold, so one device can't lock the real sender out. Others are
/// dropped without an answer.
const MAX_CONNECTIONS: usize = 32;
const MAX_PER_PEER: usize = 2;

/// The places for connections: a device's own count is kept beside the total.
struct Slots {
    all: Arc<tokio::sync::Semaphore>,
    peers: std::sync::Mutex<std::collections::HashMap<IpAddr, usize>>,
}

/// One connection's place, given back when dropped.
struct Slot {
    _permit: tokio::sync::OwnedSemaphorePermit,
    slots: Arc<Slots>,
    ip: IpAddr,
}

impl Slots {
    fn new() -> Arc<Self> {
        Arc::new(Slots { all: Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS)), peers: Default::default() })
    }

    fn admit(self: &Arc<Self>, ip: IpAddr) -> Option<Slot> {
        let ip = ip.to_canonical();
        let mut peers = self.peers.lock().unwrap();
        if peers.get(&ip).copied().unwrap_or(0) >= MAX_PER_PEER {
            return None;
        }
        let permit = self.all.clone().try_acquire_owned().ok()?;
        *peers.entry(ip).or_default() += 1;
        Some(Slot { _permit: permit, slots: self.clone(), ip })
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut peers = self.slots.peers.lock().unwrap();
        if let Some(n) = peers.get_mut(&self.ip) {
            *n -= 1;
            if *n == 0 {
                peers.remove(&self.ip);
            }
        }
    }
}

/// The running form, if any, so a second start or a stop finds it.
static CURRENT: std::sync::Mutex<Option<tauri::async_runtime::JoinHandle<()>>> = std::sync::Mutex::new(None);

pub fn stop() {
    if let Some(h) = CURRENT.lock().unwrap().take() {
        h.abort();
    }
}

pub async fn start(app: AppHandle, engine: Arc<Engine>, ru: bool) -> Result<Receiving, Error> {
    stop();
    let ip = lan_ip().ok_or_else(|| Error::Other("This device is not on a local network".into()))?;
    let listener = TcpListener::bind(SocketAddr::new(ip, 0)).await?;
    let token = uuid::Uuid::new_v4().simple().to_string();
    let url = format!("http://{}/{token}", listener.local_addr()?);
    let task = tauri::async_runtime::spawn(async move {
        let notify = app.clone();
        let added: Added = Arc::new(move |name: &str| {
            let _ = notify.emit("engine", &kova_engine::Event::Profiles);
            let _ = notify.emit("toast", format!("{} {name}", if ru { "Добавлено с другого устройства:" } else { "Added from another device:" }));
        });
        let (token, taken) = (Arc::new(token), Arc::new(tokio::sync::Mutex::new(false)));
        let done = Arc::new(tokio::sync::Notify::new());
        let slots = Slots::new();
        // Each request on its own: a device that connects and says nothing must not
        // keep the phone waiting.
        let serve = async {
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((sock, peer)) = accepted else { continue };
                        let Some(slot) = slots.admit(peer.ip()) else { continue };
                        let (token, engine, added, taken, done) = (token.clone(), engine.clone(), added.clone(), taken.clone(), done.clone());
                        tokio::spawn(async move {
                            let _slot = slot;
                            if handle(sock, &token, &engine, &*added, &taken, ru).await {
                                done.notify_one();
                            }
                        });
                    }
                    _ = done.notified() => return,
                }
            }
        };
        let _ = tokio::time::timeout(Duration::from_secs(600), serve).await;
        let _ = app.emit("receive-ended", ());
    });
    *CURRENT.lock().unwrap() = Some(task);
    Ok(Receiving { url })
}

type Added = Arc<dyn Fn(&str) + Send + Sync>;

/// How a request came in: whole, too large, or not whole before the time ran out or the
/// sender hung up.
enum Received {
    Whole(Vec<u8>, usize),
    TooLarge,
    Incomplete,
}

/// Read one request: the request line and headers within 3 seconds, the whole within 10.
/// The `usize` is where the headers end.
async fn receive(sock: &mut (impl AsyncReadExt + Unpin)) -> Received {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    let head = tokio::time::timeout(Duration::from_secs(3), async {
        loop {
            if let Some(end) = find(&buf, b"\r\n\r\n") {
                return Some(end);
            }
            let n = sock.read(&mut chunk).await.ok()?;
            if n == 0 || buf.len() > 16 * 1024 {
                return None;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
    })
    .await;
    let Ok(Some(end)) = head else { return Received::Incomplete };
    let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
    let len = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
    if len > MAX_BODY {
        return Received::TooLarge;
    }
    let rest = tokio::time::timeout(Duration::from_secs(7), async {
        while buf.len() < end + 4 + len {
            let n = sock.read(&mut chunk).await.ok()?;
            if n == 0 {
                return None;
            }
            buf.extend_from_slice(&chunk[..n]);
        }
        Some(())
    })
    .await;
    if matches!(rest, Ok(Some(()))) { Received::Whole(buf, end) } else { Received::Incomplete }
}

/// Serve one request. True once a profile was added; `taken` lets only one be.
async fn handle(mut sock: tokio::net::TcpStream, token: &str, engine: &Arc<Engine>, added: &(dyn Fn(&str) + Sync), taken: &tokio::sync::Mutex<bool>, ru: bool) -> bool {
    let (buf, end) = match receive(&mut sock).await {
        Received::Whole(buf, end) => (buf, end),
        Received::TooLarge => {
            let body = "That is too much text to add at once";
            let _ = sock.write_all(format!("HTTP/1.1 413 Payload Too Large\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).as_bytes()).await;
            // Read what is still coming first: closing with unread data makes the system
            // reset the connection, and the sender may then never see the answer.
            let _ = tokio::time::timeout(Duration::from_secs(2), async {
                let (mut left, mut chunk) = (1usize << 20, [0u8; 8192]);
                while left > 0 {
                    match sock.read(&mut chunk).await {
                        Ok(n) if n > 0 => left = left.saturating_sub(n),
                        _ => break
                    }
                }
            })
            .await;
            let _ = sock.shutdown().await;
            return false;
        }
        Received::Incomplete => return false,
    };
    let head = String::from_utf8_lossy(&buf[..end]).to_string();
    let first = head.lines().next().unwrap_or_default().to_string();
    let path = format!("/{token}");
    let t = |en: &'static str, rus: &'static str| if ru { rus } else { en };

    let (status, body, added) = if first.starts_with(&format!("GET {path} ")) {
        ("200 OK", page(t("Add a profile to Kova", "Добавить профиль в Kova"), &form(&path, ru)), false)
    } else if first.starts_with(&format!("POST {path} ")) {
        let text = form_field(&String::from_utf8_lossy(&buf[end + 4..]), "text");
        let mut taken = taken.lock().await;
        let result = if *taken { Err(Error::Other(t("A profile was already added from this page", "С этой страницы профиль уже добавлен").into())) } else { engine.add_profile(text.trim(), None).await };
        match result {
            Ok(p) => {
                *taken = true;
                added(&p.name);
                ("200 OK", page(t("Added", "Добавлено"), &format!("<p>{} <b>{}</b>.</p>", t("Kova added", "Kova добавила"), esc(&p.name))), true)
            }
            Err(e) => ("200 OK", page(t("Not added", "Не добавлено"), &format!("<p>{}</p>{}", esc(&e.to_string()), form(&path, ru))), false),
        }
    } else {
        ("404 Not Found", String::new(), false)
    };
    let reply = format!("HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = sock.write_all(reply.as_bytes()).await;
    let _ = sock.shutdown().await;
    added
}

fn find(hay: &[u8], needle: &[u8]) -> Option<usize> {
    hay.windows(needle.len()).position(|w| w == needle)
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// One field of an `application/x-www-form-urlencoded` body.
fn form_field(body: &str, name: &str) -> String {
    body.split('&')
        .filter_map(|kv| kv.split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| {
            let v = v.replace('+', " ");
            let bytes = v.as_bytes();
            let mut out = Vec::with_capacity(bytes.len());
            let mut i = 0;
            while i < bytes.len() {
                // Bytes, not a slice of the text: a `%` before a non-ASCII letter would cut
                // it in half, which panics (and with panic = abort, closes Kova).
                if bytes[i] == b'%' && i + 2 < bytes.len()
                    && let (Some(hi), Some(lo)) = ((bytes[i + 1] as char).to_digit(16), (bytes[i + 2] as char).to_digit(16)) {
                        out.push((hi * 16 + lo) as u8);
                        i += 3;
                        continue;
                    }
                out.push(bytes[i]);
                i += 1;
            }
            String::from_utf8_lossy(&out).into_owned()
        })
        .unwrap_or_default()
}

fn form(path: &str, ru: bool) -> String {
    let (label, button) = if ru {
        ("Ссылка на подписку или ссылки на серверы", "Добавить в Kova")
    } else {
        ("Subscription link or share links", "Add to Kova")
    };
    format!(
        "<form method=\"post\" action=\"{path}\"><label for=\"t\">{label}</label><textarea id=\"t\" name=\"text\" rows=\"6\" required autofocus></textarea><button>{button}</button></form>"
    )
}

fn page(title: &str, content: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><title>{title}</title>\
<style>body{{margin:0;background:#101012;color:#ededef;font:16px/1.5 system-ui,sans-serif;padding:24px}}h1{{font-size:20px}}label{{display:block;color:#9b9ba3;margin:16px 0 8px}}\
textarea{{width:100%;box-sizing:border-box;background:#17171a;color:#ededef;border:1px solid #323238;border-radius:10px;padding:10px;font:14px monospace}}\
button{{margin-top:12px;width:100%;height:48px;border:0;border-radius:10px;background:#f2f2f4;color:#111113;font-weight:600;font-size:16px}}</style></head>\
<body><h1>{title}</h1>{content}</body></html>"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A phone opens the page, then posts a share link; a wrong address gets nothing.
    #[tokio::test]
    async fn adds_what_another_device_posts() {
        let root = tempfile::tempdir().unwrap();
        let engine = Engine::new(root.path()).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = std::sync::Mutex::new(Vec::<String>::new());
        let added = |n: &str| seen.lock().unwrap().push(n.to_string());
        let taken = tokio::sync::Mutex::new(false);
        async fn send(addr: SocketAddr, req: String) -> String {
            let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
            s.write_all(req.as_bytes()).await.unwrap();
            let mut out = String::new();
            s.read_to_string(&mut out).await.unwrap();
            out
        }
        // A request line, headers and body, with the CRLF line ends HTTP needs.
        fn http(line: &str, headers: &[&str], body: &str) -> String {
            let crlf = "\r\n";
            let mut out = format!("{line} HTTP/1.1{crlf}Host: x{crlf}");
            for h in headers {
                out.push_str(h);
                out.push_str(crlf);
            }
            out.push_str(crlf);
            out.push_str(body);
            out
        }
        let link = "trojan://pw@example.com:443#Phone%20node";
        let body = format!("text={}", link.replace(':', "%3A").replace('/', "%2F").replace('@', "%40").replace('#', "%23"));
        let requests = [
            (http("GET /wrong", &[], ""), "404", false),
            (http("GET /tok", &[], ""), "<form", false),
            (http("POST /tok", &[&format!("Content-Length: {}", body.len())], &body), "Phone node", true),
            (http("POST /tok", &[&format!("Content-Length: {}", body.len())], &body), "already added", false),
        ];
        for (req, expect, done) in requests {
            let reply = tokio::spawn(send(addr, req));
            let (sock, _) = listener.accept().await.unwrap();
            assert_eq!(handle(sock, "tok", &engine, &added, &taken, false).await, done);
            let reply = reply.await.unwrap();
            assert!(reply.contains(expect), "{reply}");
        }
        assert_eq!(*seen.lock().unwrap(), ["Phone node"]);
        assert_eq!(engine.profiles().len(), 1);
    }

    #[test]
    fn a_device_holds_few_places_and_the_total_is_limited() {
        let slots = Slots::new();
        let a: IpAddr = "192.168.1.5".parse().unwrap();
        let first = slots.admit(a).unwrap();
        let _second = slots.admit(a).unwrap();
        assert!(slots.admit(a).is_none(), "a third from one device");
        assert!(slots.admit("::ffff:192.168.1.5".parse().unwrap()).is_none(), "the same device by its mapped address");
        assert!(slots.admit("192.168.1.6".parse().unwrap()).is_some(), "another device is not held up");
        drop(first);
        assert!(slots.admit(a).is_some(), "a place given back");
        let held: Vec<_> = (0..MAX_CONNECTIONS).filter_map(|i| slots.admit(format!("10.0.{}.{}", i / 2 % 250, 1 + i % 2).parse().unwrap())).collect();
        assert!(held.len() < MAX_CONNECTIONS && slots.admit("10.9.9.9".parse().unwrap()).is_none(), "the total is full");
    }

    #[tokio::test]
    async fn a_request_is_whole_too_large_or_incomplete() {
        async fn feed(bytes: &'static [u8]) -> Received {
            let (mut client, mut server) = tokio::io::duplex(1 << 20);
            client.write_all(bytes).await.unwrap();
            drop(client);
            receive(&mut server).await
        }
        assert!(matches!(feed(b"POST /t HTTP/1.1\r\nContent-Length: 4\r\n\r\ntext").await, Received::Whole(_, _)));
        assert!(matches!(feed(b"POST /t HTTP/1.1\r\nContent-Length: 999999\r\n\r\ntext").await, Received::TooLarge));
        assert!(matches!(feed(b"POST /t HTTP/1.1\r\nContent-Length: 40\r\n\r\ntext").await, Received::Incomplete), "hung up half way");
        assert!(matches!(feed(b"POST /t HTTP/1.1\r\nContent-").await, Received::Incomplete));
    }

    #[tokio::test]
    async fn an_oversized_post_is_answered_with_413_and_adds_nothing() {
        let root = tempfile::tempdir().unwrap();
        let engine = Engine::new(root.path()).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let client = tokio::spawn(async move {
            let mut s = tokio::net::TcpStream::connect(addr).await.unwrap();
            let body = vec![b'a'; 600 * 1024];
            let head = format!("POST /tok HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n", body.len());
            s.write_all(head.as_bytes()).await.unwrap();
            let _ = s.write_all(&body).await;
            let _ = s.shutdown().await;
            let mut out = String::new();
            let _ = s.read_to_string(&mut out).await;
            out
        });
        let (sock, _) = listener.accept().await.unwrap();
        let taken = tokio::sync::Mutex::new(false);
        assert!(!handle(sock, "tok", &engine, &|_: &str| {}, &taken, false).await);
        let reply = client.await.unwrap();
        assert!(reply.starts_with("HTTP/1.1 413"), "{reply:?}");
        assert_eq!(engine.profiles().len(), 0);
    }

    #[test]
    fn reads_form_fields() {
        assert_eq!(form_field("text=vless%3A%2F%2Fa%40b%3A443%23My+node&x=1", "text"), "vless://a@b:443#My node");
        assert_eq!(form_field("text=%D0%9A", "text"), "К");
        assert_eq!(form_field("other=1", "text"), "");
        assert_eq!(form_field("text=%a\u{e9}%zz%4", "text"), "%a\u{e9}%zz%4", "broken escapes stay as they are");
        assert_eq!(esc("<b>\"&"), "&lt;b&gt;&quot;&amp;");
    }
}
