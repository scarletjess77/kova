//! HTTP downloads: subscriptions and release metadata.

use std::time::Duration;

use kova_core::subscription::{UserInfo, decode_title, parse_userinfo};

use crate::Error;

/// Providers choose the response format from the User-Agent. Mentioning `clash.meta`
/// gets the richest one (a full Mihomo config with groups and rules) from most panels.
pub const DEFAULT_SUB_UA: &str = concat!("clash.meta/1.19 mihomo Kova/", env!("CARGO_PKG_VERSION"));
pub const APP_UA: &str = concat!("Kova/", env!("CARGO_PKG_VERSION"));

/// SHA-256 of `data` as lowercase hex, as GitHub publishes release digests.
pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(data))
}

/// Start every HTTP client here. On Android the certificate check reqwest uses by
/// default needs a Java component Kova doesn't ship, so the device's own trusted
/// certificates are handed over instead.
pub fn builder() -> reqwest::ClientBuilder {
    let b = reqwest::Client::builder();
    #[cfg(target_os = "android")]
    let b = b.tls_certs_only(android_roots().iter().cloned());
    b
}

/// How many trusted certificates HTTPS checks use: the device's on Android, `None`
/// where the system checks them itself.
pub fn trusted_certificates() -> Option<usize> {
    #[cfg(target_os = "android")]
    return Some(android_roots().len());
    #[cfg(not(target_os = "android"))]
    None
}

/// A request error with everything it was caused by: reqwest's own message alone
/// ("error sending request") doesn't say what went wrong.
pub fn describe(e: &reqwest::Error) -> String {
    let mut out = e.to_string();
    let mut src = std::error::Error::source(e);
    while let Some(s) = src {
        out.push_str(": ");
        out.push_str(&s.to_string());
        src = s.source();
    }
    out
}

/// The certificates Android trusts: the updatable store (Android 14+), else the system one.
#[cfg(target_os = "android")]
fn android_roots() -> &'static [reqwest::Certificate] {
    static ROOTS: std::sync::OnceLock<Vec<reqwest::Certificate>> = std::sync::OnceLock::new();
    ROOTS.get_or_init(|| {
        for dir in ["/apex/com.android.conscrypt/cacerts", "/system/etc/security/cacerts"] {
            let certs: Vec<reqwest::Certificate> = std::fs::read_dir(dir)
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|e| std::fs::read(e.path()).ok())
                .filter_map(|pem| reqwest::Certificate::from_pem(&pem).ok())
                .filter(|c| reqwest::Client::builder().tls_certs_only([c.clone()]).build().is_ok())
                .collect();
            if !certs.is_empty() {
                return certs;
            }
        }
        tracing::warn!("no trusted certificates found on this device");
        Vec::new()
    })
}

pub fn client(proxy: Option<&str>, user_agent: &str) -> Result<reqwest::Client, Error> {
    let mut b = builder().user_agent(user_agent).connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(60));
    b = match proxy {
        Some(p) => b.proxy(reqwest::Proxy::all(p)?),
        None => b.no_proxy(),
    };
    Ok(b.build()?)
}

#[derive(Debug, Clone)]
pub struct Subscription {
    pub body: String,
    pub usage: Option<UserInfo>,
    pub title: Option<String>,
    pub update_hours: Option<u32>,
    pub support_url: Option<String>,
    /// A message from the provider (the `announce` header), such as planned maintenance.
    pub announce: Option<String>,
}

/// The operating system as panels list it next to a device.
fn device_os() -> &'static str {
    match std::env::consts::OS {
        "windows" => "Windows",
        "android" => "Android",
        "macos" => "macOS",
        "linux" => "Linux",
        other => other,
    }
}

/// `device_id` is a random value made once per install, not a hardware ID. Panels with a
/// device limit (Remnawave and others) refuse a subscription request without it.
pub async fn subscription(url: &str, user_agent: Option<&str>, proxy: Option<&str>, device_id: &str) -> Result<Subscription, Error> {
    let c = client(proxy, user_agent.unwrap_or(DEFAULT_SUB_UA))?;
    let mut req = c.get(url).header("x-device-os", device_os());
    if !device_id.is_empty() {
        req = req.header("x-hwid", device_id);
    }
    let resp = req.send().await?;
    if !resp.status().is_success() {
        return Err(Error::Http(format!("the server answered {}", resp.status())));
    }
    let h = resp.headers().clone();
    let header = |k: &str| h.get(k).and_then(|v| v.to_str().ok()).map(str::to_string);
    let title = header("profile-title").map(|t| decode_title(&t)).or_else(|| {
        header("content-disposition").and_then(|d| d.split("filename=").nth(1).map(|f| f.trim_matches(['"', ';', ' ']).to_string()))
    });
    Ok(Subscription {
        usage: header("subscription-userinfo").map(|u| parse_userinfo(&u)),
        title: title.filter(|t| !t.is_empty()),
        update_hours: header("profile-update-interval").and_then(|v| v.trim().parse().ok()),
        support_url: header("profile-web-page-url").or_else(|| header("support-url")),
        announce: header("announce").map(|a| decode_title(&a).trim().chars().take(300).collect::<String>()).filter(|a| !a.is_empty()),
        body: read_capped(resp, MAX_SUBSCRIPTION).await?,
    })
}

