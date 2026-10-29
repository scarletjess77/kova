//! Checks that explain what is going on: "why isn't it working?" step by step, what
//! sites can see (the privacy check), and how fast the connection really is.

use std::time::{Duration, Instant};

use kova_core::options::{Mode, TEST_URL};
use serde::Serialize;

use crate::Engine;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum State {
    Ok,
    Warn,
    Fail,
    /// Not checked: an earlier step failed in a way that makes this one meaningless.
    Skipped,
}

/// One step of a check. The interface words it by `id` and `state`; `detail` carries the
/// specifics (a node name, an address, an error), `fix` names the one action that helps.
#[derive(Debug, Clone, Serialize)]
pub struct Check {
    pub id: &'static str,
    pub state: State,
    pub detail: Option<String>,
    pub fix: Option<&'static str>,
}

fn check(id: &'static str, state: State, detail: Option<String>, fix: Option<&'static str>) -> Check {
    Check { id, state, detail, fix }
}

fn client(proxy: Option<&str>, timeout: u64) -> Option<reqwest::Client> {
    let b = crate::fetch::builder().timeout(Duration::from_secs(timeout));
    let b = match proxy {
        Some(p) => b.proxy(reqwest::Proxy::all(p).ok()?),
        None => b.no_proxy(),
    };
    b.build().ok()
}

/// Whether `url` answers at all (any HTTP status counts), and how it failed if not.
async fn answers(url: &str, proxy: Option<&str>) -> Result<(), String> {
    let c = client(proxy, 8).ok_or("no client")?;
    c.get(url).send().await.map(|_| ()).map_err(|e| crate::Error::Net(e).to_string())
}

/// The public address a plain-text echo service sees.
async fn public_ip(url: &str, proxy: Option<&str>) -> Option<String> {
    let c = client(proxy, 8)?;
    let t = c.get(url).send().await.ok()?.text().await.ok()?;
    let t = t.trim();
    t.parse::<std::net::IpAddr>().ok().map(|_| t.to_string())
}

impl Engine {
    /// Why isn't it working? Each step either passes or says what is wrong and what fixes
    /// it; once a step makes the rest meaningless, they are skipped.
    pub async fn diagnose(&self) -> Vec<Check> {
        use State::*;
        let mut out = Vec::new();
        let settings = self.settings();
        let st = self.status();

        let has_profile = settings.active_profile.as_ref().is_some_and(|id| self.profiles().iter().any(|p| &p.id == id));
        out.push(if has_profile { check("profile", Ok, None, None) } else { check("profile", Fail, None, Some("add-profile")) });

        let missing: Vec<String> = self.core_status(false).await.into_iter().filter(|c| c.installed.is_none()).map(|c| c.core.label().to_string()).collect();
        out.push(if missing.is_empty() { check("cores", Ok, None, None) } else { check("cores", Fail, Some(missing.join(", ")), Some("install-cores")) });

        // The network itself, without Kova: a captive portal or no signal looks like a
        // broken VPN otherwise.
        let direct = match answers(TEST_URL, None).await {
            Result::Ok(()) => Result::Ok(()),
            Err(_) => answers("https://ya.ru/", None).await,
        };
        let online = direct.is_ok();
        out.push(match direct {
            Result::Ok(()) => check("internet", Ok, None, None),
            Err(e) => check("internet", Fail, Some(e), None),
        });

        if !has_profile || !missing.is_empty() || !online {
            for id in ["running", "node", "dns", "through", "capture"] {
                out.push(check(id, Skipped, None, None));
            }
            return out;
        }

        if !st.running {
            out.push(check("running", Fail, st.error.clone(), Some("connect")));
            for id in ["node", "dns", "through", "capture"] {
                out.push(check(id, Skipped, None, None));
            }
            return out;
        }
        out.push(check("running", Ok, None, None));

        let api = self.mihomo().await;
        let node = self.current_node().await;
        match (&api, &node) {
            (Some(api), Some(node)) if settings.core.mode != Mode::Direct => {
                match api.delay(node, TEST_URL, 5000).await.ok().flatten() {
                    Some(ms) => out.push(check("node", Ok, Some(format!("{node} · {ms} ms")), None)),
                    None => out.push(check("node", Fail, Some(node.clone()), Some("switch-node"))),
                }
            }
            _ if settings.core.mode == Mode::Direct => out.push(check("node", Warn, None, Some("rule-mode"))),
            _ => out.push(check("node", Skipped, None, None)),
        }

        match &api {
            Some(api) => match api.dns_query("example.com").await {
                Result::Ok(ips) if !ips.is_empty() => out.push(check("dns", Ok, Some(ips.join(", ")), None)),
                Result::Ok(_) => out.push(check("dns", Fail, None, Some("settings"))),
                Err(e) => out.push(check("dns", Fail, Some(e.to_string()), Some("settings"))),
            },
            None => out.push(check("dns", Skipped, None, None)),
        }

        out.push(match self.proxy_url() {
            Some(p) => match answers(TEST_URL, Some(&p)).await {
                Result::Ok(()) => check("through", Ok, None, None),
                Err(e) => check("through", Fail, Some(e), Some("switch-node")),
            },
            None => check("through", Skipped, None, None),
        });

        // Is anything actually sent to Kova? A running core with nothing routed to it
        // looks like "the VPN does nothing".
        let captured = st.tun_active || (!self.has_tun_fd() && settings.system_proxy);
        out.push(if captured { check("capture", Ok, None, None) } else { check("capture", Warn, None, Some("system-proxy")) });
        out
    }

