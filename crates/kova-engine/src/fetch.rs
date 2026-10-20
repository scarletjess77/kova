//! HTTP downloads: subscriptions and release metadata.

use std::time::Duration;

use kova_core::subscription::{UserInfo, decode_title, parse_userinfo};

use crate::Error;

/// Providers choose the response format from the User-Agent. Mentioning `clash.meta`
/// gets the richest one (a full Mihomo config with groups and rules) from most panels.
pub const DEFAULT_SUB_UA: &str = concat!("clash.meta/1.19 mihomo Kova/", env!("CARGO_PKG_VERSION"));
pub const APP_UA: &str = concat!("Kova/", env!("CARGO_PKG_VERSION"));

/// Start every HTTP client here. On Android the certificate check reqwest uses by
/// default needs a Java component Kova doesn't ship, so the device's own trusted
/// certificates are handed over instead.
pub fn builder() -> reqwest::ClientBuilder {
    let b = reqwest::Client::builder();
    #[cfg(target_os = "android")]
    let b = b.tls_certs_only(android_roots().iter().cloned());
    b
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
}

pub async fn subscription(url: &str, user_agent: Option<&str>, proxy: Option<&str>) -> Result<Subscription, Error> {
    let c = client(proxy, user_agent.unwrap_or(DEFAULT_SUB_UA))?;
    let resp = c.get(url).send().await?;
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
        body: resp.text().await?,
    })
}
