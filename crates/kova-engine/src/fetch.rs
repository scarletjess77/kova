//! HTTP downloads: subscriptions, release metadata and the big files (cores).

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
    build_client(proxy, user_agent, reqwest::redirect::Policy::default(), None)
}

fn build_client(proxy: Option<&str>, user_agent: &str, redirect: reqwest::redirect::Policy, resolver: Option<std::sync::Arc<dyn reqwest::dns::Resolve>>) -> Result<reqwest::Client, Error> {
    let mut b = builder().user_agent(user_agent).connect_timeout(Duration::from_secs(10)).timeout(Duration::from_secs(60)).redirect(redirect);
    if let Some(r) = resolver {
        b = b.dns_resolver(r);
    }
    b = match proxy {
        Some(p) => b.proxy(reqwest::Proxy::all(p)?),
        None => b.no_proxy(),
    };
    Ok(b.build()?)
}

/// Time limits for a download of a big file (a core is up to ~60 MB).
///
/// The normal client's 60 s limit covers the whole request including the body, which
/// cuts off a download that is slow but still moving. Here only silence is limited: no
/// connection within `connect`, or no byte for `idle`. `ceiling` is a last resort for a
/// server that trickles bytes forever or a link throttled to a crawl that still moves
/// (which never trips the idle limit). The default is 10 minutes (about 100 KB/s for
/// 60 MB); a direct attempt with Kova's proxy as the next route gets 4 minutes, so the
/// fallback starts sooner; callers that someone waits on (connecting, the service's
/// install) use [`BigLimits::QUICK`], 5 minutes.
#[derive(Debug, Clone, Copy)]
pub struct BigLimits {
    pub connect: Duration,
    pub idle: Duration,
    pub ceiling: Duration,
}

impl BigLimits {
    pub const DEFAULT: BigLimits = BigLimits { connect: Duration::from_secs(10), idle: Duration::from_secs(30), ceiling: Duration::from_secs(10 * 60) };
    /// For installs a person or the app is waiting on: the app waits 600 s for the
    /// service, and connecting must not hang for longer than a few minutes.
    pub const QUICK: BigLimits = BigLimits { connect: Duration::from_secs(10), idle: Duration::from_secs(30), ceiling: Duration::from_secs(5 * 60) };
    /// The ceiling of a direct attempt that has Kova's proxy to fall back on.
    pub const DIRECT_BEFORE_PROXY: Duration = Duration::from_secs(4 * 60);

    /// The limits of one attempt. A direct attempt (`direct`) that is followed by a try
    /// through Kova's proxy (`has_fallback`) gets the shorter ceiling, never a longer one
    /// than `self` already has; every other attempt keeps `self`.
    pub fn for_attempt(self, direct: bool, has_fallback: bool) -> BigLimits {
        if direct && has_fallback { BigLimits { ceiling: self.ceiling.min(Self::DIRECT_BEFORE_PROXY), ..self } } else { self }
    }
}

/// A client for big files, see [`BigLimits`].
pub fn big_client(proxy: Option<&str>, limits: BigLimits) -> Result<reqwest::Client, Error> {
    let mut b = builder().user_agent(APP_UA).connect_timeout(limits.connect).read_timeout(limits.idle).timeout(limits.ceiling);
    b = match proxy {
        Some(p) => b.proxy(reqwest::Proxy::all(p)?),
        None => b.no_proxy(),
    };
    Ok(b.build()?)
}

/// Downloads `url` whole, through `proxy` or directly. An error status is an error.
pub async fn download_big(url: &str, proxy: Option<&str>, limits: BigLimits) -> Result<Vec<u8>, Error> {
    let c = big_client(proxy, limits)?;
    // `Vec::from` reuses the buffer when it is the only owner, instead of copying ~60 MB.
    Ok(Vec::from(c.get(url).send().await?.error_for_status()?.bytes().await?))
}

/// Whether `e` says the server could not be reached or went silent (timeout, connect,
/// DNS, reset) rather than answering with a refusal: only then is another route worth a try.
pub fn is_connection_error(e: &Error) -> bool {
    match e {
        Error::Net(r) => r.is_timeout() || r.is_connect() || r.is_request() || r.is_body(),
        _ => false,
    }
}

