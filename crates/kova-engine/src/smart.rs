//! Smart routing in the engine: watch Mihomo for connections that fail on their usual
//! path, check the site both ways, and route a confirmed site the other way. Learned
//! sites reach Mihomo as two rule lists it fetches from a small server on loopback, so
//! a new rule applies without restarting the core, also when it runs in the service.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kova_core::options::{SmartRuntime, TEST_URL};
use kova_core::smart::{self, Learned, PROVIDER_DIRECT, PROVIDER_PROXY, Route};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use tokio::sync::{OnceCell, mpsc};
use tokio::task::JoinSet;

use crate::mihomo_api::Mihomo;
use crate::sitecheck::{Outcome, Probe, probe};
use crate::store::{now, read_json, write_json};

/// The learned sites, kept on disk and served to Mihomo.
pub struct Smart {
    file: PathBuf,
    list: Mutex<Vec<Learned>>,
    server: OnceCell<String>,
}

impl Smart {
    pub fn load(file: PathBuf) -> Arc<Self> {
        let mut list: Vec<Learned> = read_json(&file);
        smart::prune(&mut list, now());
        Arc::new(Self { file, list: Mutex::new(list), server: OnceCell::new() })
    }

    /// Newest first.
    pub fn list(&self) -> Vec<Learned> {
        let mut list = self.list.lock().unwrap();
        if smart::prune(&mut list, now()) {
            self.save(&list);
        }
        let mut out = list.clone();
        out.sort_by_key(|l| std::cmp::Reverse(l.learned_at));
        out
    }

    pub fn get(&self, domain: &str) -> Option<Learned> {
        self.list.lock().unwrap().iter().find(|l| l.domain == domain).cloned()
    }

    fn change(&self, f: impl FnOnce(&mut Vec<Learned>)) {
        let mut list = self.list.lock().unwrap();
        f(&mut list);
        self.save(&list);
    }

    fn save(&self, list: &[Learned]) {
        if let Err(e) = write_json(&self.file, &list) {
            tracing::warn!("could not save the learned sites: {e}");
        }
    }

    pub fn learn(&self, item: Learned) {
        self.change(|l| smart::upsert(l, item));
    }

    pub fn forget(&self, domain: &str) -> Option<Learned> {
        let found = self.get(domain)?;
        self.change(|l| l.retain(|x| x.domain != domain));
        Some(found)
    }

    pub fn clear(&self) {
        self.change(Vec::clear);
    }

    /// Start the list server (once) and return the base URL Mihomo fetches from.
    pub async fn lists_url(self: &Arc<Self>) -> std::io::Result<String> {
        self.server
            .get_or_try_init(|| async {
                let listener = TcpListener::bind("127.0.0.1:0").await?;
                // Only Mihomo, given the full URL, can read the lists: they say which
                // sites the user visits.
                let token = uuid::Uuid::new_v4().simple().to_string();
                let base = format!("http://127.0.0.1:{}/{token}/", listener.local_addr()?.port());
                tokio::spawn(serve(listener, token, Arc::downgrade(self)));
                Ok(base)
            })
            .await
            .cloned()
    }
}

async fn serve(listener: TcpListener, token: String, smart: std::sync::Weak<Smart>) {
    loop {
        let Ok((mut sock, _)) = listener.accept().await else { continue };
        let Some(smart) = smart.upgrade() else { return };
        let token = token.clone();
        tokio::spawn(async move {
            let mut buf = Vec::with_capacity(1024);
            let read = tokio::time::timeout(Duration::from_secs(5), async {
                let mut chunk = [0u8; 1024];
                while !buf.windows(4).any(|w| w == b"\r\n\r\n") && buf.len() < 8192 {
                    let n = sock.read(&mut chunk).await?;
                    if n == 0 {
                        break;
                    }
                    buf.extend_from_slice(&chunk[..n]);
                }
                std::io::Result::Ok(())
            });
            if !matches!(read.await, Ok(Ok(()))) {
                return;
            }
            let head = String::from_utf8_lossy(&buf);
            let path = head.lines().next().and_then(|l| l.strip_prefix("GET ")).and_then(|l| l.split(' ').next()).unwrap_or_default();
            let route = match path.strip_prefix(&format!("/{token}/")).and_then(|p| p.strip_suffix(".txt")) {
                Some(PROVIDER_PROXY) => Some(Route::Proxy),
                Some(PROVIDER_DIRECT) => Some(Route::Direct),
                _ => None,
            };
            let reply = match route {
                Some(r) => {
                    let body = smart::provider_text(&smart.list.lock().unwrap(), r);
                    format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                }
                None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
            };
            let _ = sock.write_all(reply.as_bytes()).await;
            let _ = sock.shutdown().await;
        });
    }
}

