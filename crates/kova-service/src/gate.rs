//! The gate in front of the service's Mihomo controller.
//!
//! Kova drives Mihomo through its controller, and the controller's secret sits in the
//! user's data folder, where every program of that account can read it. With the
//! controller itself, that secret would let any of them load a config of its own into
//! the Mihomo that runs with system rights (`PUT /configs`), past everything
//! `sanitize_for_service` takes out, or have it replace or restart itself.
//!
//! So the service gives Mihomo a controller port and a secret of its own, known to
//! nobody else, and listens here on the port Kova asked for, with Kova's secret. Only
//! the requests Kova makes are passed on; the rest are refused.
//!
//! Each connection carries one request. Whatever else a client sends on it is thrown
//! away, so a second request can't ride in behind an allowed one.

use std::sync::Arc;
use std::time::Duration;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// A request's first line and headers together; Kova's are a few hundred bytes.
const MAX_HEAD: usize = 16 * 1024;
/// Kova's largest request body is a server's name.
const MAX_BODY: usize = 64 * 1024;
/// A client that connects and says nothing must not hold a task forever.
const SLOW: Duration = Duration::from_secs(10);

/// What Kova may read: the parts of the controller that only report.
const READS: &[&str] = &["version", "proxies", "providers", "group", "dns", "connections", "traffic", "memory", "logs"];

/// Both ends of the gate.
pub struct Keys {
    /// The secret Kova sends: the one in the config it asked the service to run.
    pub client_secret: String,
    /// Where Mihomo's controller really listens, on loopback.
    pub upstream_port: u16,
    /// The secret of that controller, made by the service for this run.
    pub upstream_secret: String,
}

/// The gate's listener on `port`, on loopback only.
pub async fn bind(port: u16) -> std::io::Result<TcpListener> {
    TcpListener::bind(("127.0.0.1", port)).await
}

/// Pass Kova's requests on to Mihomo until the task is dropped.
pub async fn serve(listener: TcpListener, keys: Arc<Keys>) {
    loop {
        let Ok((client, _)) = listener.accept().await else {
            tokio::time::sleep(Duration::from_millis(100)).await;
            continue;
        };
        let keys = keys.clone();
        tokio::spawn(async move { handle(client, &keys).await });
    }
}

/// A request's first line and the few headers the gate passes on.
#[derive(Debug, PartialEq)]
struct Head {
    method: String,
    /// The path and query, as sent.
    target: String,
    content_length: usize,
    content_type: Option<String>,
    accept: Option<String>,
    bearer: Option<String>,
}

/// Read a request's head. Anything a strict reader and Mihomo could understand
/// differently is refused rather than passed on: a body without a plain length, a
/// header given twice, a connection asking to become something else.
fn parse_head(text: &str) -> Option<Head> {
    let mut lines = text.split("\r\n");
    let mut first = lines.next()?.split(' ');
    let (method, target, version) = (first.next()?, first.next()?, first.next()?);
    if first.next().is_some() || version != "HTTP/1.1" || method.is_empty() || !method.bytes().all(|b| b.is_ascii_uppercase()) {
        return None;
    }
    if !target.starts_with('/') || !target.bytes().all(|b| (0x21..=0x7e).contains(&b)) {
        return None;
    }
    let mut head = Head { method: method.to_string(), target: target.to_string(), content_length: 0, content_type: None, accept: None, bearer: None };
    let mut length_seen = false;
    for line in lines {
        let (name, value) = line.split_once(':')?;
        let token = |b: u8| b.is_ascii_alphanumeric() || b"-_".contains(&b);
        if name.is_empty() || !name.bytes().all(token) || !value.bytes().all(|b| b == b'\t' || (0x20..=0x7e).contains(&b)) {
            return None;
        }
        let value = value.trim();
        match name.to_ascii_lowercase().as_str() {
            "transfer-encoding" | "upgrade" | "expect" => return None,
            "content-length" => {
                if length_seen || value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                    return None;
                }
                length_seen = true;
                head.content_length = value.parse().ok().filter(|n| *n <= MAX_BODY)?;
            }
            "authorization" => {
                if head.bearer.is_some() {
                    return None;
                }
                head.bearer = Some(value.strip_prefix("Bearer ").unwrap_or_default().to_string());
            }
            "content-type" => head.content_type = Some(value.to_string()),
            "accept" => head.accept = Some(value.to_string()),
            _ => {}
        }
    }
    Some(head)
}

