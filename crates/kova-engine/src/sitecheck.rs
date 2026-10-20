//! "Can I reach this site?" checks, through Kova and without it, with failures
//! described the way a user experiences them (timeout, reset, broken TLS, ...).

use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

/// A site in the check list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Site {
    pub name: String,
    pub url: String,
    pub group: String,
}

fn site(group: &str, name: &str, url: &str) -> Site {
    Site { name: name.into(), url: url.into(), group: group.into() }
}

/// Sites that are commonly blocked or region-locked, plus two baselines that tell
/// "this site is blocked" apart from "there is no internet".
pub fn default_sites() -> Vec<Site> {
    vec![
        site("Baseline", "Google (connectivity check)", "https://www.gstatic.com/generate_204"),
        site("Baseline", "Yandex", "https://ya.ru/"),
        site("Video", "YouTube", "https://www.youtube.com/"),
        site("Video", "YouTube video servers", "https://redirector.googlevideo.com/report_mapping"),
        site("Video", "Netflix", "https://www.netflix.com/"),
        site("Video", "Twitch", "https://www.twitch.tv/"),
        site("Social", "Instagram", "https://www.instagram.com/"),
        site("Social", "Facebook", "https://www.facebook.com/"),
        site("Social", "X (Twitter)", "https://x.com/"),
        site("Social", "LinkedIn", "https://www.linkedin.com/"),
        site("Messengers", "Telegram Web", "https://web.telegram.org/"),
        site("Messengers", "WhatsApp Web", "https://web.whatsapp.com/"),
        site("Messengers", "Discord", "https://discord.com/"),
        site("Messengers", "Signal", "https://signal.org/"),
        site("AI", "ChatGPT", "https://chatgpt.com/"),
        site("AI", "Claude", "https://claude.ai/"),
        site("AI", "Gemini", "https://gemini.google.com/"),
        site("Other", "Tor Project", "https://www.torproject.org/"),
        site("Other", "Proton", "https://proton.me/"),
        site("Other", "Rutracker", "https://rutracker.org/"),
        site("Other", "Medium", "https://medium.com/"),
    ]
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Outcome {
    /// The site answered normally.
    Ok,
    /// The site answered but refused us (403/451): usually a region lock.
    Refused,
    /// The site answered with another error status; it is reachable.
    HttpError,
    Timeout,
    /// Connection reset or dropped mid-handshake: typical of blocking equipment.
    Reset,
    /// The TLS handshake failed or was tampered with.
    Tls,
    Dns,
    ConnectFailed,
}

#[derive(Debug, Clone, Serialize)]
pub struct Probe {
    pub outcome: Outcome,
    pub status: Option<u16>,
    /// Time until the response headers arrived.
    pub ms: Option<u32>,
    /// Short human description of a failure.
    pub detail: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SiteResult {
    pub url: String,
    /// `None` when Kova is not connected.
    pub via_kova: Option<Probe>,
    pub direct: Probe,
}

const BROWSER_UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0 Safari/537.36";

fn classify(e: &reqwest::Error) -> (Outcome, String) {
    if e.is_timeout() {
        return (Outcome::Timeout, "No answer within 10 seconds".into());
    }
    // The useful detail is deep in the source chain; flatten it for matching.
    let mut chain = e.to_string();
    let mut src = std::error::Error::source(e);
    while let Some(s) = src {
        chain.push_str(": ");
        chain.push_str(&s.to_string());
        src = s.source();
    }
    let low = chain.to_ascii_lowercase();
    if low.contains("dns") || low.contains("no such host") || low.contains("failed to lookup") || low.contains("name or service not known") {
        (Outcome::Dns, "The site's name could not be resolved".into())
    } else if low.contains("reset") || low.contains("10054") || low.contains("forcibly closed") || low.contains("connection aborted") || low.contains("broken pipe") {
        (Outcome::Reset, "The connection was reset, which is typical of blocking".into())
    } else if low.contains("tls") || low.contains("handshake") || low.contains("certificate") || low.contains("unexpected eof") || low.contains("close_notify") {
        (Outcome::Tls, "The secure connection was broken, which is typical of DPI blocking".into())
    } else if low.contains("refused") || low.contains("10061") {
        (Outcome::ConnectFailed, "The server refused the connection".into())
    } else if e.is_connect() {
        (Outcome::ConnectFailed, "Could not connect".into())
    } else if low.contains("protocol error") || low.contains("connection closed before message completed") || low.contains("incomplete message") {
        (Outcome::Reset, "The connection was broken mid-request, which is typical of blocking".into())
    } else {
        // Keep the innermost, most specific part of the chain.
        let inner = chain.rsplit(": ").find(|p| !p.trim().is_empty()).unwrap_or(&chain);
        (Outcome::ConnectFailed, inner.chars().take(160).collect())
    }
}

/// Fetch `url` once (headers only) directly or through `proxy`.
pub async fn probe(url: &str, proxy: Option<&str>) -> Probe {
    let mut b = crate::fetch::builder()
        .user_agent(BROWSER_UA)
        .connect_timeout(Duration::from_secs(8))
        .timeout(Duration::from_secs(10))
        // HTTP/1.1 is enough to tell reachable from blocked, and avoids HTTP/2 quirks
        // (some Google front ends reset HTTP/2 streams from non-browser clients).
        .http1_only()
        .redirect(reqwest::redirect::Policy::limited(5));
    b = match proxy.map(reqwest::Proxy::all) {
        Some(Ok(p)) => b.proxy(p),
        _ => b.no_proxy(),
    };
    let client = match b.build() {
        Ok(c) => c,
        Err(e) => return Probe { outcome: Outcome::ConnectFailed, status: None, ms: None, detail: Some(e.to_string()) },
    };
    let started = Instant::now();
    match client.get(url).send().await {
        Ok(r) => {
            let status = r.status().as_u16();
            let ms = Some(started.elapsed().as_millis() as u32);
            // Cloudflare answers non-browser clients with a bot check (403/503 and
            // `cf-mitigated: challenge`). The site answered, so it is reachable.
            let challenge = r.headers().get("cf-mitigated").is_some_and(|v| v.as_bytes().eq_ignore_ascii_case(b"challenge"));
            let (outcome, detail) = match status {
                _ if challenge => (Outcome::Ok, Some("Answered with a bot check, so the site is reachable".to_string())),
                200..=399 => (Outcome::Ok, None),
                403 | 451 => (Outcome::Refused, Some(format!("The site refused access (HTTP {status}); often a region lock"))),
                _ => (Outcome::HttpError, Some(format!("The site answered with HTTP {status}"))),
            };
            Probe { outcome, status: Some(status), ms, detail }
        }
        Err(e) => {
            let (outcome, detail) = classify(&e);
            Probe { outcome, status: None, ms: None, detail: Some(detail) }
        }
    }
}

/// Check one site both ways at once. `kova_proxy` is Kova's local proxy, if connected.
pub async fn check(url: &str, kova_proxy: Option<&str>) -> SiteResult {
    let direct = probe(url, None);
    match kova_proxy {
        Some(p) => {
            let (via, direct) = tokio::join!(probe(url, Some(p)), direct);
            SiteResult { url: url.into(), via_kova: Some(via), direct }
        }
        None => SiteResult { url: url.into(), via_kova: None, direct: direct.await },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn unreachable_port_is_reported_not_panicked() {
        // Port 9 on localhost has no listener: a refused connection, quickly.
        let p = probe("http://127.0.0.1:9/", None).await;
        assert!(matches!(p.outcome, Outcome::ConnectFailed | Outcome::Reset), "{p:?}");
        assert!(p.detail.is_some());
    }

    #[test]
    fn defaults_are_valid_urls() {
        for s in default_sites() {
            assert!(s.url.starts_with("https://"), "{}", s.url);
        }
    }
}