/// Make the running Mihomo fetch both lists again.
pub async fn refresh(api: &Mihomo) {
    for name in [PROVIDER_PROXY, PROVIDER_DIRECT] {
        if let Err(e) = api.update_rule_provider(name).await {
            tracing::warn!("could not refresh {name}: {e}");
        }
    }
}

/// What the learner needs from the engine.
pub struct Hooks {
    /// Learning happens only while this holds (Rule mode, smart routing on).
    pub active: Box<dyn Fn() -> bool + Send + Sync>,
    pub learned: Box<dyn Fn(&Learned) + Send + Sync>,
}

/// A site whose connection failed on `failed`, its usual path.
#[derive(Debug, Clone)]
struct Suspect {
    host: String,
    port: u16,
    failed: Route,
}

/// Watch, check and learn for as long as the connection runs. Aborting the task stops
/// everything it started.
pub async fn learn(smart: Arc<Smart>, api: Mihomo, ports: SmartRuntime, hooks: Hooks) {
    refresh(&api).await;
    let hooks = Arc::new(hooks);
    let (tx, rx) = mpsc::channel(256);
    tokio::join!(watch_logs(api.clone(), tx.clone()), watch_connections(api.clone(), tx, hooks.clone()), check(smart, api, ports, hooks, rx));
}

/// Mihomo reports a connection it could not open as a warning. Subscribing through the
/// controller gets these whatever log level the user chose.
async fn watch_logs(api: Mihomo, tx: mpsc::Sender<Suspect>) {
    loop {
        let _ = api
            .stream("/logs?level=warning", |v| {
                if let Some(f) = v["payload"].as_str().and_then(smart::parse_dial_failure) {
                    // Only traffic no rule chose a route for: a rule the user or a
                    // preset set on purpose is never second-guessed.
                    if f.catch_all {
                        let failed = if f.direct { Route::Direct } else { Route::Proxy };
                        let _ = tx.try_send(Suspect { host: f.host, port: f.port, failed });
                    }
                }
                !tx.is_closed()
            })
            .await;
        if tx.is_closed() {
            return;
        }
        tokio::time::sleep(Duration::from_secs(3)).await;
    }
}

/// Blocking that drops packets instead of refusing them leaves a direct connection
/// open with the greeting sent and nothing ever coming back.
async fn watch_connections(api: Mihomo, tx: mpsc::Sender<Suspect>, hooks: Arc<Hooks>) {
    const SILENT_FOR: Duration = Duration::from_secs(5);
    let mut first_seen: HashMap<String, Instant> = HashMap::new();
    let mut reported: HashSet<String> = HashSet::new();
    loop {
        tokio::time::sleep(Duration::from_millis(1500)).await;
        if tx.is_closed() {
            return;
        }
        if !(hooks.active)() {
            continue;
        }
        let Ok(snap) = api.connections().await else { continue };
        let open: HashSet<&str> = snap.connections.iter().map(|c| c.id.as_str()).collect();
        first_seen.retain(|id, _| open.contains(id.as_str()));
        reported.retain(|id| open.contains(id.as_str()));
        for c in &snap.connections {
            let silent = c.network == "TCP" && c.port == "443" && c.rule == "Match" && c.chains.last().is_some_and(|x| x == "DIRECT") && c.upload > 0 && c.download == 0;
            if !silent {
                continue;
            }
            let since = *first_seen.entry(c.id.clone()).or_insert_with(Instant::now);
            if since.elapsed() >= SILENT_FOR && reported.insert(c.id.clone()) {
                let _ = tx.try_send(Suspect { host: c.host.clone(), port: 443, failed: Route::Direct });
            }
        }
    }
}