    /// What sites can see: the address they see with Kova and without, how name lookups
    /// are made, and what may go around Kova.
    pub async fn privacy_check(&self) -> Vec<Check> {
        use State::*;
        let mut out = Vec::new();
        let settings = self.settings();
        let st = self.status();
        let android = self.has_tun_fd();

        let direct = public_ip("https://api.ipify.org", None).await;
        let through = match self.proxy_url() {
            Some(p) => public_ip("https://api.ipify.org", Some(&p)).await,
            None => None,
        };
        out.push(match (st.running, &direct, &through) {
            (false, _, _) => check("exit", Warn, direct.clone(), Some("connect")),
            (true, Some(d), Some(t)) if d == t => check("exit", Warn, Some(t.clone()), None),
            (true, _, Some(t)) => check("exit", Ok, Some(t.clone()), None),
            (true, _, None) => check("exit", Fail, None, Some("diagnose")),
        });

        // Name lookups: encrypted resolvers keep the network from reading them.
        let encrypted = |s: &String| s.starts_with("https://") || s.starts_with("tls://") || s.starts_with("quic://");
        let ns = &settings.core.dns.nameservers;
        out.push(if !ns.is_empty() && ns.iter().all(encrypted) {
            check("dns-encrypted", Ok, Some(ns.join(", ")), None)
        } else {
            check("dns-encrypted", Warn, Some(ns.join(", ")), Some("settings"))
        });

        // IPv6: TUN keeps it inside the VPN whatever the IPv6 setting, and the system proxy
        // carries whatever goes through it, IPv6 sites included.
        out.push(check("ipv6", Ok, None, None));
        // Apps that ignore the system proxy, and browsers' WebRTC, reach sites directly
        // unless everything is captured.
        out.push(if android || st.tun_active { check("all-apps", Ok, None, None) } else { check("all-apps", Warn, None, Some("tun")) });
        out
    }

    /// Download speed in megabits per second, through Kova and directly, from up to
    /// 25 MB or 8 seconds of a speed-test file each.
    pub async fn speed_test(&self) -> (Option<f64>, Option<f64>) {
        async fn measure(proxy: Option<String>) -> Option<f64> {
            let c = client(proxy.as_deref(), 20)?;
            let mut r = c.get("https://speed.cloudflare.com/__down?bytes=25000000").send().await.ok()?.error_for_status().ok()?;
            let start = Instant::now();
            let mut got = 0u64;
            while start.elapsed() < Duration::from_secs(8) {
                match tokio::time::timeout(Duration::from_secs(8), r.chunk()).await {
                    Result::Ok(Result::Ok(Some(c))) => got += c.len() as u64,
                    _ => break,
                }
            }
            let secs = start.elapsed().as_secs_f64();
            (got > 0 && secs > 0.0).then(|| got as f64 * 8.0 / secs / 1_000_000.0)
        }
        let through = match self.proxy_url() {
            Some(p) => measure(Some(p)).await,
            None => None,
        };
        (through, measure(None).await)
    }
}