/// Far more than any real subscription (a few thousand servers is a few MB), and little
/// enough that a broken or hostile server can't fill a phone's memory.
const MAX_SUBSCRIPTION: usize = 20 * 1024 * 1024;

/// The body as text, refused once it passes `limit` bytes.
async fn read_capped(mut resp: reqwest::Response, limit: usize) -> Result<String, Error> {
    let too_big = || Error::Http(format!("the subscription is larger than {} MB", limit.div_ceil(1024 * 1024)));
    if resp.content_length().is_some_and(|n| n > limit as u64) {
        return Err(too_big());
    }
    let mut body = Vec::new();
    while let Some(chunk) = resp.chunk().await? {
        if body.len() + chunk.len() > limit {
            return Err(too_big());
        }
        body.extend_from_slice(&chunk);
    }
    let text = String::from_utf8_lossy(&body);
    Ok(text.strip_prefix('\u{feff}').unwrap_or(&text).to_string())
}

#[cfg(test)]
mod tests {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A one-request server: returns what the client sent, and answers with `headers`.
    async fn serve_once(headers: &'static str) -> (String, tokio::task::JoinHandle<String>) {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/sub", l.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut buf = vec![0; 8192];
            let n = s.read(&mut buf).await.unwrap();
            let body = "vless://b831381d-6324-4d53-ad4f-8cda48b30811@203.0.113.1:443?security=tls#A";
            let resp = format!("HTTP/1.1 200 OK\r\n{headers}content-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
            s.write_all(resp.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&buf[..n]).to_lowercase()
        });
        (url, task)
    }

    #[tokio::test]
    async fn sends_the_device_id_and_reads_the_announcement() {
        // "Maintenance tonight" in base64, as panels send it.
        let (url, task) = serve_once("announce: base64:TWFpbnRlbmFuY2UgdG9uaWdodA==\r\n").await;
        let sub = super::subscription(&url, None, None, "abc123").await.unwrap();
        let request = task.await.unwrap();
        assert!(request.contains("x-hwid: abc123"), "{request}");
        assert!(request.contains("x-device-os: "), "{request}");
        assert_eq!(sub.announce.as_deref(), Some("Maintenance tonight"));
    }

    #[tokio::test]
    async fn an_oversized_subscription_is_refused() {
        // Declared too big up front.
        let (url, _task) = serve_once("").await;
        let resp = reqwest::Client::new().get(&url).send().await.unwrap();
        assert!(super::read_capped(resp, 10).await.unwrap_err().to_string().contains("larger than"));
        // Within the limit it reads as text.
        let (url, _task) = serve_once("").await;
        let resp = reqwest::Client::new().get(&url).send().await.unwrap();
        assert!(super::read_capped(resp, 1024).await.unwrap().starts_with("vless://"));

        // No length given (chunked): stopped once it grows past the limit.
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/sub", l.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut buf = vec![0; 8192];
            let _ = s.read(&mut buf).await;
            let _ = s.write_all(b"HTTP/1.1 200 OK\r\ntransfer-encoding: chunked\r\n\r\n").await;
            for _ in 0..64 {
                let chunk = [b"400\r\n".as_slice(), &[b'a'; 0x400], b"\r\n"].concat();
                if s.write_all(&chunk).await.is_err() {
                    return;
                }
            }
            let _ = s.write_all(b"0\r\n\r\n").await;
        });
        let resp = reqwest::Client::new().get(&url).send().await.unwrap();
        assert!(super::read_capped(resp, 16 * 1024).await.is_err(), "64 KB through a 16 KB limit");
    }

    #[tokio::test]
    async fn no_device_id_header_when_there_is_none() {
        let (url, task) = serve_once("").await;
        let sub = super::subscription(&url, None, None, "").await.unwrap();
        assert!(!task.await.unwrap().contains("x-hwid"));
        assert_eq!(sub.announce, None);
    }
}