const RECHECK_AFTER: Duration = Duration::from_secs(600);

async fn check(smart: Arc<Smart>, api: Mihomo, ports: SmartRuntime, hooks: Arc<Hooks>, mut rx: mpsc::Receiver<Suspect>) {
    let mut checked: HashMap<String, Instant> = HashMap::new();
    let mut running = JoinSet::new();
    let health = Arc::new(Health::new(TEST_URL));
    while let Some(s) = rx.recv().await {
        while running.try_join_next().is_some() {}
        if !(hooks.active)() {
            continue;
        }
        let Some(domain) = smart::rule_domain(&s.host) else { continue };
        if smart.get(&domain).is_some() || checked.get(&s.host).is_some_and(|t| t.elapsed() < RECHECK_AFTER) || running.len() >= 2 {
            continue;
        }
        let url = match s.port {
            443 => format!("https://{}/", s.host),
            80 => format!("http://{}/", s.host),
            _ => continue,
        };
        checked.insert(s.host.clone(), Instant::now());
        checked.retain(|_, t| t.elapsed() < RECHECK_AFTER);
        let (smart, api, ports, hooks, health) = (smart.clone(), api.clone(), ports.clone(), hooks.clone(), health.clone());
        running.spawn(async move {
            let Some(reason) = confirm(&url, s.failed, &ports, &health).await else { return };
            let route = match s.failed {
                Route::Direct => Route::Proxy,
                Route::Proxy => Route::Direct,
            };
            let item = Learned { domain: domain.clone(), route, reason, learned_at: now() };
            smart.learn(item.clone());
            refresh(&api).await;
            // Connections still stuck on the failing path would keep the page hanging;
            // closing them makes the browser retry on the new route.
            if let Ok(snap) = api.connections().await {
                for c in snap.connections.iter().filter(|c| c.rule == "Match" && smart::covers(&domain, &c.host)) {
                    let _ = api.close_connection(&c.id).await;
                }
            }
            (hooks.learned)(&item);
        });
    }
}

fn answered(p: &Probe) -> bool {
    matches!(p.outcome, Outcome::Ok | Outcome::HttpError)
}