/// Whether Kova makes this request. A list of what is allowed, not of what is
/// dangerous: a controller route added by a later Mihomo stays closed until it is
/// named here.
fn allowed(method: &str, target: &str) -> bool {
    let path = target.split('?').next().unwrap_or_default();
    let Some(rest) = path.strip_prefix('/') else { return false };
    let parts: Vec<&str> = rest.split('/').collect();
    // No empty part and no `.` or `..`, however it is written: nothing that could
    // name another route than the one it looks like.
    let dots = |s: &str| matches!(s.replace("%2e", ".").replace("%2E", ".").as_str(), "." | "..");
    if parts.iter().any(|&s| s.is_empty() || dots(s)) {
        return false;
    }
    match (method, parts.as_slice()) {
        ("GET", [first, ..]) => READS.contains(first),
        // Choosing a server in a group, and fetching a rule list again.
        ("PUT", ["proxies", _]) | ("PUT", ["providers", "rules", _]) => true,
        ("DELETE", ["connections"]) | ("DELETE", ["connections", _]) => true,
        // The mode only; see `mode_only`.
        ("PATCH", ["configs"]) => true,
        _ => false,
    }
}

/// Whether a `PATCH /configs` body changes the mode (rule, global, direct) and nothing
/// else. The same request can turn TUN on, open ports to the network and more.
fn mode_only(body: &[u8]) -> bool {
    matches!(serde_json::from_slice::<serde_json::Value>(body), Ok(serde_json::Value::Object(m)) if m.len() == 1 && m.get("mode").is_some_and(|v| v.is_string()))
}

/// Compare without stopping at the first difference, so timing says nothing of the secret.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len() && a.bytes().zip(b.bytes()).fold(0u8, |d, (x, y)| d | (x ^ y)) == 0
}

