//! Smart routing in the engine: watch Mihomo for connections that fail on their usual
//! path, check the site both ways, and route a confirmed site the other way. Learned
//! sites reach Mihomo as two rule lists it fetches from a small server on loopback, so
//! a new rule applies without restarting the core, also when it runs in the service.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use kova_core::options::{SmartRuntime, TEST_URL};
use std::net::{IpAddr, SocketAddr};

use kova_core::smart::{self, Learned, PROVIDERS, Route};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
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
        let Ok((mut sock, _)) = listener.accept().await else {
            // Out of descriptors or similar: wait instead of spinning.
            tokio::time::sleep(Duration::from_secs(1)).await;
            continue;
        };
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
            let name = path.strip_prefix(&format!("/{token}/")).and_then(|p| p.strip_suffix(".txt"));
            let body = name.and_then(|n| smart::provider_text(&smart.list.lock().unwrap(), n));
            let reply = match body {
                Some(body) => {
                    format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len())
                }
                None => "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string(),
            };
            let _ = sock.write_all(reply.as_bytes()).await;
            let _ = sock.shutdown().await;
        });
    }
}

/// Make the running Mihomo fetch the lists again.
pub async fn refresh(api: &Mihomo) {
    for name in PROVIDERS {
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
    /// Mihomo could not open the connection at all (rather than it going silent).
    dial_failed: bool,
}

/// Direct HTTPS connections the log reported opening, by host, with when: those that
/// never get an answer are checked (see `watch_connections`).
type Opened = Arc<Mutex<HashMap<String, Instant>>>;

/// Watch, check and learn for as long as the connection runs. Aborting the task stops
/// everything it started.
pub async fn learn(smart: Arc<Smart>, api: Mihomo, ports: SmartRuntime, hooks: Hooks) {
    refresh(&api).await;
    let hooks = Arc::new(hooks);
    let scope = Arc::new(ports.scope.clone());
    let opened: Opened = Default::default();
    let (tx, rx) = mpsc::channel(256);
    tokio::join!(
        watch_logs(api.clone(), tx.clone(), scope.clone(), opened.clone()),
        watch_connections(api.clone(), tx, hooks.clone(), scope, opened),
        check(smart, api, ports, hooks, rx)
    );
}

/// Mihomo logs every connection it opens, with the rule that chose its route, and
/// warns about each it could not open. Subscribing through the controller gets these
/// whatever log level the user chose.
async fn watch_logs(api: Mihomo, tx: mpsc::Sender<Suspect>, scope: Arc<smart::Scope>, opened: Opened) {
    loop {
        let _ = api
            .stream("/logs?level=info", |v| {
                let line = v["payload"].as_str().unwrap_or_default();
                // Only routes no rule chose on purpose: the user's rules and presets
                // are never second-guessed, a subscription's only when allowed.
                if let Some(f) = smart::parse_dial_failure(line) {
                    if scope.covers(&f.rule) {
                        let failed = if f.direct { Route::Direct } else { Route::Proxy };
                        let _ = tx.try_send(Suspect { host: f.host, port: f.port, failed, dial_failed: true });
                    }
                } else if let Some(o) = smart::parse_opened(line) {
                    // Blocking often lets the connection open and resets it right after
                    // the greeting: too quick to ever show among open connections.
                    if o.direct && o.port == 443 && scope.covers(&o.rule) {
                        opened.lock().unwrap().entry(o.host).or_insert_with(Instant::now);
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

/// Two ways blocking shows among direct HTTPS connections:
/// - dropped packets: the connection stays open, greeting sent, nothing ever back;
/// - a reset right after the greeting: the connection was opened (the log said so) but
///   gone before it got an answer, so the host is never seen receiving anything.
///
/// A site that works has connections that receive data, so it's left alone.
async fn watch_connections(api: Mihomo, tx: mpsc::Sender<Suspect>, hooks: Arc<Hooks>, scope: Arc<smart::Scope>, opened: Opened) {
    const SILENT_FOR: Duration = Duration::from_secs(5);
    const ANSWER_WITHIN: Duration = Duration::from_secs(6);
    let mut first_seen: HashMap<String, Instant> = HashMap::new();
    let mut reported: HashSet<String> = HashSet::new();
    let mut answered: HashMap<String, Instant> = HashMap::new();
    loop {
        tokio::time::sleep(Duration::from_secs(2)).await;
        if tx.is_closed() {
            return;
        }
        if !(hooks.active)() {
            opened.lock().unwrap().clear();
            continue;
        }
        let Ok(snap) = api.connections().await else { continue };
        let open: HashSet<&str> = snap.connections.iter().map(|c| c.id.as_str()).collect();
        first_seen.retain(|id, _| open.contains(id.as_str()));
        reported.retain(|id| open.contains(id.as_str()));
        answered.retain(|_, t| t.elapsed() < Duration::from_secs(120));
        for c in &snap.connections {
            if c.download > 0 {
                answered.insert(c.host.clone(), Instant::now());
            }
            let direct = c.network == "TCP" && c.port == "443" && c.chains.last().is_some_and(|x| x == "DIRECT");
            let silent = direct && scope.covers(&api_rule_key(&c.rule)) && c.upload > 0 && c.download == 0;
            if !silent {
                continue;
            }
            let since = *first_seen.entry(c.id.clone()).or_insert_with(Instant::now);
            if since.elapsed() >= SILENT_FOR && reported.insert(c.id.clone()) {
                let _ = tx.try_send(Suspect { host: c.host.clone(), port: 443, failed: Route::Direct, dial_failed: false });
            }
        }
        // Opened a while ago and still nothing received for that host: check it.
        let due: Vec<String> = {
            let mut o = opened.lock().unwrap();
            let due = o.iter().filter(|(_, t)| t.elapsed() >= ANSWER_WITHIN).map(|(h, _)| h.clone()).collect::<Vec<_>>();
            o.retain(|_, t| t.elapsed() < ANSWER_WITHIN);
            due
        };
        for host in due.into_iter().filter(|h| !answered.contains_key(h)) {
            let _ = tx.try_send(Suspect { host, port: 443, failed: Route::Direct, dial_failed: false });
        }
    }
}

/// The controller shows a connection's rule as `GeoSite category-games`; the log, as
/// `GeoSite(category-games)`.
fn api_rule_key(rule: &str) -> String {
    match rule.split_once(' ') {
        Some((kind, payload)) => smart::rule_key(kind, payload),
        None => rule.to_string(),
    }
}

/// A host checked once isn't checked again for this long, whatever the result: sites
/// that work keep opening short connections, and each would cost a check.
const RECHECK_AFTER: Duration = Duration::from_secs(1800);

async fn check(smart: Arc<Smart>, api: Mihomo, ports: SmartRuntime, hooks: Arc<Hooks>, mut rx: mpsc::Receiver<Suspect>) {
    let mut checked: HashMap<String, Instant> = HashMap::new();
    let mut running = JoinSet::new();
    let health = Arc::new(Health::new(TEST_URL));
    while let Some(s) = rx.recv().await {
        while running.try_join_next().is_some() {}
        if !(hooks.active)() {
            continue;
        }
        let Some(domain) = smart::rule_target(&s.host) else { continue };
        if smart.get(&domain).is_some() || checked.get(&s.host).is_some_and(|t| t.elapsed() < RECHECK_AFTER) || running.len() >= 2 {
            continue;
        }
        // A bare address (an app that connects without a name, as TUN mode shows) is
        // checked at the connection level: whatever it speaks, it either connects or it
        // doesn't. Only for a connection that could not be opened at all, and only
        // toward the proxy: an address that refuses the proxy can't be told apart from
        // one the proxy server can't reach.
        let target = match smart::parse_ip(&s.host) {
            Some(ip) if s.dial_failed && s.failed == Route::Direct => Target::Addr(SocketAddr::new(ip, s.port)),
            Some(_) => continue,
            None => match s.port {
                443 => Target::Url(format!("https://{}/", s.host)),
                80 => Target::Url(format!("http://{}/", s.host)),
                _ => continue,
            },
        };
        checked.insert(s.host.clone(), Instant::now());
        checked.retain(|_, t| t.elapsed() < RECHECK_AFTER);
        let (smart, api, ports, hooks, health) = (smart.clone(), api.clone(), ports.clone(), hooks.clone(), health.clone());
        running.spawn(async move {
            let Some(reason) = confirm(&target, s.failed, &ports, &health).await else { return };
            // The check takes a while; smart routing may have been turned off meanwhile.
            if !(hooks.active)() {
                return;
            }
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

/// What gets checked: a website, or a bare address and port.
enum Target {
    Url(String),
    Addr(SocketAddr),
}

/// Try `target` through one of Mihomo's probe ports. `Err` holds how it failed.
async fn attempt(target: &Target, port: u16) -> Result<(), String> {
    match target {
        Target::Url(url) => {
            let p = probe(url, Some(&format!("http://127.0.0.1:{port}"))).await;
            if answered(&p) {
                return Ok(());
            }
            Err(serde_json::to_value(&p.outcome).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_else(|| "connect-failed".into()))
        }
        Target::Addr(addr) => if reaches(port, *addr).await { Ok(()) } else { Err("connect-failed".into()) },
    }
}

/// Whether the site fails on `failed` and works the other way, twice. Returns how it
/// failed. A path that is down for everything proves nothing about one site, so each
/// path is first checked against a site that always answers.
async fn confirm(target: &Target, failed: Route, ports: &SmartRuntime, health: &Health) -> Option<String> {
    let (usual, other) = match failed {
        Route::Direct => (ports.direct_port, ports.proxy_port),
        Route::Proxy => (ports.proxy_port, ports.direct_port),
    };
    if attempt(target, usual).await.is_ok() || attempt(target, other).await.is_err() {
        return None;
    }
    let via = |port: u16| format!("http://127.0.0.1:{port}");
    if !health.ok(&via(usual)).await || !health.ok(&via(other)).await {
        return None;
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    attempt(target, usual).await.err()
}

/// Whether a TCP connection to `dest` can be opened through the probe port `port`,
/// using its SOCKS5 side. Mihomo may confirm the request before it has connected, so a
/// connection it then could not open shows as it closing; one that stays open waiting
/// for us to speak, or that greets us, is up.
pub async fn reaches(port: u16, dest: SocketAddr) -> bool {
    let attempt = async {
        let mut s = TcpStream::connect(("127.0.0.1", port)).await?;
        s.write_all(&[5, 1, 0]).await?;
        let mut hello = [0u8; 2];
        s.read_exact(&mut hello).await?;
        if hello != [5, 0] {
            return Ok(false);
        }
        let mut req = vec![5, 1, 0];
        match dest.ip() {
            IpAddr::V4(ip) => {
                req.push(1);
                req.extend_from_slice(&ip.octets());
            }
            IpAddr::V6(ip) => {
                req.push(4);
                req.extend_from_slice(&ip.octets());
            }
        }
        req.extend_from_slice(&dest.port().to_be_bytes());
        s.write_all(&req).await?;
        let mut head = [0u8; 4];
        s.read_exact(&mut head).await?;
        if head[1] != 0 {
            return Ok(false);
        }
        let rest = match head[3] {
            1 => 6,
            4 => 18,
            3 => {
                let mut len = [0u8; 1];
                s.read_exact(&mut len).await?;
                len[0] as usize + 2
            }
            _ => return Ok(false),
        };
        let mut bound = vec![0u8; rest];
        s.read_exact(&mut bound).await?;
        let mut first = [0u8; 1];
        Ok(match tokio::time::timeout(Duration::from_secs(8), s.read(&mut first)).await {
            Err(_) => true,
            Ok(Ok(n)) => n > 0,
            Ok(Err(_)) => false,
        })
    };
    matches!(tokio::time::timeout(Duration::from_secs(20), attempt).await, Ok(Ok::<bool, std::io::Error>(true)))
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
    use kova_core::smart::{PROVIDER_DIRECT, PROVIDER_PROXY, PROVIDER_PROXY_IP};

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
        SmartRuntime { lists_url: String::new(), direct_port: fake_path(direct).await, proxy_port: fake_path(proxy).await, scope: Default::default() }
    }

    #[tokio::test]
    async fn learns_only_what_fails_on_one_path_and_works_on_the_other() {
        let health = || Health::new("http://health.test/");
        let site = &Target::Url("http://blocked.test/".into());
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
        s.learn(Learned { domain: "149.154.167.51".into(), route: Route::Proxy, reason: "connect-failed".into(), learned_at: now() });
        let proxy = get(format!("{base}{PROVIDER_PROXY}.txt")).await.text().await.unwrap();
        assert_eq!(proxy, "+.kova-learned.invalid\n+.blocked.example\n");
        let direct = get(format!("{base}{PROVIDER_DIRECT}.txt")).await.text().await.unwrap();
        assert!(direct.ends_with("+.local.example\n"));
        let ips = get(format!("{base}{PROVIDER_PROXY_IP}.txt")).await.text().await.unwrap();
        assert_eq!(ips, "255.255.255.255/32\n149.154.167.51/32\n");
        let port = base.split(':').nth(2).unwrap().split('/').next().unwrap();
        assert_eq!(get(format!("http://127.0.0.1:{port}/wrong/{PROVIDER_PROXY}.txt")).await.status(), 404);
        assert_eq!(get(format!("{base}nothing.txt")).await.status(), 404);

        // Kept across restarts; forgetting updates what is served.
        let again = Smart::load(dir.path().join("learned.json"));
        assert_eq!(again.list().len(), 3);
        assert!(s.forget("blocked.example").is_some());
        let proxy = get(format!("{base}{PROVIDER_PROXY}.txt")).await.text().await.unwrap();
        assert_eq!(proxy, "+.kova-learned.invalid\n");
    }
}