/// Whether the site fails on `failed` and works the other way, twice. Returns how it
/// failed. A path that is down for everything proves nothing about one site, so each
/// path is first checked against a site that always answers.
async fn confirm(url: &str, failed: Route, ports: &SmartRuntime, health: &Health) -> Option<String> {
    let via = |port: u16| format!("http://127.0.0.1:{port}");
    let (usual, other) = match failed {
        Route::Direct => (via(ports.direct_port), via(ports.proxy_port)),
        Route::Proxy => (via(ports.proxy_port), via(ports.direct_port)),
    };
    if answered(&probe(url, Some(&usual)).await) || !answered(&probe(url, Some(&other)).await) {
        return None;
    }
    if !health.ok(&usual).await || !health.ok(&other).await {
        return None;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    let again = probe(url, Some(&usual)).await;
    if answered(&again) {
        return None;
    }
    serde_json::to_value(&again.outcome).ok().and_then(|v| v.as_str().map(String::from))
}

/// Whether a path reaches the internet at all, remembered for a minute.
struct Health {
    url: String,
    seen: tokio::sync::Mutex<HashMap<String, (Instant, bool)>>,
}

impl Health {
    fn new(url: &str) -> Self {
        Self { url: url.into(), seen: Default::default() }
    }

    async fn ok(&self, proxy: &str) -> bool {
        let mut seen = self.seen.lock().await;
        if let Some((at, ok)) = seen.get(proxy) {
            if at.elapsed() < Duration::from_secs(60) {
                return *ok;
            }
        }
        let ok = matches!(probe(&self.url, Some(proxy)).await.outcome, Outcome::Ok);
        seen.insert(proxy.to_string(), (Instant::now(), ok));
        ok
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stand-in for one of Mihomo's probe ports: an HTTP proxy that answers every
    /// request except those for hosts in `drops`, whose connection it just closes.
    async fn fake_path(drops: &'static [&'static str]) -> u16 {
        let l = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            loop {
                let (mut sock, _) = l.accept().await.unwrap();
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let n = sock.read(&mut buf).await.unwrap_or(0);
                    let head = String::from_utf8_lossy(&buf[..n]).to_string();
                    if n == 0 || drops.iter().any(|d| head.contains(d)) {
                        return;
                    }
                    let _ = sock.write_all(b"HTTP/1.1 204 No Content
Connection: close

").await;
                });
            }
        });
        port
    }

    async fn ports(direct: &'static [&'static str], proxy: &'static [&'static str]) -> SmartRuntime {
        SmartRuntime { lists_url: String::new(), direct_port: fake_path(direct).await, proxy_port: fake_path(proxy).await }
    }

    #[tokio::test]
    async fn learns_only_what_fails_on_one_path_and_works_on_the_other() {
        let health = || Health::new("http://health.test/");
        let site = "http://blocked.test/";
        // Blocked direct, fine through the proxy: send it through the proxy.
        let p = ports(&["blocked.test"], &[]).await;
        assert!(confirm(site, Route::Direct, &p, &health()).await.is_some());
        // Refused by the proxy, fine direct: the reverse.
        let p = ports(&[], &["blocked.test"]).await;
        assert!(confirm(site, Route::Proxy, &p, &health()).await.is_some());
        // Works both ways (a passing hiccup): nothing to learn.
        let p = ports(&[], &[]).await;
        assert!(confirm(site, Route::Direct, &p, &health()).await.is_none());
        // Down for everyone: routing it elsewhere wouldn't help.
        let p = ports(&["blocked.test"], &["blocked.test"]).await;
        assert!(confirm(site, Route::Direct, &p, &health()).await.is_none());
        // The direct path is down altogether (no internet): one site proves nothing.
        let p = ports(&["blocked.test", "health.test"], &[]).await;
        assert!(confirm(site, Route::Direct, &p, &health()).await.is_none());
    }

    #[tokio::test]
    async fn serves_each_list_only_under_its_token() {
        let dir = tempfile::tempdir().unwrap();
        let s = Smart::load(dir.path().join("learned.json"));
        s.learn(Learned { domain: "blocked.example".into(), route: Route::Proxy, reason: "reset".into(), learned_at: now() });
        s.learn(Learned { domain: "local.example".into(), route: Route::Direct, reason: "refused".into(), learned_at: now() });
        let base = s.lists_url().await.unwrap();
        assert_eq!(s.lists_url().await.unwrap(), base, "one server per app");
        let get = |u: String| async move { reqwest::Client::builder().no_proxy().build().unwrap().get(u).send().await.unwrap() };
        let proxy = get(format!("{base}{PROVIDER_PROXY}.txt")).await.text().await.unwrap();
        assert_eq!(proxy, "+.kova-learned.invalid\n+.blocked.example\n");
        let direct = get(format!("{base}{PROVIDER_DIRECT}.txt")).await.text().await.unwrap();
        assert!(direct.ends_with("+.local.example\n"));
        let port = base.split(':').nth(2).unwrap().split('/').next().unwrap();
        assert_eq!(get(format!("http://127.0.0.1:{port}/wrong/{PROVIDER_PROXY}.txt")).await.status(), 404);

        // Kept across restarts; forgetting updates what is served.
        let again = Smart::load(dir.path().join("learned.json"));
        assert_eq!(again.list().len(), 2);
        assert!(s.forget("blocked.example").is_some());
        let proxy = get(format!("{base}{PROVIDER_PROXY}.txt")).await.text().await.unwrap();
        assert_eq!(proxy, "+.kova-learned.invalid\n");
    }
}