/// Runs `attempt(None)` (direct); when it fails to reach the server and Kova's own proxy
/// is up (`proxy`), runs `attempt(Some(proxy))` once. Never more than two tries. The error
/// of a failed second try names both, so the user can tell a blocked site from a broken
/// connection. Without a proxy the first error is returned as it is.
pub async fn with_proxy_fallback<T, F, Fut>(proxy: Option<&str>, what: &str, mut attempt: F) -> Result<T, Error>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: std::future::Future<Output = Result<T, Error>>,
{
    let first = attempt(None).await;
    match (first, proxy) {
        (Err(e), Some(p)) if is_connection_error(&e) => match attempt(Some(p.to_string())).await {
            Ok(v) => {
                tracing::info!("{what} was downloaded through Kova's proxy, the direct download failed: {e}");
                Ok(v)
            }
            Err(e2) => Err(Error::Core(format!("{what} could not be downloaded, neither directly ({e}) nor through Kova's own proxy ({e2})"))),
        },
        (r, _) => r,
    }
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
pub async fn subscription(url: &str, user_agent: Option<&str>, headers: &[crate::store::Header], proxy: Option<&str>, device_id: &str) -> Result<Subscription, Error> {
    subscription_guarded(url, user_agent, headers, proxy, device_id, &Guard::OPEN).await
}

/// What a download may reach (see [`subscription_guarded`]).
pub struct Guard<'a> {
    /// Asked about every address: the first and each redirect.
    pub allow: &'a (dyn Fn(&str) -> bool + Sync),
    /// Set: a name that resolves only to this computer (loopback, link-local, unspecified)
    /// is refused, except this one host (the subscription's own).
    pub local_dns_except: Option<&'a str>,
}

impl Guard<'_> {
    /// No limits.
    pub const OPEN: Guard<'static> = Guard { allow: &|_| true, local_dns_except: None };
}

/// Whether an address is this computer's own or unusable as a destination.
pub fn is_local_address(ip: std::net::IpAddr) -> bool {
    match ip.to_canonical() {
        std::net::IpAddr::V4(v4) => v4.is_loopback() || v4.is_link_local() || v4.is_unspecified(),
        std::net::IpAddr::V6(v6) => v6.is_loopback() || v6.is_unspecified() || (v6.segments()[0] & 0xffc0) == 0xfe80,
    }
}

/// The system's name lookup without the answers that point to this computer, so a name
/// that resolves to one (127.0.0.1.nip.io) can't aim a download at it. `tripped` is set
/// when a name had only such answers.
struct PublicOnly {
    except: String,
    tripped: std::sync::Arc<std::sync::atomic::AtomicBool>,
}

impl reqwest::dns::Resolve for PublicOnly {
    fn resolve(&self, name: reqwest::dns::Name) -> reqwest::dns::Resolving {
        let (except, tripped) = (self.except.clone(), self.tripped.clone());
        Box::pin(async move {
            let host = name.as_str().to_string();
            let found: Vec<std::net::SocketAddr> = tokio::net::lookup_host((host.as_str(), 0)).await?.collect();
            let kept: Vec<std::net::SocketAddr> = if host.eq_ignore_ascii_case(&except) { found } else { found.into_iter().filter(|a| !is_local_address(a.ip())).collect() };
            if kept.is_empty() {
                tripped.store(true, std::sync::atomic::Ordering::Relaxed);
            }
            Ok(Box::new(kept.into_iter()) as reqwest::dns::Addrs)
        })
    }
}

