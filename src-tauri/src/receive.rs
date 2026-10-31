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
        // Each request on its own: a device that connects and says nothing must not
        // keep the phone waiting.
        let serve = async {
            loop {
                tokio::select! {
                    accepted = listener.accept() => {
                        let Ok((sock, _)) = accepted else { continue };
                        let (token, engine, added, taken, done) = (token.clone(), engine.clone(), added.clone(), taken.clone(), done.clone());
                        tokio::spawn(async move {
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

/// Serve one request. True once a profile was added; `taken` lets only one be.
async fn handle(mut sock: tokio::net::TcpStream, token: &str, engine: &Arc<Engine>, added: &(dyn Fn(&str) + Sync), taken: &tokio::sync::Mutex<bool>, ru: bool) -> bool {
    let mut buf = Vec::new();
    let read = tokio::time::timeout(Duration::from_secs(10), async {
        let mut chunk = [0u8; 8192];
        loop {
            let n = sock.read(&mut chunk).await?;
            if n == 0 {
                break;
            }
            buf.extend_from_slice(&chunk[..n]);
            if let Some(end) = find(&buf, b"\r\n\r\n") {
                let head = String::from_utf8_lossy(&buf[..end]).to_ascii_lowercase();
                let len = head.lines().find_map(|l| l.strip_prefix("content-length:")).and_then(|v| v.trim().parse::<usize>().ok()).unwrap_or(0);
                if len > 1 << 20 || buf.len() >= end + 4 + len {
                    break;
                }
            }
            if buf.len() > (1 << 20) + 8192 {
                break;
            }
        }
        std::io::Result::Ok(())
    })
    .await;
    if !matches!(read, Ok(Ok(()))) {
        return false;
    }
    let Some(end) = find(&buf, b"\r\n\r\n") else { return false };
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
    fn reads_form_fields() {
        assert_eq!(form_field("text=vless%3A%2F%2Fa%40b%3A443%23My+node&x=1", "text"), "vless://a@b:443#My node");
        assert_eq!(form_field("text=%D0%9A", "text"), "К");
        assert_eq!(form_field("other=1", "text"), "");
        assert_eq!(form_field("text=%a\u{e9}%zz%4", "text"), "%a\u{e9}%zz%4", "broken escapes stay as they are");
        assert_eq!(esc("<b>\"&"), "&lt;b&gt;&quot;&amp;");
    }
}