/// The request's head and whatever of its body came with it.
async fn read_head(client: &mut TcpStream) -> Option<(String, Vec<u8>)> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 4096];
    loop {
        if let Some(end) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
            let rest = buf.split_off(end + 4);
            buf.truncate(end);
            return Some((String::from_utf8(buf).ok()?, rest));
        }
        if buf.len() > MAX_HEAD {
            return None;
        }
        let n = client.read(&mut chunk).await.ok()?;
        if n == 0 {
            return None;
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

async fn refuse(client: &mut TcpStream, status: &str, message: &str) {
    let body = serde_json::json!({ "message": message }).to_string();
    let reply = format!("HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
    let _ = client.write_all(reply.as_bytes()).await;
    let _ = client.shutdown().await;
}

async fn handle(mut client: TcpStream, keys: &Keys) {
    let Ok(Some((text, mut body))) = tokio::time::timeout(SLOW, read_head(&mut client)).await else { return };
    let Some(head) = parse_head(&text) else { return refuse(&mut client, "400 Bad Request", "the Kova service did not understand the request").await };
    if !head.bearer.as_deref().is_some_and(|b| same(b, &keys.client_secret)) {
        return refuse(&mut client, "401 Unauthorized", "Unauthorized").await;
    }
    if !allowed(&head.method, &head.target) {
        return refuse(&mut client, "403 Forbidden", "the Kova service does not pass this request on to Mihomo").await;
    }
    let rest = async {
        let mut chunk = [0u8; 4096];
        while body.len() < head.content_length {
            match client.read(&mut chunk).await {
                Ok(n) if n > 0 => body.extend_from_slice(&chunk[..n]),
                _ => return false,
            }
        }
        true
    };
    if !matches!(tokio::time::timeout(SLOW, rest).await, Ok(true)) {
        return;
    }
    // Only the body the request declared: what follows it is not passed on.
    body.truncate(head.content_length);
    if head.method == "PATCH" && !mode_only(&body) {
        return refuse(&mut client, "403 Forbidden", "the Kova service lets only the mode be changed this way").await;
    }

    let Ok(Ok(mut upstream)) = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(("127.0.0.1", keys.upstream_port))).await else {
        return refuse(&mut client, "502 Bad Gateway", "Mihomo's controller does not answer yet").await;
    };
    // Written anew from what was read, with Mihomo's own secret, and closed after one
    // answer: none of the client's own header lines reach Mihomo.
    let mut out = format!(
        "{} {} HTTP/1.1\r\nHost: 127.0.0.1:{}\r\nAuthorization: Bearer {}\r\nConnection: close\r\n",
        head.method, head.target, keys.upstream_port, keys.upstream_secret
    );
    for (name, value) in [("Content-Type", &head.content_type), ("Accept", &head.accept)] {
        if let Some(v) = value {
            out.push_str(&format!("{name}: {v}\r\n"));
        }
    }
    if head.method != "GET" || head.content_length > 0 {
        out.push_str(&format!("Content-Length: {}\r\n", body.len()));
    }
    out.push_str("\r\n");
    let mut request = out.into_bytes();
    request.extend_from_slice(&body);
    if upstream.write_all(&request).await.is_err() {
        return refuse(&mut client, "502 Bad Gateway", "Mihomo's controller closed the connection").await;
    }

    // Mihomo's answer goes back as it is, for as long as it lasts (traffic and logs
    // are endless). The client's side is only watched for its end.
    let (mut from_client, mut to_client) = client.split();
    let (mut from_upstream, _to_upstream) = upstream.split();
    {
        let answer = tokio::io::copy(&mut from_upstream, &mut to_client);
        tokio::pin!(answer);
        let mut unread = [0u8; 1024];
        loop {
            tokio::select! {
                _ = &mut answer => break,
                n = from_client.read(&mut unread) => {
                    if !matches!(n, Ok(n) if n > 0) {
                        return;
                    }
                }
            }
        }
    }
    let _ = to_client.shutdown().await;
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn kovas_requests_pass_and_the_rest_do_not() {
        for (method, target) in [
            ("GET", "/version"),
            ("GET", "/proxies"),
            ("GET", "/proxies/Fastest%20in%20US/delay?timeout=3000&url=http%3A%2F%2Fwww.gstatic.com%2Fgenerate_204"),
            ("GET", "/providers/proxies/sub%2Fone/node/healthcheck?timeout=3000&url=x"),
            ("GET", "/group/PROXY/delay?timeout=3000&url=x"),
            ("GET", "/dns/query?name=example.com&type=A"),
            ("GET", "/connections"),
            ("GET", "/traffic"),
            ("GET", "/memory"),
            ("GET", "/logs?level=info"),
            ("PUT", "/proxies/PROXY"),
            ("PUT", "/providers/rules/kova-smart-proxy"),
            ("DELETE", "/connections"),
            ("DELETE", "/connections/3f1c"),
            ("PATCH", "/configs"),
        ] {
            assert!(allowed(method, target), "{method} {target}");
        }
        for (method, target) in [
            ("PUT", "/configs"),
            ("PUT", "/configs?force=true"),
            ("POST", "/configs/geo"),
            ("GET", "/configs"),
            ("POST", "/restart"),
            ("POST", "/upgrade"),
            ("POST", "/upgrade/ui"),
            ("GET", "/debug/pprof/heap"),
            ("PUT", "/debug/gc"),
            ("POST", "/cache/fakeip/flush"),
            ("PUT", "/providers/proxies/sub"),
            ("PATCH", "/configs/"),
            ("PATCH", "/proxies/PROXY"),
            ("DELETE", "/proxies/PROXY"),
            ("PUT", "/proxies/../configs"),
            ("PUT", "/proxies/%2e%2E/configs"),
            ("PUT", "/proxies//configs"),
            ("PUT", "//configs"),
            ("GET", "/"),
            ("GET", "version"),
            ("OPTIONS", "/version"),
        ] {
            assert!(!allowed(method, target), "{method} {target}");
        }
    }

    #[test]
    fn only_the_mode_may_be_patched() {
        assert!(mode_only(br#"{"mode":"rule"}"#));
        assert!(!mode_only(br#"{"mode":"rule","tun":{"enable":true}}"#));
        assert!(!mode_only(br#"{"allow-lan":true}"#));
        assert!(!mode_only(br#"{"Mode":"rule"}"#), "only the name Kova sends");
        assert!(!mode_only(br#"{"mode":{"x":1}}"#));
        assert!(!mode_only(b"[]"));
        assert!(!mode_only(b""));
    }

    #[test]
    fn a_head_that_could_be_read_two_ways_is_refused() {
        let head = |lines: &[&str]| parse_head(&lines.join("\r\n"));
        let ok = head(&["PUT /proxies/PROXY HTTP/1.1", "Host: 127.0.0.1:9097", "authorization: Bearer abc", "Content-Type: application/json", "Content-Length: 15"]).unwrap();
        assert_eq!((ok.method.as_str(), ok.target.as_str(), ok.content_length, ok.bearer.as_deref()), ("PUT", "/proxies/PROXY", 15, Some("abc")));
        assert_eq!(ok.content_type.as_deref(), Some("application/json"));
        assert_eq!(head(&["GET /version HTTP/1.1"]).unwrap().bearer, None);

        for bad in [
            vec!["GET /version HTTP/1.0"],
            vec!["GET  /version HTTP/1.1"],
            vec!["GET /version HTTP/1.1 x"],
            vec!["get /version HTTP/1.1"],
            vec!["GET http://127.0.0.1/version HTTP/1.1"],
            vec!["GET /ver\tsion HTTP/1.1"],
            vec!["PUT /proxies/x HTTP/1.1", "Transfer-Encoding: chunked"],
            vec!["PUT /proxies/x HTTP/1.1", "Content-Length: 2", "Content-Length: 40"],
            vec!["PUT /proxies/x HTTP/1.1", "Content-Length: +2"],
            vec!["PUT /proxies/x HTTP/1.1", "Content-Length: 99999999"],
            vec!["GET /logs HTTP/1.1", "Upgrade: websocket"],
            vec!["GET /version HTTP/1.1", "Authorization: Bearer a", "Authorization: Bearer b"],
            vec!["GET /version HTTP/1.1", " folded: line"],
            vec!["GET /version HTTP/1.1", "Content-Length : 2"],
            vec!["GET /version HTTP/1.1", "no colon here"],
            vec!["GET /version HTTP/1.1", "X: a\nPUT /configs HTTP/1.1"],
        ] {
            assert_eq!(head(&bad), None, "{bad:?}");
        }
    }

    /// What a client's request gets back through the gate.
    async fn send(port: u16, request: &str) -> String {
        let mut s = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
        s.write_all(request.as_bytes()).await.unwrap();
        let mut out = Vec::new();
        let _ = s.read_to_end(&mut out).await;
        String::from_utf8_lossy(&out).to_string()
    }

    /// A stand-in for Mihomo's controller that keeps every request it is sent, whole.
    async fn mihomo(seen: Arc<Mutex<Vec<String>>>) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let Ok((mut s, _)) = listener.accept().await else { return };
                let Some((head, mut body)) = read_head(&mut s).await else { continue };
                let length = head.to_ascii_lowercase().lines().find_map(|l| l.strip_prefix("content-length:").and_then(|v| v.trim().parse::<usize>().ok())).unwrap_or(0);
                let mut chunk = [0u8; 1024];
                while body.len() < length {
                    match s.read(&mut chunk).await {
                        Ok(n) if n > 0 => body.extend_from_slice(&chunk[..n]),
                        _ => break,
                    }
                }
                seen.lock().unwrap().push(format!("{head}\r\n\r\n{}", String::from_utf8_lossy(&body)));
                let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").await;
            }
        });
        port
    }

    #[tokio::test]
    async fn only_kovas_own_requests_reach_mihomo() {
        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let upstream_port = mihomo(seen.clone()).await;
        let listener = bind(0).await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let keys = Keys { client_secret: "the-secret-kova-knows".into(), upstream_port, upstream_secret: "mihomos-own-secret".into() };
        tokio::spawn(serve(listener, Arc::new(keys)));
        let request = |line: &str, secret: &str, body: &str| {
            let length = if body.is_empty() { String::new() } else { format!("Content-Type: application/json\r\nContent-Length: {}\r\n", body.len()) };
            format!("{line} HTTP/1.1\r\nHost: 127.0.0.1\r\nAuthorization: Bearer {secret}\r\n{length}\r\n{body}")
        };
        let kova = "the-secret-kova-knows";
        let count = || seen.lock().unwrap().len();

        // Kova's request arrives with Mihomo's secret in place of Kova's.
        let reply = send(port, &request("GET /version", kova, "")).await;
        assert!(reply.starts_with("HTTP/1.1 200 OK") && reply.ends_with("ok"), "{reply}");
        let got = seen.lock().unwrap()[0].clone();
        assert!(got.starts_with("GET /version HTTP/1.1\r\n"), "{got}");
        assert!(got.contains("Authorization: Bearer mihomos-own-secret\r\n") && got.contains("Connection: close\r\n"), "{got}");
        assert!(!got.contains(kova), "{got}");

        // The mode goes through with its body.
        let reply = send(port, &request("PATCH /configs", kova, r#"{"mode":"rule"}"#)).await;
        assert!(reply.starts_with("HTTP/1.1 200 OK"), "{reply}");
        assert!(seen.lock().unwrap()[1].ends_with(r#"{"mode":"rule"}"#), "{:?}", seen.lock().unwrap()[1]);
        assert_eq!(count(), 2);

        // Nothing else does, and Mihomo never hears of it.
        for (req, status) in [
            (request("GET /version", "a-guess", ""), "401"),
            ("GET /version HTTP/1.1\r\nHost: x\r\n\r\n".to_string(), "401"),
            (request("PUT /configs?force=true", kova, r#"{"payload":"tun: {enable: true}"}"#), "403"),
            (request("PATCH /configs", kova, r#"{"mode":"rule","allow-lan":true}"#), "403"),
            (request("POST /restart", kova, ""), "403"),
            (request("POST /upgrade", kova, ""), "403"),
            (format!("PUT /proxies/x HTTP/1.1\r\nAuthorization: Bearer {kova}\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n"), "400"),
        ] {
            let reply = send(port, &req).await;
            assert!(reply.starts_with(&format!("HTTP/1.1 {status}")), "{req:?} -> {reply}");
        }
        assert_eq!(count(), 2, "{:?}", seen.lock().unwrap());

        // A second request sent behind an allowed one is thrown away.
        let two = format!("{}{}", request("GET /version", kova, ""), request("PUT /configs", kova, r#"{"payload":"x"}"#));
        let reply = send(port, &two).await;
        assert!(reply.starts_with("HTTP/1.1 200 OK"), "{reply}");
        assert_eq!(count(), 3);
        assert!(!seen.lock().unwrap()[2].contains("/configs"), "{:?}", seen.lock().unwrap()[2]);

        // So is whatever follows a body of the length the request gave.
        let body = r#"{"name":"A"}"#;
        let more = format!("{}PUT /configs HTTP/1.1\r\n\r\n", request("PUT /proxies/PROXY", kova, body));
        send(port, &more).await;
        assert!(seen.lock().unwrap()[3].ends_with(body), "{:?}", seen.lock().unwrap()[3]);
    }
}