/// Like [`subscription`], but held to a [`Guard`]: a refused address, or a name that
/// resolves only to this computer, ends the download with `Error::Refused`. Redirects are
/// followed here (up to 10) rather than by reqwest. The device id (the install's) and the
/// profile's own headers (which may hold a token) go only where the download began, the
/// same scheme, host and port: a redirect elsewhere gets neither.
pub async fn subscription_guarded(url: &str, user_agent: Option<&str>, headers: &[crate::store::Header], proxy: Option<&str>, device_id: &str, guard: &Guard<'_>) -> Result<Subscription, Error> {
    let tripped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let resolver = guard.local_dns_except.map(|e| std::sync::Arc::new(PublicOnly { except: e.to_string(), tripped: tripped.clone() }) as std::sync::Arc<dyn reqwest::dns::Resolve>);
    let c = build_client(proxy, user_agent.unwrap_or(DEFAULT_SUB_UA), reqwest::redirect::Policy::none(), resolver)?;
    let first = reqwest::Url::parse(url).map_err(|e| Error::Other(format!("the address is not valid: {e}")))?;
    let mut current = first.clone();
    let mut hops = 0;
    let resp = loop {
        if !matches!(current.scheme(), "http" | "https") || !(guard.allow)(current.as_str()) {
            return Err(Error::Refused(format!("the address {} is not allowed", current.host_str().unwrap_or_default())));
        }
        let same_origin = current.scheme() == first.scheme() && current.host_str() == first.host_str() && current.port_or_known_default() == first.port_or_known_default();
        let mut req = c.get(current.clone()).header("x-device-os", device_os());
        if same_origin {
            if !device_id.is_empty() {
                req = req.header("x-hwid", device_id);
            }
            // The profile's own headers last, so they can replace Kova's.
            for h in headers.iter().filter(|h| h.problem().is_none()) {
                req = req.header(h.name.trim(), h.value.trim());
            }
        }
        let resp = match req.send().await {
            Ok(r) => r,
            Err(_) if tripped.load(std::sync::atomic::Ordering::Relaxed) => return Err(Error::Refused(format!("{} points to this computer", current.host_str().unwrap_or_default()))),
            Err(e) => return Err(e.into()),
        };
        let next = resp.status().is_redirection().then(|| resp.headers().get(reqwest::header::LOCATION).and_then(|v| v.to_str().ok()).and_then(|l| current.join(l).ok())).flatten();
        let Some(next) = next else { break resp };
        hops += 1;
        if hops > 10 {
            return Err(Error::Http("too many redirects".into()));
        }
        current = next;
    };
    if !resp.status().is_success() {
        return Err(Error::Http(format!("the server answered {}", resp.status())));
    }
    let h = resp.headers().clone();
    let body = read_capped(resp, MAX_SUBSCRIPTION).await?;
    Ok(assemble(&h, body))
}

/// The subscription's details: from the response headers, and where a header is missing,
/// from the `#key: value` comment lines some providers put at the top of the body. A header
/// that is there but empty counts as missing; one that is there but unusable (a support
/// address that isn't a web address, an interval that isn't a number) still wins, so the
/// body never overrides what the server said in its headers.
fn assemble(h: &reqwest::header::HeaderMap, body: String) -> Subscription {
    let inline = leading_comments(&body);
    let header = |k: &str| h.get(k).and_then(|v| v.to_str().ok()).filter(|v| !v.trim().is_empty()).map(str::to_string);
    let inline = |k: &str| inline.get(k).cloned();
    let title = header("profile-title").or_else(|| inline("profile-title")).map(|t| decode_title(&t).chars().take(300).collect::<String>()).or_else(|| {
        header("content-disposition").and_then(|d| d.split("filename=").nth(1).map(|f| f.trim_matches(['"', ';', ' ']).to_string()))
    });
    Subscription {
        usage: header("subscription-userinfo").or_else(|| inline("subscription-userinfo")).map(|u| parse_userinfo(&u)),
        title: title.filter(|t| !t.is_empty()),
        update_hours: header("profile-update-interval").or_else(|| inline("profile-update-interval")).and_then(|v| v.trim().parse().ok()),
        // A web address only: the Provider button opens it.
        support_url: header("profile-web-page-url").or_else(|| header("support-url")).or_else(|| inline("profile-web-page-url")).or_else(|| inline("support-url")).map(|u| u.trim().to_string()).filter(|u| (u.starts_with("https://") || u.starts_with("http://")) && !u.contains(char::is_whitespace)),
        announce: header("announce").or_else(|| inline("announce")).map(|a| decode_title(&a).trim().chars().take(300).collect::<String>()).filter(|a| !a.is_empty()),
        body,
    }
}

/// The details a provider may put in the first comment lines of the body, by lowercase name.
/// Only the leading block counts: up to 32 lines and 8 KB, ending at the first line that is
/// neither a comment nor empty. The first non-empty value of a name wins.
fn leading_comments(body: &str) -> std::collections::HashMap<&'static str, String> {
    const KEYS: [&str; 6] = ["profile-title", "profile-update-interval", "subscription-userinfo", "profile-web-page-url", "support-url", "announce"];
    let mut out = std::collections::HashMap::new();
    let mut seen = 0;
    for line in body.lines().take(32) {
        seen += line.len();
        let line = line.trim();
        if seen > 8 * 1024 || (!line.is_empty() && !line.starts_with('#')) {
            break;
        }
        let Some(rest) = line.strip_prefix('#') else { continue };
        let Some(at) = rest.find([':', '=']) else { continue };
        let key = rest[..at].trim().to_ascii_lowercase();
        let value = rest[at + 1..].trim();
        if let Some(k) = KEYS.iter().find(|k| **k == key)
            && !value.is_empty()
        {
            out.entry(*k).or_insert_with(|| value.to_string());
        }
    }
    out
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
    // Some servers send the subscription as a .gz file rather than compressing the
    // response, so it arrives still packed. Unpack it, under the same limit.
    if body.starts_with(&[0x1f, 0x8b]) {
        use std::io::Read;
        let mut out = Vec::new();
        flate2::read::GzDecoder::new(&body[..]).take(limit as u64 + 1).read_to_end(&mut out).map_err(|_| Error::Http("the subscription is a damaged .gz file".into()))?;
        if out.len() > limit {
            return Err(too_big());
        }
        body = out;
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
        let sub = super::subscription(&url, None, &[crate::store::Header { name: "X-Provider-Token".into(), value: "t0k".into() }], None, "abc123").await.unwrap();
        let request = task.await.unwrap();
        assert!(request.contains("x-hwid: abc123"), "{request}");
        assert!(request.contains("x-provider-token: t0k"), "the profile's own header: {request}");
        assert!(request.contains("x-device-os: "), "{request}");
        assert_eq!(sub.announce.as_deref(), Some("Maintenance tonight"));
    }

    /// A redirect to another host does not carry the device id, and `allow` sees every hop.
    #[tokio::test]
    async fn a_redirect_to_another_host_gets_no_device_id_and_is_checked() {
        let (target, task) = serve_once("").await;
        let port = target.rsplit(':').next().unwrap().split('/').next().unwrap().to_string();
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let first = format!("http://{}/sub", l.local_addr().unwrap());
        let location = format!("http://localhost:{port}/sub");
        let hop = tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut buf = vec![0; 8192];
            let n = s.read(&mut buf).await.unwrap();
            let resp = format!("HTTP/1.1 302 Found
location: {location}
content-length: 0
connection: close

");
            s.write_all(resp.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&buf[..n]).to_lowercase()
        });
        super::subscription_guarded(&first, None, &[], None, "abc123", &super::Guard::OPEN).await.unwrap();
        assert!(hop.await.unwrap().contains("x-hwid: abc123"), "the first host gets it");
        assert!(!task.await.unwrap().contains("x-hwid"), "the other host does not");

        // A refused hop ends the download before anything is sent there.
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let first = format!("http://{}/sub", l.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut buf = vec![0; 8192];
            let _ = s.read(&mut buf).await;
            let _ = s.write_all(b"HTTP/1.1 302 Found
location: http://localhost:1/x
content-length: 0
connection: close

").await;
        });
        let guard = super::Guard { allow: &|u| !u.contains("localhost"), local_dns_except: None };
        let err = super::subscription_guarded(&first, None, &[], None, "", &guard).await.unwrap_err();
        assert!(matches!(err, crate::Error::Refused(_)), "{err}");
    }

    /// The profile's own headers and the device id stay with the origin the download
    /// began at: another host or another port gets neither.
    #[tokio::test]
    async fn a_redirect_to_another_origin_keeps_the_profiles_headers_home() {
        let header = [crate::store::Header { name: "X-Provider-Token".into(), value: "t0k".into() }];
        for same_host in [false, true] {
            let (target, task) = serve_once("").await;
            let port = target.rsplit(':').next().unwrap().split('/').next().unwrap().to_string();
            let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let first = format!("http://{}/sub", l.local_addr().unwrap());
            // Same host, other port; or another host name.
            let location = if same_host { format!("http://127.0.0.1:{port}/sub") } else { format!("http://localhost:{port}/sub") };
            let hop = tokio::spawn(async move {
                let (mut s, _) = l.accept().await.unwrap();
                let mut buf = vec![0; 8192];
                let n = s.read(&mut buf).await.unwrap();
                let resp = format!("HTTP/1.1 302 Found\r\nlocation: {location}\r\ncontent-length: 0\r\nconnection: close\r\n\r\n");
                s.write_all(resp.as_bytes()).await.unwrap();
                String::from_utf8_lossy(&buf[..n]).to_lowercase()
            });
            super::subscription_guarded(&first, None, &header, None, "abc123", &super::Guard::OPEN).await.unwrap();
            let first_request = hop.await.unwrap();
            assert!(first_request.contains("x-provider-token: t0k") && first_request.contains("x-hwid: abc123"), "{first_request}");
            let second = task.await.unwrap();
            assert!(!second.contains("x-provider-token") && !second.contains("x-hwid"), "{second}");
        }
    }

    /// A name that resolves only to this computer is refused, except the subscription's own.
    #[tokio::test]
    async fn a_name_that_resolves_to_this_computer_is_refused() {
        let (url, _task) = serve_once("").await;
        let port = url.rsplit(':').next().unwrap().split('/').next().unwrap().to_string();
        let by_name = format!("http://localhost:{port}/sub");
        let guard = super::Guard { allow: &|_| true, local_dns_except: Some("") };
        let err = super::subscription_guarded(&by_name, None, &[], None, "", &guard).await.unwrap_err();
        assert!(matches!(err, crate::Error::Refused(_)), "{err}");
        let (url, _task) = serve_once("").await;
        let port = url.rsplit(':').next().unwrap().split('/').next().unwrap().to_string();
        let guard = super::Guard { allow: &|_| true, local_dns_except: Some("localhost") };
        super::subscription_guarded(&format!("http://localhost:{port}/sub"), None, &[], None, "", &guard).await.unwrap();
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
    async fn a_gzipped_subscription_file_is_unpacked() {
        use std::io::Write;
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        gz.write_all(b"vless://b831381d-6324-4d53-ad4f-8cda48b30811@203.0.113.1:443?security=tls#A").unwrap();
        let body = gz.finish().unwrap();
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/sub.gz", l.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut buf = vec![0; 8192];
            let _ = s.read(&mut buf).await;
            let head = format!("HTTP/1.1 200 OK\r\ncontent-type: application/gzip\r\ncontent-length: {}\r\nconnection: close\r\n\r\n", body.len());
            let _ = s.write_all(head.as_bytes()).await;
            let _ = s.write_all(&body).await;
        });
        let sub = super::subscription(&url, None, &[], None, "").await.unwrap();
        assert!(sub.body.starts_with("vless://"), "{}", sub.body);
    }

    fn headers(pairs: &[(&'static str, &str)]) -> reqwest::header::HeaderMap {
        let mut h = reqwest::header::HeaderMap::new();
        for (k, v) in pairs {
            h.insert(*k, v.parse().unwrap());
        }
        h
    }

    const LINK: &str = "vless://b831381d-6324-4d53-ad4f-8cda48b30811@203.0.113.1:443?security=tls#A";

    #[test]
    fn details_in_the_body_comments_are_used_where_no_header_gives_them() {
        let body = format!("#profile-title: My VPN\n# Profile-Update-Interval=12\n#subscription-userinfo: upload=1; download=2; total=30; expire=4\n#profile-web-page-url: https://example.com/me\n#announce: Hello\n{LINK}\n");
        let s = super::assemble(&headers(&[]), body.clone());
        assert_eq!(s.title.as_deref(), Some("My VPN"));
        assert_eq!(s.update_hours, Some(12));
        let u = s.usage.unwrap();
        assert_eq!((u.upload, u.download, u.total, u.expire), (1, 2, 30, Some(4)));
        assert_eq!(s.support_url.as_deref(), Some("https://example.com/me"));
        assert_eq!(s.announce.as_deref(), Some("Hello"));
        assert_eq!(s.body, body, "the body is passed on untouched");
    }

    #[test]
    fn a_header_wins_over_the_body_comments() {
        let body = format!("#profile-title: From body\n#profile-update-interval: 12\n#announce: body\n{LINK}");
        let s = super::assemble(&headers(&[("profile-title", "From header"), ("profile-update-interval", "6")]), body);
        assert_eq!(s.title.as_deref(), Some("From header"));
        assert_eq!(s.update_hours, Some(6));
        assert_eq!(s.announce.as_deref(), Some("body"), "only the missing ones come from the body");
    }

    #[test]
    fn body_comment_values_may_be_base64_and_junk_is_ignored() {
        // "Мой VPN" and "Maintenance tonight" in base64.
        let body = format!("#profile-title: base64:0JzQvtC5IFZQTg==\n#announce: base64:TWFpbnRlbmFuY2UgdG9uaWdodA==\n# just a remark\n#\n#profile-web-page-url: javascript:alert(1)\n#profile-update-interval: soon\n\n{LINK}");
        let s = super::assemble(&headers(&[]), body);
        assert_eq!(s.title.as_deref(), Some("Мой VPN"));
        assert_eq!(s.announce.as_deref(), Some("Maintenance tonight"));
        assert_eq!(s.support_url, None);
        assert_eq!(s.update_hours, None);
        assert!(s.usage.is_none());
    }

    #[test]
    fn comments_after_the_first_line_of_content_are_ignored() {
        let body = format!("#profile-title: First\n{LINK}\n#profile-title: Later\n#announce: Later\n");
        let s = super::assemble(&headers(&[]), body);
        assert_eq!(s.title.as_deref(), Some("First"));
        assert_eq!(s.announce, None);
        // Nothing leading at all.
        let s = super::assemble(&headers(&[]), format!("{LINK}\n#profile-title: Late\n"));
        assert_eq!(s.title, None);
        // A long run of comments stops at the line limit.
        let long = format!("{}#announce: too far\n{LINK}", "# x\n".repeat(32));
        assert_eq!(super::assemble(&headers(&[]), long).announce, None);
    }

    #[test]
    fn empty_values_are_skipped_and_the_title_has_a_limit() {
        // An empty first line doesn't block a later one, nor the file name fallback.
        let body = format!("#profile-title:
#profile-title:   
#profile-title: Later
#support-url: https://example.com/help
{LINK}");
        let s = super::assemble(&headers(&[]), body);
        assert_eq!(s.title.as_deref(), Some("Later"));
        assert_eq!(s.support_url.as_deref(), Some("https://example.com/help"));
        let s = super::assemble(&headers(&[("content-disposition", "attachment; filename=\"From file\"")]), format!("#profile-title:
{LINK}"));
        assert_eq!(s.title.as_deref(), Some("From file"));
        // The title in the body wins over the file name, the header title over both.
        let s = super::assemble(&headers(&[("content-disposition", "attachment; filename=\"From file\"")]), format!("#profile-title: Inline
{LINK}"));
        assert_eq!(s.title.as_deref(), Some("Inline"));
        let s = super::assemble(&headers(&[("profile-title", "Header"), ("content-disposition", "attachment; filename=\"From file\"")]), format!("#profile-title: Inline
{LINK}"));
        assert_eq!(s.title.as_deref(), Some("Header"));
        // An empty header counts as missing.
        assert_eq!(super::assemble(&headers(&[("profile-title", " ")]), format!("#profile-title: Inline
{LINK}")).title.as_deref(), Some("Inline"));
        // Long titles are cut like announcements.
        let s = super::assemble(&headers(&[]), format!("#profile-title: {}
{LINK}", "t".repeat(1000)));
        assert_eq!(s.title.unwrap().chars().count(), 300);
    }

    #[test]
    fn a_header_that_is_unusable_still_wins_over_the_body() {
        let body = format!("#profile-web-page-url: https://example.com/me
#profile-update-interval: 12
{LINK}");
        let s = super::assemble(&headers(&[("profile-web-page-url", "ftp://nope"), ("profile-update-interval", "often")]), body);
        assert_eq!(s.support_url, None);
        assert_eq!(s.update_hours, None);
    }

    #[test]
    fn download_limits_are_chosen_per_attempt() {
        use super::BigLimits as L;
        use std::time::Duration;
        let min = |n: u64| Duration::from_secs(n * 60);
        assert_eq!(L::DEFAULT.ceiling, min(10));
        assert_eq!(L::QUICK.ceiling, min(5));
        // A direct try with Kova's proxy behind it gives up after 4 minutes.
        assert_eq!(L::DEFAULT.for_attempt(true, true).ceiling, min(4));
        assert_eq!(L::QUICK.for_attempt(true, true).ceiling, min(4));
        // No proxy to fall back on, or the try through the proxy itself: the full ceiling.
        assert_eq!(L::DEFAULT.for_attempt(true, false).ceiling, min(10));
        assert_eq!(L::DEFAULT.for_attempt(false, true).ceiling, min(10));
        assert_eq!(L::QUICK.for_attempt(false, false).ceiling, min(5));
        // A caller's own shorter ceiling is never raised, and the rest is kept.
        let short = L { ceiling: Duration::from_secs(20), ..L::DEFAULT };
        assert_eq!(short.for_attempt(true, true).ceiling, Duration::from_secs(20));
        assert_eq!(short.for_attempt(true, true).idle, L::DEFAULT.idle);
    }

    #[test]
    fn body_comments_may_end_lines_with_crlf_and_are_limited_in_size() {
        let s = super::assemble(&headers(&[]), format!("#profile-title: Windows
#announce: Hi

{LINK}
"));
        assert_eq!((s.title.as_deref(), s.announce.as_deref()), (Some("Windows"), Some("Hi")));
        // Three 3000-byte remarks pass 8 KB: what follows is not read.
        let filler = format!("# {}
", "x".repeat(3000));
        let s = super::assemble(&headers(&[]), format!("{}#announce: too late
{LINK}", filler.repeat(3)));
        assert_eq!(s.announce, None);
        let s = super::assemble(&headers(&[]), format!("{}#announce: in time
{LINK}", filler.repeat(2)));
        assert_eq!(s.announce.as_deref(), Some("in time"));
    }

    fn limits(idle_ms: u64) -> super::BigLimits {
        super::BigLimits { connect: std::time::Duration::from_secs(2), idle: std::time::Duration::from_millis(idle_ms), ceiling: std::time::Duration::from_secs(20) }
    }

    /// Answers one request with `n` bytes, one every `gap_ms`, then closes.
    async fn serve_drip(n: usize, gap_ms: u64) -> String {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/core.zip", l.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let _ = s.read(&mut [0u8; 4096]).await.unwrap();
            s.write_all(format!("HTTP/1.1 200 OK\r\ncontent-length: {n}\r\nconnection: close\r\n\r\n").as_bytes()).await.unwrap();
            for _ in 0..n {
                s.write_all(b"x").await.unwrap();
                s.flush().await.unwrap();
                tokio::time::sleep(std::time::Duration::from_millis(gap_ms)).await;
            }
        });
        url
    }

    /// A body that takes longer in total than the idle limit, but is never silent for it.
    #[tokio::test]
    async fn a_slow_but_steady_big_download_is_not_cut() {
        let url = serve_drip(12, 100).await;
        let started = std::time::Instant::now();
        let data = super::download_big(&url, None, limits(400)).await.unwrap();
        assert_eq!(data.len(), 12);
        assert!(started.elapsed() > std::time::Duration::from_millis(800), "took {:?}", started.elapsed());
    }

    #[tokio::test]
    async fn a_stalled_big_download_is_cut_by_the_idle_limit() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/core.zip", l.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let _ = s.read(&mut [0u8; 4096]).await.unwrap();
            s.write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 1000\r\n\r\nabc").await.unwrap();
            tokio::time::sleep(std::time::Duration::from_secs(30)).await;
        });
        let started = std::time::Instant::now();
        let e = super::download_big(&url, None, limits(400)).await.expect_err("a stalled body must fail");
        assert!(super::is_connection_error(&e), "{e:?}");
        assert!(started.elapsed() < std::time::Duration::from_secs(5));
    }

    /// A tiny HTTP proxy: answers every request with `body`, whatever address was asked for.
    async fn serve_proxy(body: &'static str) -> (String, tokio::task::JoinHandle<String>) {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = format!("http://{}", l.local_addr().unwrap());
        let task = tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut buf = vec![0; 8192];
            let n = s.read(&mut buf).await.unwrap();
            let resp = format!("HTTP/1.1 200 OK\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}", body.len());
            s.write_all(resp.as_bytes()).await.unwrap();
            String::from_utf8_lossy(&buf[..n]).to_string()
        });
        (addr, task)
    }

    /// An address nothing listens on.
    async fn dead_url() -> String {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/core.zip", l.local_addr().unwrap());
        drop(l);
        url
    }

    #[tokio::test]
    async fn a_failed_direct_download_is_retried_once_through_kovas_proxy() {
        let url = dead_url().await;
        let (proxy, task) = serve_proxy("core-bytes").await;
        let tries = std::sync::atomic::AtomicUsize::new(0);
        let data = super::with_proxy_fallback(Some(&proxy), "the core", |p| {
            tries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let url = url.clone();
            async move { super::download_big(&url, p.as_deref(), limits(1000)).await }
        })
        .await
        .unwrap();
        assert_eq!(data, b"core-bytes");
        assert_eq!(tries.load(std::sync::atomic::Ordering::SeqCst), 2);
        assert!(task.await.unwrap().starts_with("GET http://"), "the request went to the proxy");
    }

    #[tokio::test]
    async fn without_a_proxy_the_direct_error_is_returned_and_nothing_is_retried() {
        let url = dead_url().await;
        let tries = std::sync::atomic::AtomicUsize::new(0);
        let e = super::with_proxy_fallback(None, "the core", |p| {
            tries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let url = url.clone();
            async move { super::download_big(&url, p.as_deref(), limits(1000)).await }
        })
        .await
        .expect_err("nothing listens");
        assert!(matches!(e, crate::Error::Net(_)), "{e:?}");
        assert_eq!(tries.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn when_the_proxy_fails_too_the_error_names_both_and_there_is_no_third_try() {
        let url = dead_url().await;
        let proxy = dead_url().await.replace("/core.zip", "");
        let tries = std::sync::atomic::AtomicUsize::new(0);
        let e = super::with_proxy_fallback(Some(&proxy), "the core", |p| {
            tries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let url = url.clone();
            async move { super::download_big(&url, p.as_deref(), limits(1000)).await }
        })
        .await
        .expect_err("both fail");
        let text = e.to_string();
        assert!(text.contains("neither directly") && text.contains("Kova's own proxy"), "{text}");
        assert_eq!(tries.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    /// A server that answers with a refusal is not a blocked route: no retry.
    #[tokio::test]
    async fn an_error_status_does_not_trigger_the_proxy() {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/core.zip", l.local_addr().unwrap());
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let _ = s.read(&mut [0u8; 4096]).await.unwrap();
            s.write_all(b"HTTP/1.1 404 Not Found\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await.unwrap();
        });
        let tries = std::sync::atomic::AtomicUsize::new(0);
        let e = super::with_proxy_fallback(Some("http://127.0.0.1:9"), "the core", |p| {
            tries.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let url = url.clone();
            async move { super::download_big(&url, p.as_deref(), limits(1000)).await }
        })
        .await
        .expect_err("404");
        assert!(!super::is_connection_error(&e), "{e:?}");
        assert_eq!(tries.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn no_device_id_header_when_there_is_none() {
        let (url, task) = serve_once("").await;
        let sub = super::subscription(&url, None, &[], None, "").await.unwrap();
        assert!(!task.await.unwrap().contains("x-hwid"));
        assert_eq!(sub.announce, None);
    }
}
