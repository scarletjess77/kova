//! The engine: owns settings and profiles, starts and supervises cores, and publishes
//! live events (status, traffic, logs) for the UI.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock, Weak};
use std::time::Duration;

use kova_core::compat::{self, CoreKind, CorePreference, Plan};
use kova_core::gen_mihomo::{self, Bridges};
use kova_core::gen_xray::{self, Selection};
use kova_core::model::Node;
use kova_core::options::{AUTO, FALLBACK, Mode, PROXY, Rule, RuleKind, RuntimeOptions, SmartRuntime};
use kova_core::subscription::{self, Format};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, broadcast, watch};
use tokio::task::JoinHandle;

use crate::cores::{self, CoreStatus};
use crate::mihomo_api::{ConnectionsSnapshot, Mihomo, ProxyGroup, ProxyInfo, ProxySnapshot};
use crate::process::{CoreProcess, LineSink, check_config};
use crate::store::{Paths, Profile, ProfileIndex, Settings, Source, now};
use crate::{Error, Result, fetch, sysproxy};

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum Event {
    Status(Status),
    Traffic { up: u64, down: u64, up_total: u64, down_total: u64 },
    Log { source: String, level: String, message: String },
    Memory { bytes: u64 },
    Profiles,
    Delays { delays: HashMap<String, Option<u32>> },
    /// Smart routing learned a site.
    Learned { domain: String, route: kova_core::smart::Route, reason: String },
    /// Smart routing suggests an exception to a rule the user owns.
    Suggested { domain: String, route: kova_core::smart::Route, rule: String, origin: kova_core::smart::Origin },
    /// Something the user should hear about once, in plain words.
    Notice { text: String },
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct Status {
    pub running: bool,
    pub busy: bool,
    pub core: Option<CoreKind>,
    pub sidecar: bool,
    /// Unix seconds the connection started.
    pub since: Option<i64>,
    pub error: Option<String>,
    pub mixed_port: u16,
    pub bridged: Vec<String>,
    pub dropped: Vec<(String, String)>,
    /// TUN was requested and is actually running.
    pub tun_active: bool,
    /// Something the user should know that didn't stop the connection.
    pub warning: Option<String>,
}

/// A node as the UI lists it, available without a running core.
#[derive(Debug, Clone, Serialize)]
pub struct NodeView {
    pub name: String,
    pub protocol: String,
    pub transport: String,
    pub server: String,
    pub port: u16,
    pub mihomo: bool,
    pub xray: bool,
    pub link: Option<String>,
}

impl NodeView {
    fn of(n: &Node) -> Self {
        Self {
            name: n.name.clone(),
            protocol: n.protocol_label(),
            transport: n.transport_label(),
            server: n.server.clone(),
            port: n.port,
            mihomo: compat::supports(CoreKind::Mihomo, n),
            xray: compat::supports(CoreKind::Xray, n),
            link: kova_core::links::to_link(n),
        }
    }
}

/// The profile id that stands for every profile's servers in one list (all but those in
/// `Settings::combined_skip`). Each profile still works on its own too.
pub const COMBINED: &str = "combined";

/// Editable profile fields; `None` leaves a field as it is.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProfilePatch {
    pub name: Option<String>,
    pub core: Option<CorePreference>,
    pub url: Option<String>,
    pub user_agent: Option<String>,
    pub update_hours: Option<u32>,
    pub via_proxy: Option<bool>,
    pub headers: Option<Vec<crate::store::Header>>,
}

#[derive(Clone, Copy, PartialEq)]
enum Purpose {
    Main,
    /// A short-lived instance on spare ports used only to measure latency.
    Tester,
}

/// The core that owns the ports: a child process, or Mihomo inside the Kova service.
/// One exists per connection, so the size difference between the two doesn't matter.
#[allow(clippy::large_enum_variant)]
enum Front {
    Local(CoreProcess),
    Service(crate::service::RemoteCore),
}

impl Front {
    async fn exited(&mut self) -> Option<String> {
        match self {
            Front::Local(p) => p.exited(),
            Front::Service(s) => s.exited().await,
        }
    }

    fn last_error(&self) -> String {
        match self {
            Front::Local(p) => p.last_error(),
            Front::Service(s) => s.last_error(),
        }
    }

    async fn stop(self) {
        match self {
            Front::Local(p) => p.stop().await,
            Front::Service(s) => s.stop().await,
        }
    }
}

struct Running {
    /// Tells this connection from a later one (see `spawn_watchdog`).
    id: u64,
    front: Front,
    front_kind: CoreKind,
    sidecar: Option<CoreProcess>,
    /// Shoes, for the servers only it runs (NaiveProxy).
    shoes: Option<CoreProcess>,
    tasks: Vec<JoinHandle<()>>,
    api: Option<Mihomo>,
    nodes: Vec<Node>,
    metrics_port: u16,
    /// The group `GLOBAL` should point at in Global mode.
    main_group: String,
    tun_active: bool,
    warning: Option<String>,
}

impl Running {
    async fn shutdown(self) {
        for t in &self.tasks {
            t.abort();
        }
        self.front.stop().await;
        if let Some(s) = self.sidecar {
            s.stop().await;
        }
        if let Some(s) = self.shoes {
            s.stop().await;
        }
    }
}

pub struct Engine {
    pub paths: Paths,
    settings: RwLock<Settings>,
    profiles: RwLock<ProfileIndex>,
    run: Mutex<Option<Running>>,
    status: RwLock<Status>,
    delays: RwLock<HashMap<String, Option<u32>>>,
    /// Set when the main Mihomo reports that TUN failed to start.
    tun_error: RwLock<Option<String>>,
    /// Routing-data updates run one at a time (background refresh, Update now, source
    /// change); they write the same files.
    geodata_lock: Mutex<()>,
    /// Sites smart routing learned.
    smart: Arc<crate::smart::Smart>,
    /// Android: the VPN interface the system opened for Kova. Mihomo reads and writes
    /// it directly; it stays open across core restarts and is closed on disconnect.
    tun_fd: std::sync::Mutex<Option<i32>>,
    /// Checks in a row the chosen node failed; see [`Engine::check_node`].
    node_failures: std::sync::atomic::AtomicU32,
    /// The screen is off (Android): background checks run less often.
    quiet: std::sync::atomic::AtomicBool,
    /// Android: the phone is on mobile data rather than Wi-Fi (see `mobile_server`).
    on_mobile: std::sync::atomic::AtomicBool,
    /// A restart after the network came back is under way.
    network_restarting: std::sync::atomic::AtomicBool,
    /// When (unix seconds) the last routing-data update failed, 0 if none has: the
    /// background updater waits an hour before trying again (see `geodata_due`).
    geodata_failed: std::sync::atomic::AtomicI64,
    /// Numbers latency tests, so a newer one replaces an older one still running.
    delay_run: std::sync::atomic::AtomicU64,
    /// Android: the DNS servers of the network the phone is on (see `set_network_dns`).
    network_dns: std::sync::RwLock<Vec<String>>,
    /// The kill switch is holding: the connection dropped and the system proxy stays on
    /// until it is back or the user disconnects.
    holding: std::sync::atomic::AtomicBool,
    /// Counts disconnects (`stop`). A connect or reconnect in progress watches it and
    /// gives up when it changes, so Disconnect never waits for one to finish (see
    /// `start_with`).
    stops: watch::Sender<u64>,
    /// A launch asked the Kova service to start Mihomo; set before the request goes out,
    /// so a cancelled start knows to undo it.
    service_started: std::sync::atomic::AtomicBool,
    events: broadcast::Sender<Event>,
    me: Weak<Engine>,
}

fn next_run_id() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

fn free_port() -> Result<u16> {
    std::net::TcpListener::bind("127.0.0.1:0")
        .and_then(|l| l.local_addr())
        .map(|a| a.port())
        .map_err(|e| Error::Other(format!("Could not find a free local port ({e}). Security software may be blocking Kova")))
}

/// How old the routing data files are (the older of the two), `None` if one is missing.
fn geodata_age(dir: &Path, now: i64) -> Option<i64> {
    let mut oldest = 0;
    for f in crate::geodata::FILES {
        let modified = std::fs::metadata(dir.join(f)).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok()?;
        oldest = oldest.max(now - modified.as_secs() as i64);
    }
    Some(oldest)
}

/// Whether the routing data should be fetched. Without a marker (the files that came with
/// Xray, never updated by Kova) the default source is judged by the age of the files, so
/// it is refreshed too; any other source is wanted at once. Missing files are always due.
/// After a failure (`failed`, the time it happened) nothing is tried for an hour.
fn geodata_due_at(source: &kova_core::options::GeoSource, marker: Option<&crate::geodata::Marker>, age: Option<i64>, now: i64, failed: i64) -> bool {
    if failed != 0 && now - failed < 3600 {
        return false;
    }
    let max_age = crate::geodata::max_age(source);
    match marker {
        None => *source != kova_core::options::GeoSource::Default || age.is_none_or(|a| a > max_age),
        Some(m) => age.is_none() || m.source != source.id() || now - m.updated_at > max_age,
    }
}

/// Aborts a task when dropped, so a connect that is cancelled doesn't leave it running.
struct AbortOnDrop(Option<tokio::task::AbortHandle>);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        if let Some(h) = self.0.take() {
            h.abort();
        }
    }
}

/// The host of a provider address, lower case, when Kova may download from it. None for
/// a host that is only this computer or an unusable address (`localhost`, loopback,
/// link-local and unspecified addresses): a subscription must not make Kova probe those.
/// Addresses on a LAN stay allowed (a home server).
fn provider_host(url: &str) -> Option<String> {
    let host = reqwest::Url::parse(url).ok()?.host_str()?.trim_matches(['[', ']']).trim_end_matches('.').to_ascii_lowercase();
    if host.is_empty() || host == "localhost" || host.ends_with(".localhost") {
        return None;
    }
    if host.parse::<std::net::IpAddr>().is_ok_and(fetch::is_local_address) {
        return None;
    }
    Some(host)
}

/// What a refused provider's file holds: one server that can't connect. An empty list
/// would make Mihomo give a group that only uses the provider a built-in DIRECT, which
/// sends its traffic around the proxy.
const REFUSED_PROVIDER: &str = "proxies:\n  - {name: refused-provider, type: socks5, server: 127.0.0.1, port: 1}\n";

/// Whether Kova may download the provider at `url` (or follow a redirect to it). `sub_url`
/// is the address of the Remote subscription it belongs to; None (a local profile, the
/// user's own config) refuses nothing. A provider on the subscription's own host is
/// allowed even when that host is this computer (a local converter).
fn provider_allowed(url: &str, sub_url: Option<&str>) -> bool {
    let Some(sub) = sub_url else { return true };
    provider_host(url).is_some() || url_host(url).is_some_and(|h| url_host(sub) == Some(h))
}

/// The host of an address as `provider_host` writes it, without refusing any.
fn url_host(url: &str) -> Option<String> {
    Some(reqwest::Url::parse(url).ok()?.host_str()?.trim_matches(['[', ']']).trim_end_matches('.').to_ascii_lowercase())
}

impl Engine {
    /// A core rejects the whole config when a rule names a category its geodata lacks,
    /// with a cryptic message. Check first and say what to do.
    async fn check_geodata(&self, opts: &RuntimeOptions) -> Result<()> {
        let (site, ip) = opts.needed_categories();
        let dir = self.paths.cores_dir();
        let info = tokio::task::spawn_blocking(move || crate::geodata::info(&dir, &site, &ip)).await.expect("geodata check");
        if info.files.is_empty() || info.missing.is_empty() {
            return Ok(());
        }
        let hint = if info.missing.iter().any(|m| m.contains("ru-blocked")) {
            "The \"Only blocked sites\" preset needs the runetfreedom lists: choose them in Settings, Routing data"
        } else {
            "Update the routing data in Settings or turn off the preset that needs it"
        };
        Err(Error::Other(format!("The routing data has no {}. {hint}", info.missing.join(", "))))
    }
}

/// IP addresses (as /32 or /128) of the given servers; hostnames are looked up. Best
/// effort: a name that doesn't resolve now is skipped.
async fn resolve_servers<'a>(servers: impl Iterator<Item = (&'a str, u16)>) -> Vec<String> {
    // All at once: with Anti-DPI this is every server of the profile.
    let lookups = servers.map(|(host, port)| async move {
        match host.parse::<std::net::IpAddr>() {
            Ok(ip) => vec![ip],
            Err(_) => match tokio::time::timeout(Duration::from_secs(3), tokio::net::lookup_host((host, port))).await {
                Ok(Ok(it)) => it.map(|a| a.ip()).collect(),
                _ => Vec::new(),
            },
        }
    });
    let mut out = Vec::new();
    for ip in futures_util::future::join_all(lookups).await.into_iter().flatten() {
        let cidr = if ip.is_ipv4() { format!("{ip}/32") } else { format!("{ip}/128") };
        if !out.contains(&cidr) {
            out.push(cidr);
        }
    }
    out
}

/// The certificate SHA-256 of each node whose link turns certificate checks off, for
/// Xray to pin (it no longer accepts "allowInsecure"). With Anti-DPI on the greeting is
/// cut the way Anti-DPI cuts it. Best effort and all at once: a server that doesn't
/// answer gets no pin, and Xray then checks its certificate normally.
async fn cert_pins<'a>(nodes: impl Iterator<Item = &'a Node>, fragment: bool) -> HashMap<String, String> {
    let probes = nodes.filter(|n| n.tls.as_ref().is_some_and(|t| t.insecure && t.reality.is_none() && t.pin.is_none())).map(|n| async move {
        let name = n.tls.as_ref().map_or_else(|| n.server.clone(), |t| t.server_name(&n.server));
        let addr = tokio::net::lookup_host((n.server.as_str(), n.port)).await.ok()?.next()?;
        let pin = crate::tls_probe::leaf_sha256(addr, &name, fragment, Duration::from_secs(6)).await?;
        Some((n.name.clone(), pin))
    });
    futures_util::future::join_all(probes).await.into_iter().flatten().collect()
}

/// Fail early with a clear message instead of letting a core start without its listener.
fn port_available(addr: &str, port: u16) -> Result<()> {
    std::net::TcpListener::bind((addr, port)).map(drop).map_err(|e| {
        // Name the program only when the port really is taken; any other failure (an
        // excluded port range, say) is not that program's doing.
        let by = (e.kind() == std::io::ErrorKind::AddrInUse).then(|| port_owner(addr, port)).flatten();
        let by = by.unwrap_or_else(|| "another program".into());
        Error::Other(format!("Port {port} is already used by {by}. Close it or choose another port in Settings"))
    })
}

/// File name of the program listening on `addr:port` (never a full path), if it can be found.
#[cfg(windows)]
fn port_owner(addr: &str, port: u16) -> Option<String> {
    let pid = listener_pid(addr.parse().ok()?, port)?;
    let path = crate::programs::path_of(pid)?;
    Path::new(&path).file_name().map(|n| n.to_string_lossy().into_owned())
}

#[cfg(not(windows))]
fn port_owner(_addr: &str, _port: u16) -> Option<String> {
    None
}

/// Whether four table words hold the address `b` (they are its bytes in memory).
#[cfg(windows)]
fn same_v6(words: &[u32], b: std::net::Ipv6Addr) -> bool {
    let o = b.octets();
    (0..4).all(|i| words[i] == u32::from_ne_bytes([o[4 * i], o[4 * i + 1], o[4 * i + 2], o[4 * i + 3]]))
}

/// The process whose TCP listener conflicts with binding `bind:port`. None for the System
/// process (0, 4), when nothing conflicts, and when the table can't be read.
#[cfg(windows)]
fn listener_pid(bind: std::net::IpAddr, port: u16) -> Option<u32> {
    use std::net::IpAddr;
    use windows_sys::Win32::Foundation::ERROR_INSUFFICIENT_BUFFER;
    use windows_sys::Win32::NetworkManagement::IpHelper::{GetExtendedTcpTable, TCP_TABLE_OWNER_PID_LISTENER};
    use windows_sys::Win32::Networking::WinSock::{AF_INET, AF_INET6};
    // (family, words per row, word index of the local port, word index of the owning pid)
    for (family, row, port_at, pid_at) in [(AF_INET as u32, 6usize, 2usize, 5usize), (AF_INET6 as u32, 14, 5, 13)] {
        // u32 words keep the buffer aligned; the table is read word by word.
        let mut buf: Vec<u32> = Vec::new();
        let mut size = 0u32;
        let mut rc = ERROR_INSUFFICIENT_BUFFER;
        // The first call only asks for the size; the table can grow before the next one.
        for _ in 0..8 {
            let ptr = if buf.is_empty() { std::ptr::null_mut() } else { buf.as_mut_ptr().cast() };
            // SAFETY: `ptr` is null (size query) or points to at least `size` writable bytes.
            rc = unsafe { GetExtendedTcpTable(ptr, &mut size, 0, family, TCP_TABLE_OWNER_PID_LISTENER, 0) };
            if rc != ERROR_INSUFFICIENT_BUFFER {
                break;
            }
            buf = vec![0u32; (size as usize).div_ceil(4)];
        }
        if rc != 0 || buf.is_empty() {
            continue;
        }
        let rows = (buf[0] as usize).min((buf.len() - 1) / row);
        for r in buf[1..].chunks_exact(row).take(rows) {
            // The port is in network byte order in the low 16 bits.
            if u16::from_be(r[port_at] as u16) != port || matches!(r[pid_at], 0 | 4) {
                continue;
            }
            let conflicts = if family == AF_INET as u32 {
                // r[1] is the address as its bytes in memory.
                match bind {
                    IpAddr::V4(b) => b.is_unspecified() || r[1] == 0 || r[1] == u32::from_ne_bytes(b.octets()),
                    IpAddr::V6(_) => false,
                }
            } else {
                let unspecified = r[..4].iter().all(|w| *w == 0);
                match bind {
                    // Best effort: a listener on `::` may also take IPv4 (dual stack), and the
                    // table doesn't say whether it does, so it counts as a conflict.
                    IpAddr::V4(_) => unspecified,
                    IpAddr::V6(b) => b.is_unspecified() || unspecified || same_v6(&r[..4], b),
                }
            };
            if conflicts {
                return Some(r[pid_at]);
            }
        }
    }
    None
}

fn host_of(url: &str) -> String {
    url.split("://").nth(1).and_then(|r| r.split(['/', '?', ':']).next()).unwrap_or(url).to_string()
}

/// Split a core's stdout line into (level, message).
fn parse_log(core: &str, line: &str) -> (String, String) {
    if core == "mihomo" {
        // time="..." level=info msg="[TCP] ..."
        let level = line.split("level=").nth(1).and_then(|r| r.split_whitespace().next()).unwrap_or("info");
        let msg = line.split("msg=").nth(1).map(|m| m.trim().trim_matches('"').replace("\\\"", "\"")).unwrap_or_else(|| line.to_string());
        return (level.to_string(), msg);
    }
    // 2025/01/01 00:00:00.000000 [Warning] message
    if let Some(start) = line.find('[')
        && let Some(end) = line[start..].find(']') {
            let lvl = line[start + 1..start + end].to_ascii_lowercase();
            if ["debug", "info", "warning", "error"].contains(&lvl.as_str()) {
                return (lvl, line[start + end + 1..].trim().to_string());
            }
        }
    let msg = line.splitn(3, ' ').nth(2).filter(|_| line.as_bytes().first().is_some_and(u8::is_ascii_digit)).unwrap_or(line);
    ("info".into(), msg.trim().to_string())
}

impl Engine {
    pub fn new(root: impl Into<PathBuf>) -> Result<Arc<Self>> {
        let paths = Paths::new(root)?;
        match sysproxy::recover(&paths.proxy_marker()) {
            Ok(true) => tracing::info!("restored the system proxy left behind by a previous session"),
            Ok(false) => {}
            Err(e) => tracing::warn!("could not restore the system proxy: {e}"),
        }
        #[cfg_attr(not(target_os = "linux"), allow(unused_mut))]
        let mut bins: Vec<PathBuf> = [CoreKind::Mihomo, CoreKind::Xray].iter().map(|c| cores::binary_path(&paths.cores_dir(), *c)).collect();
        #[cfg(target_os = "linux")]
        bins.push(crate::tun_linux::mihomo_path());
        crate::process::end_leftovers(&paths.runtime_dir(), &bins);
        let settings = Settings::load(&paths);
        let profiles = ProfileIndex::load(&paths);
        let (events, _) = broadcast::channel(2048);
        let smart = crate::smart::Smart::load(paths.learned());
        Ok(Arc::new_cyclic(|me| Self {
            status: RwLock::new(Status { mixed_port: settings.core.mixed_port, ..Default::default() }),
            paths,
            settings: RwLock::new(settings),
            profiles: RwLock::new(profiles),
            run: Mutex::new(None),
            delays: RwLock::new(HashMap::new()),
            tun_error: RwLock::new(None),
            geodata_lock: Mutex::new(()),
            smart,
            tun_fd: std::sync::Mutex::new(None),
            node_failures: Default::default(),
            quiet: Default::default(),
            on_mobile: Default::default(),
            network_restarting: Default::default(),
            geodata_failed: Default::default(),
            delay_run: Default::default(),
            network_dns: Default::default(),
            holding: Default::default(),
            stops: watch::channel(0).0,
            service_started: Default::default(),
            events,
            me: me.clone(),
        }))
    }

    /// Android: hand over the VPN interface to run the core on (see `tun_fd`).
    pub fn set_tun_fd(&self, fd: Option<i32>) {
        *self.tun_fd.lock().unwrap() = fd;
    }

    pub fn has_tun_fd(&self) -> bool {
        self.tun_fd.lock().unwrap().is_some()
    }

    /// Android: give the VPN interface back, to be closed.
    pub fn take_tun_fd(&self) -> Option<i32> {
        self.tun_fd.lock().unwrap().take()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    fn emit(&self, e: Event) {
        let _ = self.events.send(e);
    }

    fn set_status(&self, f: impl FnOnce(&mut Status)) {
        let s = {
            let mut s = self.status.write().unwrap();
            f(&mut s);
            s.clone()
        };
        self.emit(Event::Status(s));
    }

    pub fn status(&self) -> Status {
        self.status.read().unwrap().clone()
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().unwrap().clone()
    }

    // ------------------------------------------------------------------ settings

    pub async fn update_settings(&self, new: Settings) -> Result<()> {
        // New rules only: one saved earlier must not block every other change.
        let before = self.settings().core.custom_rules;
        if let Some((r, why)) = new.core.custom_rules.iter().filter(|r| !before.contains(r)).find_map(|r| r.problem().map(|why| (r, why))) {
            return Err(Error::Other(format!("The rule \"{}\" can't be used: {why}", r.value.trim())));
        }
        if let Some(why) = new.core.test_url_problem() {
            return Err(Error::Other(format!("The test address can't be used: {why}")));
        }
        let old = {
            let mut s = self.settings.write().unwrap();
            let mut new = new;
            // These change only through their own engine calls (select, set_active_profile);
            // a settings save from the UI carries a possibly stale copy of them.
            new.core.secret = s.core.secret.clone();
            new.core.local_password = s.core.local_password.clone();
            new.device_id = s.device_id.clone();
            new.active_profile = s.active_profile.clone();
            new.selections = s.selections.clone();
            new.xray_selection = s.xray_selection.clone();
            std::mem::replace(&mut *s, new)
        };
        let new = self.settings();
        new.save(&self.paths)?;
        if old.core.geodata != new.core.geodata {
            self.refresh_geodata_files(false).await?;
        }
        // Turning the kill switch (or the system proxy) off lets go of a hold.
        if (!new.kill_switch || !new.system_proxy) && self.holding.swap(false, std::sync::atomic::Ordering::Relaxed) && !self.status().running {
            let _ = self.release_proxy();
            self.set_status(|s| s.error = None);
        }
        // A changed server for mobile data takes over at once while on mobile data.
        if old.mobile_server != new.mobile_server && self.on_mobile.load(std::sync::atomic::Ordering::Relaxed) {
            self.apply_network_choice().await;
        }
        if !self.status().running {
            self.set_status(|s| s.mixed_port = new.core.mixed_port);
            return Ok(());
        }
        let combined_changed = new.active_profile.as_deref() == Some(COMBINED) && old.combined_skip != new.combined_skip;
        let mut core_changed = old.core != new.core || combined_changed;
        if core_changed && !combined_changed && (RuntimeOptions { mode: new.core.mode, ..old.core.clone() }) == new.core {
            // Only the routing mode changed: Mihomo can switch live.
            let api = self.run.lock().await.as_ref().and_then(|r| r.api.clone());
            // The Android build of Mihomo refuses config changes over its API; then the
            // core restarts with the new mode like for any other change.
            if let Some(api) = api {
                core_changed = api.set_mode(new.core.mode).await.is_err();
                // Connections opened in the old mode keep their route; start them over.
                if !core_changed {
                    let _ = api.close_all().await;
                }
            }
        }
        if core_changed {
            self.restart().await?;
        } else if old.system_proxy != new.system_proxy || old.proxy_bypass != new.proxy_bypass {
            self.apply_system_proxy()?;
        }
        Ok(())
    }

    /// Whether another program took the system proxy away from Kova: connected with the
    /// system proxy on, yet it no longer points at Kova. Kova doesn't take it back by
    /// itself, which could fight another app; turning the system proxy off and on does.
    pub fn system_proxy_lost(&self) -> bool {
        if cfg!(target_os = "android") || self.holding.load(std::sync::atomic::Ordering::Relaxed) {
            return false;
        }
        let s = self.settings();
        s.system_proxy && self.status().running && !sysproxy::is_ours(&format!("127.0.0.1:{}", s.core.mixed_port))
    }

    fn apply_system_proxy(&self) -> Result<()> {
        // Android has no system proxy to set: the VPN captures everything.
        if cfg!(target_os = "android") {
            return Ok(());
        }
        let s = self.settings();
        let marker = self.paths.proxy_marker();
        if s.system_proxy && self.status().running {
            sysproxy::enable(&marker, &format!("127.0.0.1:{}", s.core.mixed_port), &s.proxy_bypass)
        } else {
            self.release_proxy()
        }
    }

    /// Give the system proxy back; see [`sysproxy::release`].
    fn release_proxy(&self) -> Result<()> {
        if cfg!(target_os = "android") {
            return Ok(());
        }
        // Tests make engines that never set the proxy: leave the developer's alone.
        #[cfg(test)]
        if !self.paths.proxy_marker().exists() {
            return Ok(());
        }
        sysproxy::release(&self.paths.proxy_marker(), &format!("127.0.0.1:{}", self.settings().core.mixed_port))
    }

    // ------------------------------------------------------------------ profiles

    pub fn profiles(&self) -> Vec<Profile> {
        self.profiles.read().unwrap().profiles.clone()
    }

    fn save_profiles(&self) -> Result<()> {
        self.profiles.read().unwrap().save(&self.paths)?;
        self.emit(Event::Profiles);
        Ok(())
    }

    fn fetch_proxy(&self, via_proxy: bool) -> Option<String> {
        if via_proxy { self.proxy_url() } else { None }
    }

    /// Download a subscription the profile's way and, when that can't reach the server
    /// while Kova is connected, the other way: through the proxy where the provider's
    /// site is blocked, directly where the provider refuses the server's address. A
    /// server that answers with an error is not asked again.
    async fn fetch_subscription(&self, url: &str, user_agent: Option<&str>, headers: &[crate::store::Header], via_proxy: bool) -> Result<fetch::Subscription> {
        let device = self.settings().device_id;
        self.fetch_subscription_as(url, user_agent, headers, via_proxy, &device, &fetch::Guard::OPEN).await
    }

    /// Like `fetch_subscription`, with the device id to send (empty: none), and `allow`
    /// held to `guard` (see [`fetch::subscription_guarded`]).
    async fn fetch_subscription_as(&self, url: &str, user_agent: Option<&str>, headers: &[crate::store::Header], via_proxy: bool, device: &str, guard: &fetch::Guard<'_>) -> Result<fetch::Subscription> {
        let device = device.to_string();
        let first = fetch::subscription_guarded(url, user_agent, headers, self.fetch_proxy(via_proxy).as_deref(), &device, guard).await;
        match first {
            Err(e @ Error::Net(_)) if self.status().running => {
                let other = if via_proxy { None } else { self.proxy_url() };
                match fetch::subscription_guarded(url, user_agent, headers, other.as_deref(), &device, guard).await {
                    Ok(sub) => {
                        let how = if via_proxy { "directly" } else { "through the proxy" };
                        self.emit(Event::Log { source: "kova".into(), level: "info".into(), message: format!("the subscription was downloaded {how}, the usual way failed: {e}") });
                        Ok(sub)
                    }
                    Err(_) => Err(e),
                }
            }
            other => other,
        }
    }

    /// The proxy port as a URL for Kova's own requests while connected, with Kova's
    /// login where the port asks for one (Android).
    pub fn proxy_url(&self) -> Option<String> {
        let st = self.status();
        if !st.running {
            return None;
        }
        let password = self.settings().core.local_password;
        Some(if self.has_tun_fd() && !password.is_empty() {
            format!("http://{}:{password}@127.0.0.1:{}", kova_core::options::LOCAL_USER, st.mixed_port)
        } else {
            format!("http://127.0.0.1:{}", st.mixed_port)
        })
    }

    /// Add a profile from a subscription URL, or from pasted share links / config text.
    pub async fn add_profile(&self, input: &str, name: Option<String>) -> Result<Profile> {
        let input = input.trim();
        let is_url = (input.starts_with("http://") || input.starts_with("https://")) && !input.contains(char::is_whitespace);
        let id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
        let (body, profile) = if is_url {
            let sub = self.fetch_subscription(input, None, &[], true).await?;
            let parsed = subscription::parse(&sub.body)?;
            let p = Profile {
                id: id.clone(),
                name: name.or(sub.title).unwrap_or_else(|| host_of(input)),
                source: Source::Remote { url: input.to_string(), user_agent: None, update_hours: sub.update_hours.unwrap_or(12), via_proxy: true, headers: Vec::new() },
                format: parsed.format,
                core: CorePreference::Auto,
                node_count: parsed.nodes.len(),
                usage: sub.usage,
                updated_at: now(),
                warnings: parsed.warnings,
                support_url: sub.support_url,
                announce: sub.announce,
            };
            (sub.body, p)
        } else {
            let parsed = subscription::parse(input)?;
            let default_name = if parsed.nodes.len() == 1 { parsed.nodes[0].name.clone() } else { format!("Imported · {} nodes", parsed.nodes.len()) };
            let p = Profile {
                id: id.clone(),
                name: name.unwrap_or(default_name),
                source: Source::Local,
                format: parsed.format,
                core: CorePreference::Auto,
                node_count: parsed.nodes.len(),
                usage: None,
                updated_at: now(),
                warnings: parsed.warnings,
                support_url: None,
                announce: None,
            };
            (input.to_string(), p)
        };
        self.paths.write_body(&id, &body)?;
        self.profiles.write().unwrap().profiles.push(profile.clone());
        let first = self.settings().active_profile.is_none();
        self.save_profiles()?;
        if first {
            self.set_active_profile(&id).await?;
        }
        Ok(profile)
    }

    pub async fn update_profile(&self, id: &str) -> Result<Profile> {
        let p = self.profiles.read().unwrap().get(id).cloned().ok_or_else(|| Error::Other("profile not found".into()))?;
        let Source::Remote { url, user_agent, via_proxy, headers, .. } = &p.source else {
            return Ok(p);
        };
        let sub = self.fetch_subscription(url, user_agent.as_deref(), headers, *via_proxy).await?;
        let parsed = subscription::parse(&sub.body)?;
        let updated = {
            let mut idx = self.profiles.write().unwrap();
            // The profile may have been deleted while the download was in flight.
            let e = idx.get_mut(id).ok_or_else(|| Error::Other("the profile was removed during the update".into()))?;
            self.paths.write_body(id, &sub.body)?;
            e.format = parsed.format;
            e.node_count = parsed.nodes.len();
            e.usage = sub.usage.or(e.usage.take());
            e.updated_at = now();
            e.warnings = parsed.warnings;
            e.support_url = sub.support_url.or(e.support_url.take());
            // The provider withdraws a message by no longer sending it.
            e.announce = sub.announce;
            e.clone()
        };
        self.save_profiles()?;
        self.reload_if_active(id).await?;
        Ok(updated)
    }

    /// Update every remote profile; returns `(profile name, error)` for failures.
    pub async fn update_all(&self) -> Vec<(String, String)> {
        let ids: Vec<(String, String)> = self.profiles().into_iter().filter(|p| matches!(p.source, Source::Remote { .. })).map(|p| (p.id, p.name)).collect();
        let mut failed = Vec::new();
        for (id, name) in ids {
            if let Err(e) = self.update_profile(&id).await {
                failed.push((name, e.to_string()));
            }
        }
        failed
    }

    /// Remote profiles whose auto-update interval has elapsed.
    pub fn profiles_due(&self) -> Vec<String> {
        let t = now();
        self.profiles()
            .into_iter()
            .filter(|p| matches!(p.source, Source::Remote { update_hours, .. } if update_hours > 0 && t - p.updated_at >= update_hours as i64 * 3600))
            .map(|p| p.id)
            .collect()
    }

    pub async fn remove_profile(&self, id: &str) -> Result<()> {
        {
            let mut idx = self.profiles.write().unwrap();
            idx.profiles.retain(|p| p.id != id);
        }
        self.paths.remove_body(id);
        self.save_profiles()?;
        if self.settings().active_profile.as_deref() == Some(id) {
            let next = self.profiles().first().map(|p| p.id.clone());
            let mut s = self.settings();
            s.active_profile = next.clone();
            *self.settings.write().unwrap() = s.clone();
            s.save(&self.paths)?;
            if self.status().running {
                if next.is_some() { self.restart().await? } else { self.stop().await? }
            }
        }
        Ok(())
    }

    pub async fn set_active_profile(&self, id: &str) -> Result<()> {
        if id != COMBINED && self.profiles.read().unwrap().get(id).is_none() {
            return Err(Error::Other("profile not found".into()));
        }
        let mut s = self.settings();
        s.active_profile = Some(id.to_string());
        s.xray_selection = None;
        *self.settings.write().unwrap() = s.clone();
        s.save(&self.paths)?;
        self.emit(Event::Profiles);
        if self.status().running {
            self.restart().await?;
        }
        Ok(())
    }

    pub async fn edit_profile(&self, id: &str, patch: ProfilePatch) -> Result<Profile> {
        let core_changed;
        let updated = {
            let mut idx = self.profiles.write().unwrap();
            let p = idx.get_mut(id).ok_or_else(|| Error::Other("profile not found".into()))?;
            if let Some(n) = patch.name.filter(|n| !n.trim().is_empty()) {
                p.name = n.trim().to_string();
            }
            core_changed = patch.core.is_some_and(|c| c != p.core);
            if let Some(c) = patch.core {
                p.core = c;
            }
            if let Some(why) = patch.headers.iter().flatten().find_map(|h| h.problem()) {
                return Err(Error::Other(format!("That header can't be sent: {why}")));
            }
            if let Source::Remote { url, user_agent, update_hours, via_proxy, headers } = &mut p.source {
                if let Some(h) = patch.headers {
                    *headers = h.into_iter().map(|h| crate::store::Header { name: h.name.trim().into(), value: h.value.trim().into() }).collect();
                }
                if let Some(u) = patch.url.filter(|u| !u.trim().is_empty()) {
                    *url = u.trim().to_string();
                }
                if let Some(ua) = patch.user_agent {
                    *user_agent = Some(ua).filter(|x| !x.trim().is_empty());
                }
                if let Some(h) = patch.update_hours {
                    *update_hours = h;
                }
                if let Some(v) = patch.via_proxy {
                    *via_proxy = v;
                }
            }
            p.clone()
        };
        self.save_profiles()?;
        if core_changed {
            self.reload_if_active(id).await?;
        }
        Ok(updated)
    }

    pub fn profile_content(&self, id: &str) -> Result<String> {
        Ok(self.paths.read_body(id)?)
    }

    /// Replace a profile's content after checking it parses (and, for Clash configs,
    /// that Mihomo accepts it).
    pub async fn save_profile_content(&self, id: &str, text: &str) -> Result<()> {
        let parsed = subscription::parse(text)?;
        if parsed.format == Format::Clash {
            self.validate_clash(text).await?;
        }
        if self.profiles.read().unwrap().get(id).is_none() {
            return Err(Error::Other("profile not found".into()));
        }
        self.paths.write_body(id, text)?;
        {
            let mut idx = self.profiles.write().unwrap();
            if let Some(p) = idx.get_mut(id) {
                p.format = parsed.format;
                p.node_count = parsed.nodes.len();
                p.warnings = parsed.warnings;
                p.updated_at = now();
            }
        }
        self.save_profiles()?;
        self.reload_if_active(id).await
    }

    /// Every profile with its content, and the settings, as backup file text; see
    /// [`crate::backup`].
    pub fn export_backup(&self, password: Option<&str>) -> Result<String> {
        let profiles = self
            .profiles()
            .into_iter()
            .map(|p| crate::backup::BackupProfile { body: self.paths.read_body(&p.id).unwrap_or_default(), profile: p })
            .collect();
        // The install's own secrets stay out of a file that gets passed around; an import
        // never takes them (see `import_backup`).
        let mut settings = self.settings();
        settings.core.secret.clear();
        settings.core.local_password.clear();
        settings.device_id.clear();
        let b = crate::backup::Backup { kova: env!("CARGO_PKG_VERSION").into(), created: now(), settings, profiles };
        crate::backup::write(&b, password)
    }

    /// Add a backup's profiles, skipping ones already here (the same subscription
    /// address, or the same content for a local profile), and with `with_settings` take
    /// its settings too, keeping what belongs to this install. Returns how many profiles
    /// were added and how many skipped.
    pub async fn import_backup(&self, b: crate::backup::Backup, with_settings: bool) -> Result<(usize, usize)> {
        let mut ids: HashMap<String, String> = HashMap::new(); // backup's id -> id here
        let (mut added, mut skipped) = (0, 0);
        for bp in b.profiles {
            let here = self.profiles().into_iter().find(|p| match (&p.source, &bp.profile.source) {
                (Source::Remote { url: a, .. }, Source::Remote { url: b, .. }) => a == b,
                (Source::Local, Source::Local) => self.paths.read_body(&p.id).is_ok_and(|body| body == bp.body),
                _ => false,
            });
            if let Some(p) = here {
                ids.insert(bp.profile.id, p.id);
                skipped += 1;
                continue;
            }
            let id = uuid::Uuid::new_v4().simple().to_string()[..12].to_string();
            self.paths.write_body(&id, &bp.body)?;
            let mut p = bp.profile;
            let old = std::mem::replace(&mut p.id, id.clone());
            ids.insert(old, id);
            self.profiles.write().unwrap().profiles.push(p);
            added += 1;
        }
        self.save_profiles()?;
        if with_settings {
            let here = self.settings();
            let mut s = b.settings;
            // Set per install, or tied to this device.
            s.device_id = here.device_id.clone();
            s.core.secret = here.core.secret.clone();
            s.core.local_password = here.core.local_password.clone();
            s.core.tun_fd = here.core.tun_fd;
            s.core.anti_dpi_port = here.core.anti_dpi_port;
            s.launch_at_login = here.launch_at_login;
            s.start_minimized = here.start_minimized;
            s.vpn_apps = here.vpn_apps.clone();
            let active = s.active_profile.take().and_then(|id| if id == COMBINED { Some(id) } else { ids.get(&id).cloned() });
            s.combined_skip = s.combined_skip.iter().filter_map(|id| ids.get(id).cloned()).collect();
            let selections: HashMap<String, String> = std::mem::take(&mut s.selections)
                .into_iter()
                .filter_map(|(key, member)| {
                    let (profile, group) = key.split_once('/')?;
                    Some((format!("{}/{group}", ids.get(profile)?), member))
                })
                .collect();
            let xray_selection = s.xray_selection.take();
            // update_settings keeps the choices of profile and servers; they follow here.
            self.update_settings(s).await?;
            if let Some(id) = active {
                self.set_active_profile(&id).await?;
            }
            let s = {
                let mut s = self.settings.write().unwrap();
                s.selections.extend(selections);
                s.xray_selection = xray_selection;
                s.clone()
            };
            s.save(&self.paths)?;
        }
        if self.settings().active_profile.is_none()
            && let Some(id) = self.profiles().first().map(|p| p.id.clone())
        {
            self.set_active_profile(&id).await?;
        }
        Ok((added, skipped))
    }

    /// Download the profile's http proxy providers here and point Mihomo at the files, so
    /// the servers don't depend on Mihomo reaching the provider (its download goes through
    /// the profile's own `proxy:`, which may not exist yet or may be unreachable from
    /// inside the VPN). A provider that can't be downloaded keeps the copy from the last
    /// run, and with none, stays as it was for Mihomo to try. `sub_url` is the address of
    /// the Remote subscription the config came from (None for a local profile, the user's
    /// own config, where nothing is refused). The device id goes only to a provider on the
    /// subscription's own host, and a provider that points to this computer (loopback and
    /// the like) is refused unless the subscription itself is on that host (a converter
    /// running here): its file holds one server that can't connect, so the groups that
    /// use it have no way out around the proxy and Mihomo doesn't fetch the address.
    ///
    /// Returns the config and the files it now reads, as (name in Mihomo's folder,
    /// content): a Mihomo that runs elsewhere (in the Kova service) needs them too.
    async fn localize_providers(&self, body: &str, home: &std::path::Path, via_proxy: bool, sub_url: Option<&str>) -> (String, Vec<(String, String)>) {
        let providers = gen_mihomo::http_providers(body);
        if providers.is_empty() {
            return (body.to_string(), Vec::new());
        }
        let ua = gen_mihomo::global_ua(body);
        let mut ready = Vec::new();
        let sub_host = sub_url.and_then(url_host);
        let mut refused = Vec::new();
        for (name, url) in providers {
            let file = home.join(gen_mihomo::provider_file(&name));
            if !provider_allowed(&url, sub_url) {
                self.refuse_provider(&name, &file, &mut ready, &mut refused);
                continue;
            }
            let device = if sub_host.is_some() && url_host(&url) == sub_host { self.settings().device_id } else { String::new() };
            // A local profile is the user's own config: no limits. Otherwise every address
            // is checked, and a name that leads to this computer is refused.
            let allow = |u: &str| provider_allowed(u, sub_url);
            let guard = fetch::Guard { allow: &allow, local_dns_except: sub_url.map(|_| sub_host.as_deref().unwrap_or_default()) };
            let fetched = self.fetch_subscription_as(&url, Some(ua.as_deref().unwrap_or("clash.meta")), &[], via_proxy, &device, &guard).await;
            match fetched {
                // Redirected or resolved to this computer: Mihomo must not fetch it either.
                Err(Error::Refused(_)) => {
                    self.refuse_provider(&name, &file, &mut ready, &mut refused);
                    continue;
                }
                Ok(sub) if gen_mihomo::provider_has_servers(&sub.body) => {
                    if let Some(dir) = file.parent() {
                        let _ = std::fs::create_dir_all(dir);
                    }
                    if std::fs::write(&file, sub.body).is_ok() {
                        ready.push(name);
                        continue;
                    }
                }
                Ok(_) => self.emit(Event::Log { source: "kova".into(), level: "warning".into(), message: format!("the provider {name} answered, but with no servers Kova could read") }),
                Err(e) => self.emit(Event::Log { source: "kova".into(), level: "warning".into(), message: format!("the provider {name} could not be downloaded: {e}") }),
            }
            // The copy from the last run is better than nothing.
            if std::fs::read_to_string(&file).is_ok_and(|t| gen_mihomo::provider_has_servers(&t)) {
                ready.push(name);
            }
        }
        // If this fails the config is not usable as it is either, and `merge_clash` says
        // so; a refused provider never reaches Mihomo with its address.
        match gen_mihomo::localize_providers_closed(body, &ready, &refused) {
            Ok(localized) => {
                let files = ready.iter().map(|name| gen_mihomo::provider_file(name)).filter_map(|f| std::fs::read_to_string(home.join(&f)).ok().map(|text| (f, text))).collect();
                (localized, files)
            }
            Err(_) => (body.to_string(), Vec::new()),
        }
    }

    /// A provider that may not be downloaded gets the fail-closed file and is read from it.
    fn refuse_provider(&self, name: &str, file: &Path, ready: &mut Vec<String>, refused: &mut Vec<String>) {
        self.emit(Event::Log { source: "kova".into(), level: "warning".into(), message: format!("the provider {name} was skipped: its address points to this computer") });
        tracing::warn!("provider {name} skipped: its address points to this computer");
        if let Some(dir) = file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if std::fs::write(file, REFUSED_PROVIDER).is_ok() {
            ready.push(name.to_string());
            refused.push(name.to_string());
        }
    }

    async fn validate_clash(&self, text: &str) -> Result<()> {
        let bin = cores::binary_path(&self.paths.cores_dir(), CoreKind::Mihomo);
        if !bin.exists() {
            return Ok(());
        }
        let dir = self.paths.runtime_dir().join("check");
        std::fs::create_dir_all(&dir)?;
        self.seed_geodata(&dir);
        let merged = gen_mihomo::merge_clash(text, &self.settings().core).map_err(|e| Error::Config(e.to_string()))?;
        let file = dir.join("config.yaml");
        std::fs::write(&file, merged)?;
        let (d, f) = (dir.to_string_lossy().to_string(), file.to_string_lossy().to_string());
        let checked = check_config(&bin, &["-t", "-d", &d, "-f", &f], &dir, &[]).await;
        let _ = std::fs::remove_file(&file);
        checked
    }

    async fn reload_if_active(&self, id: &str) -> Result<()> {
        let s = self.settings();
        let in_use = match s.active_profile.as_deref() {
            Some(COMBINED) => !s.combined_skip.iter().any(|x| x == id),
            active => active == Some(id),
        };
        if in_use && self.status().running {
            self.restart().await?;
        }
        Ok(())
    }

    pub fn profile_nodes(&self, id: &str) -> Result<Vec<NodeView>> {
        if id == COMBINED {
            return Ok(self.combined_nodes()?.iter().map(NodeView::of).collect());
        }
        let body = self.profile_content(id)?;
        Ok(subscription::parse(&body)?.nodes.iter().map(NodeView::of).collect())
    }

    /// The profiles in the combined list, in the order of the profile list.
    fn combined_members(&self) -> Vec<Profile> {
        let skip = self.settings().combined_skip;
        self.profiles().into_iter().filter(|p| !skip.contains(&p.id)).collect()
    }

    /// Every combined profile's servers in one list. Clash profiles give their servers
    /// only: their groups and rules belong to that profile alone. A name already taken by
    /// another profile's server gets its own profile's name added.
    fn combined_nodes(&self) -> Result<Vec<Node>> {
        let mut out: Vec<Node> = Vec::new();
        let mut taken = std::collections::HashSet::new();
        for p in self.combined_members() {
            let Ok(body) = self.profile_content(&p.id) else { continue };
            let Ok(parsed) = subscription::parse(&body) else { continue };
            for mut n in parsed.nodes {
                if taken.contains(&n.name) {
                    let base = format!("{} · {}", n.name, p.name);
                    let mut name = base.clone();
                    let mut i = 2;
                    while taken.contains(&name) {
                        name = format!("{base} {i}");
                        i += 1;
                    }
                    n.name = name;
                }
                taken.insert(n.name.clone());
                out.push(n);
            }
        }
        if out.is_empty() {
            return Err(Error::Other("The combined profiles have no servers. Add profiles to the list or choose one profile".into()));
        }
        Ok(out)
    }

    /// The combined list as a profile: share-link style, so Kova makes its groups.
    fn combined_profile(&self, nodes: usize) -> Profile {
        Profile {
            id: COMBINED.into(),
            name: "All profiles".into(),
            source: crate::store::Source::Local,
            format: Format::Links,
            core: CorePreference::Auto,
            node_count: nodes,
            usage: None,
            updated_at: 0,
            warnings: Vec::new(),
            support_url: None,
            announce: None,
        }
    }

    fn active(&self) -> Result<(Profile, String, Vec<Node>)> {
        let id = self.settings().active_profile.ok_or_else(|| Error::Other("Add a profile first".into()))?;
        let (p, body, mut nodes) = if id == COMBINED {
            let nodes = self.combined_nodes()?;
            (self.combined_profile(nodes.len()), String::new(), nodes)
        } else {
            let p = self.profiles.read().unwrap().get(&id).cloned().ok_or_else(|| Error::Other("The active profile was removed. Choose another one".into()))?;
            let body = self.profile_content(&id)?;
            let nodes = subscription::parse(&body)?.nodes;
            (p, body, nodes)
        };
        // Servers the user hid (by country or protocol) are left out, unless that would
        // leave none.
        let opts = self.settings().core;
        if nodes.iter().any(|n| !opts.hides_node(n)) {
            nodes.retain(|n| !opts.hides_node(n));
        }
        Ok((p, body, nodes))
    }

    // ------------------------------------------------------------------ running

    pub async fn start(&self) -> Result<()> {
        self.start_at(self.stop_epoch()).await
    }

    /// The number of disconnects so far; read it before an action that a disconnect
    /// must cancel, and hand it to `start_at` / `restart_at`.
    pub fn stop_epoch(&self) -> u64 {
        *self.stops.borrow()
    }

    /// The id of the connection that is up, if any.
    async fn run_id(&self) -> Option<u64> {
        self.run.lock().await.as_ref().map(|r| r.id)
    }

    /// Connect, unless `stop` ran since `epoch` was read: then the user disconnected and
    /// nothing is started, which is not an error.
    pub async fn start_at(&self, epoch: u64) -> Result<()> {
        self.start_with(epoch, |opts| self.launch(Purpose::Main, opts)).await
    }

    /// The body of `start_at`, with the launch passed in. `stop` raises the epoch before
    /// it waits for the `run` lock this holds, and the launch is raced against that: when
    /// it changes, the launch future is dropped (the cores it started are killed on drop,
    /// the Kova service is asked to stop what it may have started), the lock is let go and
    /// the status goes back to disconnected without an error. An uncancelled start runs as
    /// it always did.
    async fn start_with<F, Fut>(&self, epoch: u64, launch: F) -> Result<()>
    where
        F: FnOnce(RuntimeOptions) -> Fut,
        Fut: std::future::Future<Output = Result<(Running, Plan)>>,
    {
        let mut run = self.run.lock().await;
        let mut cancel = self.stops.subscribe();
        if run.is_some() || *cancel.borrow_and_update() != epoch {
            return Ok(());
        }
        self.set_status(|s| {
            s.busy = true;
            s.error = None;
        });
        self.service_started.store(false, std::sync::atomic::Ordering::Relaxed);
        let opts = self.settings().core;
        let launched = tokio::select! {
            r = launch(opts.clone()) => Some(r),
            _ = async { let _ = cancel.wait_for(|e| *e != epoch).await; } => None,
        };
        let Some(launched) = launched else {
            if self.service_started.swap(false, std::sync::atomic::Ordering::Relaxed) {
                // The lock is held here and a disconnect waits for it: a service that
                // doesn't answer must not keep it for long.
                let undo = async {
                    crate::service::stop_remote().await;
                    // A service that doesn't know about cancelled starts may finish the
                    // Start anyway: look again for a moment, and stop what turns up.
                    for _ in 0..3 {
                        tokio::time::sleep(Duration::from_millis(700)).await;
                        if crate::service::call(&crate::service::Request::Status, Duration::from_secs(2)).await.is_ok_and(|r| r.running) {
                            crate::service::stop_remote().await;
                        }
                    }
                };
                if tokio::time::timeout(Duration::from_secs(5), undo).await.is_err() {
                    tracing::warn!("the Kova service did not answer in time while a connect was cancelled");
                }
            }
            // Cores dropped with the launch are still quitting (Unix).
            crate::process::finish_dropped().await;
            self.set_status(|s| {
                s.busy = false;
                s.running = false;
                s.core = None;
                s.since = None;
                s.tun_active = false;
                s.error = None;
            });
            return Ok(());
        };
        match launched {
            Ok((r, plan)) => {
                self.holding.store(false, std::sync::atomic::Ordering::Relaxed);
                let kind = r.front_kind;
                let run_id = r.id;
                let sidecar = r.sidecar.is_some();
                let api = r.api.clone();
                let main_group = r.main_group.clone();
                // TUN can fail after launch decided to use it (another VPN holds the
                // adapter address, a driver problem); Mihomo then keeps running without it.
                let tun_error = self.tun_error.read().unwrap().clone();
                let tun_active = r.tun_active && tun_error.is_none();
                let warning = tun_error.or_else(|| r.warning.clone());
                *run = Some(r);
                // Under the lock, so a disconnect can't finish before this and be undone.
                self.set_status(|s| {
                    s.running = true;
                    s.busy = false;
                    s.core = Some(kind);
                    s.sidecar = sidecar;
                    s.since = Some(now());
                    s.mixed_port = opts.mixed_port;
                    s.bridged = plan.bridged.clone();
                    s.dropped = plan.dropped.clone();
                    s.tun_active = tun_active;
                    s.warning = warning;
                });
                drop(run);
                // A disconnect from here on has taken the connection down (or will): leave
                // it alone, and don't set the system proxy for it.
                if let Some(api) = api.filter(|_| self.stop_epoch() == epoch) {
                    // Mihomo's GLOBAL group starts on DIRECT, so Global mode would bypass the
                    // proxy. Point it at the main group; a choice the user made in GLOBAL is
                    // restored right after and wins.
                    let _ = api.select("GLOBAL", &main_group).await;
                    self.restore_selections(&api).await;
                    // Connected on mobile data: its own server, over the saved choice.
                    if self.on_mobile.load(std::sync::atomic::Ordering::Relaxed)
                        && let Some(server) = self.settings().mobile_server {
                            let _ = api.select(&main_group, &server).await;
                        }
                }
                {
                    // The lock again: a disconnect that comes after this check waits for the
                    // proxy to be set, then releases it.
                    let run = self.run.lock().await;
                    if self.stop_epoch() != epoch || run.as_ref().map(|r| r.id) != Some(run_id) {
                        return Ok(());
                    }
                    if let Err(e) = self.apply_system_proxy() {
                        self.emit(Event::Log { source: "kova".into(), level: "warning".into(), message: format!("could not set the system proxy: {e}") });
                    }
                }
                if !self.has_geodata() {
                    self.fetch_geodata_through_proxy(epoch, run_id);
                }
                Ok(())
            }
            Err(e) => {
                drop(run);
                // A failed restart must not leave the OS proxy pointing at a dead port,
                // unless the kill switch holds it there on purpose.
                if !self.holding.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = self.release_proxy();
                }
                self.set_status(|s| {
                    s.busy = false;
                    s.running = false;
                    s.core = None;
                    s.since = None;
                    s.tun_active = false;
                    s.error = Some(e.to_string());
                });
                Err(e)
            }
        }
    }

    /// Make a connect in progress give up, without disconnecting. For a caller that must
    /// wait its turn before it can call `stop` (Android serializes connect and disconnect).
    pub fn cancel_start(&self) {
        self.stops.send_modify(|e| *e = e.wrapping_add(1));
    }

    pub async fn stop(&self) -> Result<()> {
        // Disconnecting lifts the kill switch.
        self.holding.store(false, std::sync::atomic::Ordering::Relaxed);
        // Before waiting for the lock: a connect that holds it gives up on seeing this.
        self.cancel_start();
        // The lock stays held through the shutdown, so a connect can't overlap it.
        let mut run = self.run.lock().await;
        let taken = run.take();
        self.set_status(|s| s.busy = true);
        let proxy = self.release_proxy();
        if let Some(r) = taken {
            r.shutdown().await;
        }
        self.remove_run_configs();
        self.set_status(|s| {
            s.running = false;
            s.busy = false;
            s.core = None;
            s.sidecar = false;
            s.since = None;
            s.tun_active = false;
            s.warning = None;
        });
        drop(run);
        self.emit(Event::Traffic { up: 0, down: 0, up_total: 0, down_total: 0 });
        proxy
    }

    /// The configs written for the cores hold the servers' keys in plain text, which the
    /// cores need while they run; take them away once they stop. Kept: the cores' own
    /// caches next to them, including the server lists Mihomo downloads for a profile's
    /// proxy providers. Mihomo starts from those when the provider can't be reached, so
    /// removing them could leave such a profile with no servers; like the rest of the
    /// data folder they are readable by this user only.
    fn remove_run_configs(&self) {
        for sub in ["main", "tester", "check"] {
            let dir = self.paths.runtime_dir().join(sub);
            for f in [dir.join("xray.json"), dir.join("xray-sidecar.json"), dir.join("shoes.yaml"), dir.join("config.yaml"), dir.join("mihomo").join("config.yaml")] {
                let _ = std::fs::remove_file(f);
            }
        }
    }

    /// Re-apply config. The system proxy stays pointed at us across the short gap.
    pub async fn restart(&self) -> Result<()> {
        self.restart_at(self.stop_epoch()).await
    }

    /// `restart`, unless `stop` ran since `epoch` was read: a disconnect during the gap
    /// between the shutdown and the new start is not undone.
    async fn restart_at(&self, epoch: u64) -> Result<()> {
        let taken = self.run.lock().await.take();
        // Nothing was running (a disconnect got there first): nothing to start over.
        let Some(r) = taken else { return Ok(()) };
        r.shutdown().await;
        self.set_status(|s| s.running = false);
        self.start_at(epoch).await
    }

    /// Give a Mihomo home the installed geodata, so a start doesn't depend on Mihomo
    /// downloading it from GitHub (slow or blocked in some countries).
    /// Whether both routing data files are installed.
    fn has_geodata(&self) -> bool {
        let dir = self.paths.cores_dir();
        ["geoip.dat", "geosite.dat"].iter().all(|f| dir.join(f).exists())
    }

    /// Connected without the routing data: download it through the connection, which
    /// works where GitHub itself is blocked, then reconnect with every rule. Tries a few
    /// times with growing pauses while this same connection (`run_id`, started at stop
    /// count `epoch`) stays up; a disconnect or a new connection ends it.
    fn fetch_geodata_through_proxy(&self, epoch: u64, run_id: u64) {
        let me = self.me.clone();
        tokio::spawn(async move {
            for attempt in 1..=5u64 {
                let Some(e) = me.upgrade() else { return };
                if e.stop_epoch() != epoch || e.run_id().await != Some(run_id) {
                    return;
                }
                match e.refresh_geodata_files(false).await {
                    Ok(_) => {
                        // The download can take minutes: look again before reconnecting.
                        if e.stop_epoch() == epoch && e.run_id().await == Some(run_id) {
                            let _ = e.restart_at(epoch).await;
                            e.emit(Event::Notice { text: "Routing data downloaded through the server; every rule works now".into() });
                        }
                        return;
                    }
                    Err(err) => tracing::warn!("routing data through the proxy, attempt {attempt}: {err}"),
                }
                drop(e);
                tokio::time::sleep(Duration::from_secs(20 * attempt)).await;
            }
        });
    }

    fn seed_geodata(&self, home: &Path) {
        crate::geodata::share_cached_copies(&self.paths.cores_dir());
        crate::geodata::drop_stale_caches(&self.paths.cores_dir());
        crate::geodata::seed_mihomo_home(&self.paths.cores_dir(), home);
    }

    pub fn geodata_info(&self) -> crate::geodata::GeoInfo {
        let (site, ip) = self.settings().core.needed_categories();
        crate::geodata::info(&self.paths.cores_dir(), &site, &ip)
    }

    /// Make the chosen source's routing data active and hand it to every Mihomo home.
    /// Without `force`, a current cached copy is reused instead of downloading again.
    /// Running cores keep their loaded copy until they restart.
    pub async fn refresh_geodata_files(&self, force: bool) -> Result<crate::geodata::Marker> {
        let done = self.refresh_geodata_inner(force).await;
        self.geodata_failed.store(if done.is_err() { now() } else { 0 }, std::sync::atomic::Ordering::Relaxed);
        done
    }

    async fn refresh_geodata_inner(&self, force: bool) -> Result<crate::geodata::Marker> {
        let _one_at_a_time = self.geodata_lock.lock().await;
        let source = self.settings().core.geodata;
        let (dir, proxy) = (self.paths.cores_dir(), self.fetch_proxy(true));
        let m = if force {
            crate::geodata::update(&dir, &source, proxy.as_deref()).await?
        } else {
            crate::geodata::switch_to(&dir, &source, proxy.as_deref()).await?.0
        };
        let rt = self.paths.runtime_dir();
        for home in [rt.join("main").join("mihomo"), rt.join("tester").join("mihomo"), rt.join("check")] {
            if home.exists() {
                self.seed_geodata(&home);
            }
        }
        Ok(m)
    }

    /// Update the routing data now and reload the connection so it takes effect.
    pub async fn update_geodata(&self) -> Result<crate::geodata::Marker> {
        let m = self.refresh_geodata_files(true).await?;
        if self.status().running {
            self.restart().await?;
        }
        Ok(m)
    }

    /// Whether the background updater should fetch routing data now.
    pub fn geodata_due(&self) -> bool {
        let source = self.settings().core.geodata;
        let dir = self.paths.cores_dir();
        let failed = self.geodata_failed.load(std::sync::atomic::Ordering::Relaxed);
        geodata_due_at(&source, crate::geodata::marker(&dir).as_ref(), geodata_age(&dir, now()), now(), failed)
    }

    fn note_tun_failure(&self, message: &str) {
        let detail = message.split("error:").nth(1).unwrap_or(message).trim();
        let text = format!("TUN couldn't start ({detail}). Another VPN or proxy app may already be using TUN");
        *self.tun_error.write().unwrap() = Some(text.clone());
        if self.status().running {
            self.set_status(|s| {
                s.tun_active = false;
                s.warning = Some(text);
            });
        }
    }

    fn log_sink(&self, quiet: bool) -> LineSink {
        let me = self.me.clone();
        Arc::new(move |core: &str, line: &str| {
            if quiet {
                return;
            }
            if let Some(e) = me.upgrade() {
                let (level, message) = parse_log(core, line);
                if core == "mihomo" && message.contains("TUN listening error") {
                    e.note_tun_failure(&message);
                }
                e.emit(Event::Log { source: core.to_string(), level, message });
            }
        })
    }

    async fn launch(&self, purpose: Purpose, mut opts: RuntimeOptions) -> Result<(Running, Plan)> {
        let (profile, body, nodes) = self.active()?;
        let cores_dir = self.paths.cores_dir();
        let sub = if purpose == Purpose::Tester { "tester" } else { "main" };
        let run_dir = self.paths.runtime_dir().join(sub);
        std::fs::create_dir_all(&run_dir)?;
        let quiet = purpose == Purpose::Tester;
        // Internal ports are picked fresh each run so they can't collide with other clients
        // (Clash Verge also defaults its controller to 9097) or with a leftover core.
        opts.controller_port = free_port()?;
        opts.xray_metrics_port = free_port()?;
        let mut warning = None;
        if quiet {
            // Only latency tests use it: servers without rules or routing data start fastest.
            opts.mixed_port = free_port()?;
            opts.tun.enabled = false;
            opts.mode = Mode::Global;
            opts.without_geodata = true;
            opts.presets.clear();
            opts.custom_rules.clear();
            opts.apply_rules_to_clash_profiles = false;
            opts.smart_routing = false;
        } else {
            port_available(opts.listen_addr(), opts.mixed_port)?;
            *self.tun_error.write().unwrap() = None;
            // No routing data yet (a first start). Where GitHub can't be reached, waiting
            // for it would mean never connecting: connect without it instead, and fetch it
            // through the connection (see `start`).
            if !self.has_geodata() {
                let direct = tokio::time::timeout(Duration::from_secs(20), self.refresh_geodata_files(false)).await;
                if !matches!(direct, Ok(Ok(_))) {
                    opts.without_geodata = true;
                    warning = Some("Connected without the routing data, which couldn't be downloaded directly; getting it through the server".to_string());
                }
            }
            if !opts.without_geodata {
                self.check_geodata(&opts).await?;
            }
        }

        // On Android the VPN is Mihomo's TUN, which Xray can't provide: a profile set to
        // Xray only runs as Automatic there (Mihomo, with Xray bridging what needs it).
        let core = if cfg!(target_os = "android") && profile.core == CorePreference::Xray { CorePreference::Auto } else { profile.core };
        let mut plan = if profile.format == Format::Clash && core != CorePreference::Xray {
            Plan { front: CoreKind::Mihomo, bridged: Vec::new(), dropped: Vec::new(), shoes: Vec::new() }
        } else {
            compat::plan(&nodes, core)
        };
        // Testing always runs through Mihomo, which can measure every node it runs.
        if quiet && plan.front == CoreKind::Xray {
            plan = compat::plan(&nodes, CorePreference::Auto);
        }
        let xray_bin = cores::binary_path(&cores_dir, CoreKind::Xray);
        // An Xray from before Finalmask (kept from an older Kova) gets Anti-DPI the old way.
        if opts.anti_dpi && xray_bin.exists() {
            let dir = cores_dir.clone();
            let version = tokio::task::spawn_blocking(move || cores::installed_version(&dir, CoreKind::Xray)).await.ok().flatten();
            opts.xray_legacy_masks = gen_xray::legacy_masks(version.as_deref());
        }
        #[allow(unused_mut)]
        let mut mihomo_bin = cores::binary_path(&cores_dir, CoreKind::Mihomo);
        // Mihomo is the Linux copy with network rights: it needs the same care as one
        // running with root rights.
        #[allow(unused_mut)]
        let mut has_net_rights = false;
        let asset_env: [(&str, &Path); 1] = [("XRAY_LOCATION_ASSET", cores_dir.as_path())];

        if plan.front == CoreKind::Xray {
            if opts.tun.enabled {
                opts.tun.enabled = false;
                warning = Some("TUN mode needs the Mihomo core; this profile runs on Xray only".to_string());
            }
            if !quiet {
                let socks = opts.mixed_port.checked_add(1).ok_or_else(|| Error::Other("Xray profiles also use the port above the proxy port; choose a proxy port below 65535".into()))?;
                port_available("127.0.0.1", socks)?;
                if opts.allow_lan {
                    let (http_lan, socks_lan) = gen_xray::lan_ports(&opts).ok_or_else(|| Error::Other("Sharing an Xray profile with the network uses the three ports above the proxy port; choose a lower proxy port".into()))?;
                    port_available("0.0.0.0", http_lan)?;
                    port_available("0.0.0.0", socks_lan)?;
                }
            }
            let selection = match self.settings().xray_selection {
                Some(n) => Selection::Node(n),
                None => Selection::Auto,
            };
            opts.cert_pins = cert_pins(nodes.iter(), opts.anti_dpi).await;
            let cfg = run_dir.join("xray.json");
            std::fs::write(&cfg, gen_xray::full(&nodes, &selection, &opts))?;
            let c = cfg.to_string_lossy().to_string();
            check_config(&xray_bin, &["run", "-test", "-c", &c], &run_dir, &asset_env).await?;
            let mut front = CoreProcess::spawn("xray", &xray_bin, &["run", "-c", &c], &run_dir, &asset_env, self.log_sink(quiet))?;
            tokio::time::sleep(Duration::from_millis(400)).await;
            if let Some(msg) = front.exited() {
                return Err(Error::Core(msg));
            }
            let mut r = Running {
                id: next_run_id(),
                front: Front::Local(front),
                front_kind: CoreKind::Xray,
                sidecar: None,
                shoes: None,
                tasks: Vec::new(),
                api: None,
                nodes,
                metrics_port: opts.xray_metrics_port,
                main_group: PROXY.into(),
                tun_active: false,
                warning,
            };
            if !quiet {
                r.tasks.push(self.spawn_xray_metrics(opts.xray_metrics_port));
                r.tasks.push(self.spawn_watchdog());
            }
            return Ok((r, plan));
        }

        // Mihomo in front, with an Xray sidecar for nodes only Xray can run.
        // TUN needs elevated rights. Without them, Mihomo runs inside the Kova service.
        // Only Windows switches to the service on its own; elsewhere this stays as set here.
        #[cfg_attr(not(windows), allow(unused_mut))]
        let mut via_service = !quiet && std::env::var_os("KOVA_FORCE_SERVICE").is_some();
        // Android: TUN is the VPN interface the system gave Kova, which needs no rights.
        let tun_fd = if quiet { None } else { *self.tun_fd.lock().unwrap() };
        if cfg!(target_os = "android") {
            opts.tun.enabled = tun_fd.is_some();
        }
        if let Some(fd) = tun_fd {
            opts.tun.enabled = true;
            opts.tun_fd = Some(fd);
            for d in self.network_dns.read().unwrap().iter() {
                let d = if d.contains(':') { format!("[{d}]") } else { d.clone() };
                if !opts.dns.direct_nameservers.contains(&d) {
                    opts.dns.direct_nameservers.push(d);
                }
            }
        } else if opts.tun.enabled && !crate::process::is_elevated() {
            #[cfg(target_os = "linux")]
            match crate::tun_linux::granted() {
                // The copy of Mihomo with network rights runs TUN.
                Some(granted) if !quiet => {
                    mihomo_bin = granted;
                    has_net_rights = true;
                }
                _ => {
                    opts.tun.enabled = false;
                    warning = Some("TUN mode needs network rights once. Allow them in Settings, TUN mode".to_string());
                }
            }
            #[cfg(target_os = "macos")]
            {
                opts.tun.enabled = false;
                warning = Some("TUN mode on macOS needs Kova to run as administrator for now".to_string());
            }
            #[cfg(windows)]
            if !quiet && crate::service::hello().await.is_some() {
                via_service = true;
            } else {
                opts.tun.enabled = false;
                warning = Some("TUN mode needs the Kova service. Install it in Settings, TUN mode".to_string());
            }
        }
        let main_group = if profile.format == Format::Clash { gen_mihomo::main_group_of(&body) } else { PROXY.to_string() };
        let mut bridges = Bridges::new();
        let mut sidecar = None;
        // Anti-DPI (beta) runs in the same Xray, which then starts even with nothing to bridge.
        // Latency tests while disconnected (`quiet`) use it too, so they match a connection.
        let anti_dpi = opts.anti_dpi;
        if anti_dpi && !xray_bin.exists() {
            warning.get_or_insert_with(|| "Anti-DPI needs the Xray core. Install it in Settings, Cores".to_string());
        }
        if plan.needs_sidecar() || (anti_dpi && xray_bin.exists()) {
            if !xray_bin.exists() {
                for b in std::mem::take(&mut plan.bridged) {
                    plan.dropped.push((b, "needs Xray, which is not installed".into()));
                }
            } else {
                let bridged: Vec<(&Node, u16)> = nodes.iter().filter(|n| plan.bridged.contains(&n.name)).map(|n| Ok((n, free_port()?))).collect::<Result<_>>()?;
                for (n, port) in &bridged {
                    bridges.insert(n.name.clone(), *port);
                }
                if anti_dpi {
                    opts.anti_dpi_port = Some(free_port()?);
                }
                // In TUN mode Xray's own connections to the servers would be captured and
                // routed back into it, and TUN would also undo Anti-DPI's small pieces.
                // Exempt them by process (rule mode) and by destination address (all modes,
                // rules or not). With Anti-DPI Xray connects to every server.
                opts.bypass_processes = vec![cores::binary_name(CoreKind::Xray).to_string()];
                // On Android Kova's own traffic, Xray's included, is outside the VPN already.
                if opts.tun.enabled && opts.tun_fd.is_none() {
                    opts.tun_exclude = if anti_dpi {
                        resolve_servers(nodes.iter().map(|n| (n.server.as_str(), n.port))).await
                    } else {
                        resolve_servers(bridged.iter().map(|(n, _)| (n.server.as_str(), n.port))).await
                    };
                }
                opts.cert_pins = cert_pins(bridged.iter().map(|(n, _)| *n), anti_dpi).await;
                let cfg = run_dir.join("xray-sidecar.json");
                std::fs::write(&cfg, gen_xray::sidecar(&bridged, &opts))?;
                let c = cfg.to_string_lossy().to_string();
                let mut proc = CoreProcess::spawn("xray", &xray_bin, &["run", "-c", &c], &run_dir, &asset_env, self.log_sink(quiet))?;
                tokio::time::sleep(Duration::from_millis(250)).await;
                if let Some(msg) = proc.exited() {
                    return Err(Error::Core(msg));
                }
                sidecar = Some(proc);
            }
        }

        // Shoes runs what neither Mihomo nor Xray can (NaiveProxy), each server on a local
        // SOCKS port like the Xray bridge. It is fetched the first time a profile needs it.
        let mut shoes = None;
        if !plan.shoes.is_empty() {
            let shoes_bin = cores::binary_path(&cores_dir, CoreKind::Shoes);
            if !shoes_bin.exists() && !quiet && cores::available(CoreKind::Shoes)
                && let Err(e) = cores::install_pinned(&cores_dir, CoreKind::Shoes).await {
                    tracing::warn!("could not install Shoes: {e}");
                }
            if !shoes_bin.exists() {
                let why = if cores::available(CoreKind::Shoes) { "needs the Shoes core, which could not be installed" } else { "needs the Shoes core, which has no build for this system yet" };
                for n in std::mem::take(&mut plan.shoes) {
                    plan.dropped.push((n, why.into()));
                }
            } else {
                let served: Vec<(&Node, u16)> = nodes.iter().filter(|n| plan.shoes.contains(&n.name)).map(|n| Ok((n, free_port()?))).collect::<Result<_>>()?;
                for (n, port) in &served {
                    bridges.insert(n.name.clone(), *port);
                }
                // In TUN mode Shoes' own connections to the servers must not loop back in.
                opts.bypass_processes.push(cores::binary_name(CoreKind::Shoes).to_string());
                if opts.tun.enabled && opts.tun_fd.is_none() {
                    opts.tun_exclude.extend(resolve_servers(served.iter().map(|(n, _)| (n.server.as_str(), n.port))).await);
                }
                let cfg = run_dir.join("shoes.yaml");
                std::fs::write(&cfg, kova_core::gen_shoes::config(&served, &opts))?;
                let c = cfg.to_string_lossy().to_string();
                let mut proc = CoreProcess::spawn("shoes", &shoes_bin, &["--no-reload", &c], &run_dir, &[], self.log_sink(quiet))?;
                tokio::time::sleep(Duration::from_millis(250)).await;
                if let Some(msg) = proc.exited() {
                    return Err(Error::Core(msg));
                }
                shoes = Some(proc);
            }
        }

        // Smart routing needs Kova's rules in the config; an imported Clash profile has
        // them only when the user applies them.
        if !quiet && opts.smart_routing && (profile.format != Format::Clash || opts.apply_rules_to_clash_profiles) {
            let lists_url = self.smart.lists_url().await?;
            // A subscription's own rules exist only in an imported Clash profile.
            let profile_rules = opts.smart_profile_rules && profile.format == Format::Clash;
            let scope = gen_mihomo::smart_scope(&opts, profile_rules);
            let password = opts.local_password.clone();
            opts.smart = Some(SmartRuntime { lists_url, direct_port: free_port()?, proxy_port: free_port()?, password, scope });
        }
        let home = run_dir.join("mihomo");
        std::fs::create_dir_all(&home)?;
        self.seed_geodata(&home);
        if let Some(root) = self.paths.cores_dir().parent().filter(|_| !quiet) {
            for message in crate::geodata::usage_report(root) {
                self.emit(Event::Log { source: "kova".into(), level: "info".into(), message });
            }
        }
        let mut provider_files = Vec::new();
        let yaml = if profile.format == Format::Clash {
            let (sub_url, via_proxy) = match &profile.source {
                Source::Remote { url, via_proxy, .. } => (Some(url.as_str()), *via_proxy),
                _ => (None, false),
            };
            let (body, files) = self.localize_providers(&body, &home, via_proxy, sub_url).await;
            provider_files = files;
            gen_mihomo::merge_clash(&body, &opts).map_err(|e| Error::Config(e.to_string()))?
        } else {
            gen_mihomo::from_nodes(&nodes, &plan, &bridges, &opts)
        };
        // Kova itself has administrator or root rights (TUN on macOS, or started that
        // way), or the Mihomo it starts has network rights (the Linux TUN copy): its
        // config, which a subscription may have written, gets the care the Kova service
        // gives every config.
        let yaml = if !via_service && (crate::process::is_elevated() || has_net_rights) { gen_mihomo::sanitize_for_service(&yaml).map_err(|e| Error::Config(e.to_string()))? } else { yaml };
        let cfg = home.join("config.yaml");
        std::fs::write(&cfg, &yaml)?;
        let (h, c) = (home.to_string_lossy().to_string(), cfg.to_string_lossy().to_string());
        let mut log_poller = None;
        // Ends the log poller if this launch is dropped half way (a cancelled connect).
        let mut poller_guard = AbortOnDrop(None);
        let mut front = if via_service {
            use crate::service::{Request, call};
            // Only show log lines from this run.
            let since = call(&Request::Status, Duration::from_secs(5)).await?.log_seq;
            let tail = Arc::new(std::sync::Mutex::new(Vec::new()));
            log_poller = Some(self.spawn_service_logs(since, tail.clone()));
            poller_guard.0 = log_poller.as_ref().map(|p| p.abort_handle());
            // The first start may download Mihomo and the routing data into the service.
            let start = Request::Start { config: yaml, geodata: opts.geodata.clone(), owner_pid: std::process::id(), files: provider_files };
            self.service_started.store(true, std::sync::atomic::Ordering::Relaxed);
            if let Err(e) = call(&start, Duration::from_secs(600)).await {
                if let Some(p) = log_poller {
                    p.abort();
                }
                // The service may have started Mihomo after our timeout; don't leave it
                // capturing traffic while Kova reports a failure.
                crate::service::stop_remote().await;
                if let Some(s) = sidecar {
                    s.stop().await;
                }
                return Err(e);
            }
            Front::Service(crate::service::RemoteCore::new(tail))
        } else {
            Front::Local(CoreProcess::spawn_with_fd("mihomo", &mihomo_bin, &["-d", &h, "-f", &c], &home, &[], self.log_sink(quiet), opts.tun_fd)?)
        };
        let api = Mihomo::new(opts.controller_port, &opts.secret);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(if via_service { 60 } else { 30 });
        loop {
            if let Some(msg) = front.exited().await {
                if let Some(p) = &log_poller {
                    p.abort();
                }
                if via_service {
                    crate::service::stop_remote().await;
                }
                if let Some(s) = sidecar {
                    s.stop().await;
                }
                return Err(Error::Core(msg));
            }
            if api.version().await.is_ok() {
                break;
            }
            if tokio::time::Instant::now() > deadline {
                let msg = front.last_error();
                front.stop().await;
                if let Some(p) = &log_poller {
                    p.abort();
                }
                if let Some(s) = sidecar {
                    s.stop().await;
                }
                return Err(Error::Core(format!("Mihomo did not start in time. {msg}")));
            }
            tokio::time::sleep(Duration::from_millis(150)).await;
        }
        poller_guard.0 = None;
        let mut r = Running {
            id: next_run_id(),
            front,
            front_kind: CoreKind::Mihomo,
            sidecar,
            shoes,
            tasks: Vec::new(),
            api: Some(api.clone()),
            nodes,
            metrics_port: 0,
            main_group,
            tun_active: opts.tun.enabled,
            warning,
        };
        r.tasks.extend(log_poller);
        if !quiet {
            if let Some(sm) = opts.smart.clone() {
                r.tasks.push(self.spawn_learner(api.clone(), sm));
            }
            r.tasks.push(self.spawn_mihomo_streams(api));
            r.tasks.push(self.spawn_watchdog());
        }
        Ok((r, plan))
    }

    fn spawn_learner(&self, api: Mihomo, sm: SmartRuntime) -> JoinHandle<()> {
        use crate::smart::Hooks;
        let (me, me2, me3, me4) = (self.me.clone(), self.me.clone(), self.me.clone(), self.me.clone());
        let hooks = Hooks {
            quiet: Box::new(move || me4.upgrade().is_some_and(|e| e.quiet.load(std::sync::atomic::Ordering::Relaxed))),
            active: Box::new(move || me.upgrade().is_some_and(|e| {
                let c = e.settings().core;
                c.smart_routing && c.mode == Mode::Rule
            })),
            learned: Box::new(move |l| {
                if let Some(e) = me2.upgrade() {
                    let how = match l.route {
                        kova_core::smart::Route::Proxy => "now goes through the proxy",
                        kova_core::smart::Route::Direct => "now goes direct",
                    };
                    e.emit(Event::Log { source: "kova".into(), level: "info".into(), message: format!("smart routing: {} failed on its usual path ({}), {how}", l.domain, l.reason) });
                    e.emit(Event::Learned { domain: l.domain.clone(), route: l.route, reason: l.reason.clone() });
                }
            }),
            suggested: Box::new(move |s| {
                if let Some(e) = me3.upgrade() {
                    e.emit(Event::Log { source: "kova".into(), level: "info".into(), message: format!("smart routing: {} fails on the route {} chose ({}); suggesting an exception", s.domain, s.rule, s.reason) });
                    e.emit(Event::Suggested { domain: s.domain.clone(), route: s.route, rule: s.rule.clone(), origin: s.origin });
                }
            }),
        };
        tokio::spawn(crate::smart::learn(self.smart.clone(), api, sm, hooks))
    }

    // ------------------------------------------------------------------ smart routing

    /// Sites smart routing learned, newest first.
    pub fn learned_sites(&self) -> Vec<kova_core::smart::Learned> {
        self.smart.list()
    }

    pub async fn forget_learned(&self, domain: &str) -> Result<()> {
        self.smart.forget(domain);
        self.refresh_learned().await;
        Ok(())
    }

    pub async fn clear_learned(&self) -> Result<()> {
        self.smart.clear();
        self.refresh_learned().await;
        Ok(())
    }

    /// Turn a learned site into a rule of the user's own, so it never expires.
    pub async fn keep_learned(&self, domain: &str) -> Result<()> {
        let l = self.smart.get(domain).ok_or_else(|| Error::Other(format!("{domain} is not in the learned list")))?;
        let target = match l.route {
            kova_core::smart::Route::Proxy => PROXY,
            kova_core::smart::Route::Direct => "DIRECT",
        };
        let rule = match kova_core::smart::parse_ip(&l.domain) {
            Some(std::net::IpAddr::V4(ip)) => Rule { kind: RuleKind::IpCidr, value: format!("{ip}/32"), target: target.into() },
            Some(std::net::IpAddr::V6(ip)) => Rule { kind: RuleKind::IpCidr, value: format!("{ip}/128"), target: target.into() },
            None => Rule { kind: RuleKind::DomainSuffix, value: l.domain.clone(), target: target.into() },
        };
        let mut s = self.settings();
        s.core.custom_rules.retain(|r| !(r.kind == rule.kind && r.value == rule.value));
        // On top, like every new rule.
        s.core.custom_rules.insert(0, rule);
        self.smart.forget(domain);
        self.update_settings(s).await
    }

    pub fn smart_suggestions(&self) -> Vec<kova_core::smart::Suggestion> {
        self.smart.suggestions()
    }

    pub fn dismiss_suggestion(&self, domain: &str) {
        self.smart.dismiss_suggestion(domain);
    }

    /// Apply a suggestion: an exception for the site, placed right above the user's rule
    /// it is an exception to (or first, for a preset's or the subscription's rule).
    pub async fn accept_suggestion(&self, domain: &str) -> Result<()> {
        let s = self.smart.take_suggestion(domain).ok_or_else(|| Error::Other(format!("there is no suggestion for {domain}")))?;
        let target = match s.route {
            kova_core::smart::Route::Proxy => PROXY,
            kova_core::smart::Route::Direct => "DIRECT",
        };
        let rule = match kova_core::smart::parse_ip(&s.domain) {
            Some(std::net::IpAddr::V4(ip)) => Rule { kind: RuleKind::IpCidr, value: format!("{ip}/32"), target: target.into() },
            Some(std::net::IpAddr::V6(ip)) => Rule { kind: RuleKind::IpCidr, value: format!("{ip}/128"), target: target.into() },
            None => Rule { kind: RuleKind::DomainSuffix, value: s.domain.clone(), target: target.into() },
        };
        let mut settings = self.settings();
        let rules = &mut settings.core.custom_rules;
        rules.retain(|r| !(r.kind == rule.kind && r.value == rule.value));
        let key = |r: &Rule| kova_core::smart::rule_key(r.kind.mihomo_name(), &r.value);
        let at = rules.iter().position(|r| key(r) == s.rule).unwrap_or(0);
        rules.insert(at, rule);
        self.update_settings(settings).await
    }

    /// Make the running core pick up a changed learned list.
    async fn refresh_learned(&self) {
        let api = self.run.lock().await.as_ref().and_then(|r| r.api.clone());
        if let Some(api) = api
            && self.settings().core.smart_routing {
                crate::smart::refresh(&api).await;
            }
    }

    /// Relay Mihomo's log lines from the service into the normal log stream.
    fn spawn_service_logs(&self, mut since: u64, tail: Arc<std::sync::Mutex<Vec<String>>>) -> JoinHandle<()> {
        let sink = self.log_sink(false);
        tokio::spawn(async move {
            use crate::service::{Request, call};
            loop {
                if let Ok(r) = call(&Request::Logs { since }, Duration::from_secs(5)).await {
                    for (n, line) in r.lines {
                        since = n;
                        {
                            let mut t = tail.lock().unwrap();
                            t.push(line.clone());
                            if t.len() > 40 {
                                t.remove(0);
                            }
                        }
                        sink("mihomo", &line);
                    }
                }
                tokio::time::sleep(Duration::from_millis(700)).await;
            }
        })
    }

    fn spawn_mihomo_streams(&self, api: Mihomo) -> JoinHandle<()> {
        let me = self.me.clone();
        tokio::spawn(async move {
            let traffic = {
                let me = me.clone();
                let api = api.clone();
                async move {
                    let (mut up_total, mut down_total) = (0u64, 0u64);
                    loop {
                        let _ = api
                            .stream("/traffic", |v| {
                                let up = v["up"].as_u64().unwrap_or(0);
                                let down = v["down"].as_u64().unwrap_or(0);
                                up_total = v["upTotal"].as_u64().unwrap_or(up_total.saturating_add(up));
                                down_total = v["downTotal"].as_u64().unwrap_or(down_total.saturating_add(down));
                                match me.upgrade() {
                                    Some(e) => {
                                        e.emit(Event::Traffic { up, down, up_total, down_total });
                                        true
                                    }
                                    None => false,
                                }
                            })
                            .await;
                        tokio::time::sleep(Duration::from_secs(1)).await;
                    }
                }
            };
            let memory = async move {
                loop {
                    let _ = api
                        .stream("/memory", |v| match me.upgrade() {
                            Some(e) => {
                                e.emit(Event::Memory { bytes: v["inuse"].as_u64().unwrap_or(0) });
                                true
                            }
                            None => false,
                        })
                        .await;
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            };
            tokio::join!(traffic, memory);
        })
    }

    fn spawn_xray_metrics(&self, port: u16) -> JoinHandle<()> {
        let me = self.me.clone();
        tokio::spawn(async move {
            let http = fetch::builder().no_proxy().timeout(Duration::from_secs(2)).build().expect("client");
            let url = format!("http://127.0.0.1:{port}/debug/vars");
            let (mut last_up, mut last_down) = (None::<u64>, None::<u64>);
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                let Some(engine) = me.upgrade() else { return };
                let Ok(v) = http.get(&url).send().await else { continue };
                let Ok(v) = v.json::<serde_json::Value>().await else { continue };
                let sum = |dir: &str| -> u64 {
                    v["stats"]["inbound"].as_object().map(|m| m.values().map(|x| x[dir].as_u64().unwrap_or(0)).fold(0u64, u64::saturating_add)).unwrap_or(0)
                };
                let (up_total, down_total) = (sum("uplink"), sum("downlink"));
                let up = last_up.map(|l| up_total.saturating_sub(l)).unwrap_or(0);
                let down = last_down.map(|l| down_total.saturating_sub(l)).unwrap_or(0);
                last_up = Some(up_total);
                last_down = Some(down_total);
                engine.emit(Event::Traffic { up, down, up_total, down_total });
                if let Some(obs) = v["observatory"].as_object() {
                    let delays = obs.iter().map(|(k, o)| (gen_xray::node_of_tag(k).to_string(), o["delay"].as_u64().filter(|_| o["alive"].as_bool().unwrap_or(false)).map(|d| d as u32))).collect::<HashMap<_, _>>();
                    engine.delays.write().unwrap().extend(delays.clone());
                }
            }
        })
    }

    /// Notices a core that died on its own and cleans up after it.
    fn spawn_watchdog(&self) -> JoinHandle<()> {
        let me = self.me.clone();
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(2)).await;
                let Some(engine) = me.upgrade() else { return };
                // Local processes are looked at under the lock; the service is asked
                // without it, as that can take seconds and everything else waits on it.
                let (id, local, remote) = {
                    let mut guard = engine.run.lock().await;
                    let Some(r) = guard.as_mut() else { return };
                    let local = match &mut r.front {
                        Front::Local(p) => p.exited(),
                        Front::Service(_) => None,
                    };
                    let local = local.or_else(|| r.sidecar.as_mut().and_then(|s| s.exited())).or_else(|| r.shoes.as_mut().and_then(|s| s.exited()));
                    let remote = match &r.front {
                        Front::Service(s) => Some(s.clone()),
                        Front::Local(_) => None,
                    };
                    (r.id, local, remote)
                };
                let died = match (local, remote) {
                    (Some(m), _) => Some(m),
                    (None, Some(s)) => s.exited().await,
                    (None, None) => None,
                };
                if let Some(msg) = died {
                    let mut guard = engine.run.lock().await;
                    // Disconnected or reconnected meanwhile: not this connection's to clean up.
                    if guard.as_ref().map(|r| r.id) != Some(id) {
                        return;
                    }
                    let r = guard.take();
                    drop(guard);
                    let kill_switch = engine.settings().kill_switch;
                    if kill_switch {
                        engine.holding.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    // Only a system proxy can be held; in TUN-only mode the kill switch
                    // just reconnects.
                    let holds_proxy = kill_switch && engine.settings().system_proxy;
                    // Clean up in a task of its own: shutdown() aborts every task in
                    // `r.tasks`, including this watchdog, which would cancel the cleanup.
                    tokio::spawn(async move {
                        if !holds_proxy {
                            let _ = engine.release_proxy();
                        }
                        if let Some(r) = r {
                            r.shutdown().await;
                        }
                        engine.set_status(|s| {
                            s.running = false;
                            s.busy = false;
                            s.core = None;
                            s.sidecar = false;
                            s.since = None;
                            s.tun_active = false;
                            s.error = Some(msg);
                        });
                        engine.emit(Event::Traffic { up: 0, down: 0, up_total: 0, down_total: 0 });
                        if kill_switch {
                            engine.reconnect_after_drop(holds_proxy).await;
                        }
                    });
                    return;
                }
            }
        })
    }

    /// The kill switch: try to bring the connection back a few times while apps stay
    /// offline. Stops as soon as the user disconnects (which lifts the hold).
    async fn reconnect_after_drop(&self, holds_proxy: bool) {
        use std::sync::atomic::Ordering::Relaxed;
        for wait in [2, 5, 10] {
            tokio::time::sleep(Duration::from_secs(wait)).await;
            // The count first: a disconnect after this makes the start give up.
            let epoch = self.stop_epoch();
            if !self.holding.load(Relaxed) || self.status().running {
                return;
            }
            // A start cancelled by a disconnect also returns Ok, with nothing running.
            if self.start_at(epoch).await.is_ok() && self.status().running {
                let text = if holds_proxy {
                    "The connection dropped and is back; the kill switch kept apps offline meanwhile"
                } else {
                    "The connection dropped and is back"
                };
                self.emit(Event::Notice { text: text.into() });
                return;
            }
        }
        if !self.holding.load(Relaxed) {
            return;
        }
        if holds_proxy {
            self.set_status(|s| s.error = Some("The connection dropped. The kill switch keeps apps offline until you connect again or disconnect".into()));
        } else {
            // Nothing to hold (TUN only): let go, and say what happened.
            self.holding.store(false, Relaxed);
            self.set_status(|s| s.error = Some("The connection dropped and couldn't be restored. Connect again".into()));
        }
    }

    // ------------------------------------------------------------------ proxies

    fn selection_key(&self, group: &str) -> String {
        format!("{}/{group}", self.settings().active_profile.unwrap_or_default())
    }

    async fn restore_selections(&self, api: &Mihomo) {
        let prefix = self.selection_key("");
        let wanted: Vec<(String, String)> = self.settings().selections.iter().filter_map(|(k, v)| k.strip_prefix(&prefix).map(|g| (g.to_string(), v.clone()))).collect();
        for (group, name) in wanted {
            let _ = api.select(&group, &name).await;
        }
    }

    /// Groups and nodes, live from the core if it is running, otherwise from the profile.
    pub async fn proxies(&self) -> Result<ProxySnapshot> {
        let (api, xray_nodes) = {
            let run = self.run.lock().await;
            match run.as_ref() {
                Some(r) => (r.api.clone(), (r.front_kind == CoreKind::Xray).then(|| r.nodes.clone())),
                None => (None, None),
            }
        };
        if let Some(api) = api {
            let mut snap = api.proxies().await?;
            let cached = self.delays.read().unwrap().clone();
            for (name, p) in snap.proxies.iter_mut() {
                if p.delay.is_none() {
                    p.delay = cached.get(name).copied().flatten();
                }
            }
            return Ok(snap);
        }
        let nodes = match xray_nodes {
            Some(n) => n,
            None => match self.active() {
                Ok((p, body, n)) if p.format == Format::Clash && p.core != CorePreference::Xray => {
                    return Ok(self.offline_clash_snapshot(&body, &n));
                }
                Ok((_, _, n)) => n,
                Err(_) => return Ok(ProxySnapshot::default()),
            },
        };
        Ok(self.offline_snapshot(&nodes))
    }

    /// A disconnected Clash profile: its own groups, with remembered selections.
    fn offline_clash_snapshot(&self, body: &str, nodes: &[Node]) -> ProxySnapshot {
        let delays = self.delays.read().unwrap().clone();
        let selections = self.settings().selections;
        let mut proxies: HashMap<String, ProxyInfo> = nodes
            .iter()
            .map(|n| {
                let d = delays.get(&n.name).copied().flatten();
                (n.name.clone(), ProxyInfo { name: n.name.clone(), kind: n.protocol_label(), udp: n.udp, alive: d.is_some(), delay: d })
            })
            .collect();
        let groups: Vec<ProxyGroup> = kova_core::clash::groups(body)
            .into_iter()
            .map(|g| {
                let saved = selections.get(&self.selection_key(&g.name)).filter(|s| g.members.contains(s)).cloned();
                let now = saved.or_else(|| g.members.first().cloned()).unwrap_or_default();
                proxies.insert(g.name.clone(), ProxyInfo { name: g.name.clone(), kind: g.kind.clone(), udp: true, alive: true, delay: None });
                ProxyGroup { name: g.name, kind: g.kind, now, all: g.members }
            })
            .collect();
        ProxySnapshot { groups, proxies }
    }

    fn offline_snapshot(&self, nodes: &[Node]) -> ProxySnapshot {
        let delays = self.delays.read().unwrap().clone();
        let names: Vec<String> = nodes.iter().map(|n| n.name.clone()).collect();
        let mut proxies: HashMap<String, ProxyInfo> = nodes
            .iter()
            .map(|n| {
                let d = delays.get(&n.name).copied().flatten();
                (n.name.clone(), ProxyInfo { name: n.name.clone(), kind: n.protocol_label(), udp: n.udp, alive: d.is_some(), delay: d })
            })
            .collect();
        let fastest = names.iter().filter_map(|n| delays.get(n).copied().flatten().map(|d| (n.clone(), d))).min_by_key(|(_, d)| *d).map(|(n, _)| n).or_else(|| names.first().cloned()).unwrap_or_default();
        let key = self.selection_key(PROXY);
        let chosen = self.settings().xray_selection.or_else(|| self.settings().selections.get(&key).cloned()).unwrap_or_else(|| AUTO.to_string());
        let mut all = vec![AUTO.to_string(), FALLBACK.to_string()];
        all.extend(names.iter().cloned());
        for (g, kind) in [(AUTO, "URLTest"), (FALLBACK, "Fallback")] {
            proxies.insert(g.into(), ProxyInfo { name: g.into(), kind: kind.into(), udp: true, alive: true, delay: delays.get(&fastest).copied().flatten() });
        }
        ProxySnapshot {
            groups: vec![
                ProxyGroup { name: PROXY.into(), kind: "Selector".into(), now: chosen, all },
                ProxyGroup { name: AUTO.into(), kind: "URLTest".into(), now: fastest.clone(), all: names.clone() },
                ProxyGroup { name: FALLBACK.into(), kind: "Fallback".into(), now: fastest, all: names },
            ],
            proxies,
        }
    }

    pub async fn select(&self, group: &str, name: &str) -> Result<()> {
        let key = self.selection_key(group);
        {
            let mut s = self.settings.write().unwrap();
            s.selections.insert(key, name.to_string());
        }
        let (api, front) = {
            let run = self.run.lock().await;
            (run.as_ref().and_then(|r| r.api.clone()), run.as_ref().map(|r| r.front_kind))
        };
        if front == Some(CoreKind::Xray) || (front.is_none() && group == PROXY) {
            let mut s = self.settings.write().unwrap();
            s.xray_selection = (name != AUTO && name != FALLBACK).then(|| name.to_string());
        }
        self.settings().save(&self.paths)?;
        match (api, front) {
            (Some(api), _) => {
                api.select(group, name).await?;
                // Existing connections keep their old route; close them so the switch is immediate.
                let _ = api.close_all().await;
                Ok(())
            }
            (None, Some(CoreKind::Xray)) => self.restart().await,
            _ => Ok(()),
        }
    }

    /// Measure latency for `names` (or every node). Uses the running Mihomo, or a
    /// temporary one on spare ports when disconnected or running Xray.
    pub async fn test_delays(&self, names: Option<Vec<String>>) -> Result<HashMap<String, Option<u32>>> {
        let running_api = self.run.lock().await.as_ref().and_then(|r| r.api.clone());
        let (api, tester) = match running_api {
            Some(a) => (a, None),
            None => {
                let (r, _) = self.launch(Purpose::Tester, self.settings().core).await?;
                (r.api.clone().expect("tester runs mihomo"), Some(r))
            }
        };
        let names = match names {
            Some(n) => n,
            None => {
                let snap = api.proxies().await?;
                let groups: Vec<&str> = snap.groups.iter().map(|g| g.name.as_str()).collect();
                // Built-in policies (DIRECT, REJECT, PASS-RULE, ...) have no server to test.
                let builtin_kinds = ["Direct", "Reject", "RejectDrop", "Pass", "Compatible", "Dns"];
                let builtin_names = ["DIRECT", "REJECT", "REJECT-DROP", "PASS", "PASS-RULE", "COMPATIBLE"];
                snap.proxies
                    .values()
                    .filter(|p| !groups.contains(&p.name.as_str()) && !builtin_kinds.contains(&p.kind.as_str()) && !builtin_names.contains(&p.name.as_str()))
                    .map(|p| p.name.clone())
                    .collect()
            }
        };
        let test_url = self.settings().core.test_url().to_string();
        // A test started later (pressing "Test latency" during the one after connecting)
        // takes over: this one stops starting new checks.
        let run = self.delay_run.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1;
        let current = || self.delay_run.load(std::sync::atomic::Ordering::SeqCst) == run;
        // Dead servers each wait out the timeout, so many go at once.
        let at_once = if cfg!(target_os = "android") { 32 } else { 64 };
        let mut checks = futures_util::stream::iter(names)
            .take_while(|_| std::future::ready(current()))
            .map(|n| {
                let api = api.clone();
                let test_url = &test_url;
                async move {
                    let d = api.delay(&n, test_url, 5000).await.ok().flatten();
                    (n, d)
                }
            })
            .buffer_unordered(at_once);
        // Results go out as they come, a few times a second, so fast servers show at once.
        let mut map: HashMap<String, Option<u32>> = HashMap::new();
        let mut batch: HashMap<String, Option<u32>> = HashMap::new();
        let mut sent = std::time::Instant::now();
        while let Some((n, d)) = checks.next().await {
            map.insert(n.clone(), d);
            batch.insert(n, d);
            if sent.elapsed() >= Duration::from_millis(250) {
                self.delays.write().unwrap().extend(batch.clone());
                self.emit(Event::Delays { delays: std::mem::take(&mut batch) });
                sent = std::time::Instant::now();
            }
        }
        drop(checks);
        // Nothing answered: say why in the log, as the toast can only say that.
        if current()
            && !map.is_empty()
            && map.values().all(Option::is_none)
            && let Some(first) = map.keys().next()
        {
            let why = api.delay_failure(first, &test_url, 5000).await;
            self.emit(Event::Log { source: "kova".into(), level: "warning".into(), message: format!("none of {} servers answered {test_url} within 5 s; {first}: {why}", map.len()) });
        }
        if let Some(t) = tester {
            t.shutdown().await;
        }
        if !batch.is_empty() {
            self.delays.write().unwrap().extend(batch.clone());
            self.emit(Event::Delays { delays: batch });
        }
        Ok(map)
    }

    /// Latency spread, jitter and loss of the connection now: ten probes to the latency
    /// test address, through the proxy while connected and directly otherwise.
    pub async fn check_quality(&self) -> Result<crate::quality::Quality> {
        let url = self.settings().core.test_url().to_string();
        crate::quality::probe(&url, self.proxy_url().as_deref(), 10, Duration::from_secs(5)).await
    }

    /// Android: the screen went off or on.
    pub fn set_quiet(&self, quiet: bool) {
        self.quiet.store(quiet, std::sync::atomic::Ordering::Relaxed);
    }

    /// Android: the phone had no network at all (airplane mode, a tunnel) and has one again.
    /// The core's connections, and the sockets it keeps for its servers, died with the old
    /// one, and the network usually comes back as the same kind, so nothing else notices.
    /// Starting the core over is what reconnecting by hand does.
    pub async fn network_restored(&self) {
        use std::sync::atomic::Ordering::SeqCst;
        // The link and its DNS are not ready the moment the network reports back.
        tokio::time::sleep(Duration::from_millis(1500)).await;
        if !self.status().running || !self.has_tun_fd() || self.network_restarting.swap(true, SeqCst) {
            return;
        }
        let restarted = self.restart().await;
        self.network_restarting.store(false, SeqCst);
        match restarted {
            Ok(()) => self.emit(Event::Log { source: "kova".into(), level: "info".into(), message: "the network came back; the connection was started over".into() }),
            Err(e) => tracing::warn!("restart after the network came back failed: {e}"),
        }
    }

    /// Android: the phone moved between Wi-Fi and mobile data. With a server set for
    /// Android: the DNS servers of the network the phone is on. Sites that go direct are
    /// looked up with them too, as some Wi-Fi networks let no other DNS server answer. A
    /// change while connected restarts the core: the new network has broken its
    /// connections anyway.
    pub async fn set_network_dns(&self, dns: Vec<String>) {
        let dns: Vec<String> = dns.iter().map(|d| d.trim()).filter(|d| d.parse::<std::net::IpAddr>().is_ok()).map(String::from).collect();
        if dns.is_empty() {
            return;
        }
        {
            let mut cur = self.network_dns.write().unwrap();
            if *cur == dns {
                return;
            }
            *cur = dns;
        }
        if self.status().running && self.has_tun_fd()
            && let Err(e) = self.restart().await
        {
            tracing::warn!("restart for the new network's DNS failed: {e}");
        }
    }

    /// mobile data, switch to it there and back to the usual choice on Wi-Fi. Neither
    /// switch is saved as the user's choice, which stays the Wi-Fi one.
    pub async fn network_changed(&self, mobile: bool) {
        use std::sync::atomic::Ordering::Relaxed;
        if self.on_mobile.swap(mobile, Relaxed) == mobile || self.settings().mobile_server.is_none() {
            return;
        }
        self.apply_network_choice().await;
    }

    /// Select the main group's server for the current network: the mobile-data one on
    /// mobile data (when set), else the user's own choice.
    async fn apply_network_choice(&self) {
        let mobile = self.on_mobile.load(std::sync::atomic::Ordering::Relaxed);
        let (api, main) = {
            let run = self.run.lock().await;
            match run.as_ref() {
                Some(r) => (r.api.clone(), r.main_group.clone()),
                None => return,
            }
        };
        let Some(api) = api else { return };
        let (target, on_mobile_server) = match self.settings().mobile_server.filter(|_| mobile) {
            Some(server) => (server, true),
            None => (self.settings().selections.get(&self.selection_key(&main)).cloned().unwrap_or_else(|| AUTO.to_string()), false),
        };
        if api.select(&main, &target).await.is_ok() {
            // Connections made on the other network keep their old route; start them over.
            let _ = api.close_all().await;
            let text = match (mobile, on_mobile_server) {
                (true, true) => format!("Mobile data: switched to {target}"),
                // The mobile-data server was just turned off in Settings.
                (true, false) => format!("Mobile data: using {target}"),
                (false, _) => format!("Wi-Fi: back to {target}"),
            };
            self.emit(Event::Notice { text });
        }
    }

    /// The main group while connected: its name, its current choice and its members,
    /// for picking a server outside the window (the tray).
    pub async fn main_choices(&self) -> Option<(String, String, Vec<String>)> {
        let (api, main) = {
            let run = self.run.lock().await;
            let r = run.as_ref()?;
            (r.api.clone()?, r.main_group.clone())
        };
        let snap = api.proxies().await.ok()?;
        let g = snap.groups.into_iter().find(|g| g.name == main)?;
        Some((g.name, g.now, g.all))
    }

    /// The node traffic goes through now: the main group's choice, followed through
    /// groups that choose themselves (Auto, Fallback).
    pub async fn current_node(&self) -> Option<String> {
        let (api, main) = {
            let run = self.run.lock().await;
            let r = run.as_ref()?;
            (r.api.clone()?, r.main_group.clone())
        };
        let snap = api.proxies().await.ok()?;
        let mut name = main;
        for _ in 0..6 {
            match snap.groups.iter().find(|g| g.name == name) {
                Some(g) if !g.now.is_empty() => name = g.now.clone(),
                _ => return Some(name),
            }
        }
        Some(name)
    }

    /// Called every minute or so while connected: when the node chosen in the main group
    /// has failed two checks in a row, pick the fastest working node of that group and
    /// say which. Groups that pick a node themselves (Auto, Fallback) are left alone, and
    /// so is everything when no node answers, as then the network itself is down.
    ///
    /// The switch lasts for this connection only, like the one for mobile data: the
    /// user's own choice stays saved, is used again on the next connect (and switched
    /// away from again if it is still down), and is where Wi-Fi goes back to.
    pub async fn check_node(&self) -> Option<(String, String)> {
        use std::sync::atomic::Ordering;
        if !self.settings().auto_switch || self.settings().core.mode == Mode::Direct {
            return None;
        }
        let (api, main) = {
            let run = self.run.lock().await;
            let r = run.as_ref()?;
            (r.api.clone()?, r.main_group.clone())
        };
        let snap = api.proxies().await.ok()?;
        let group = snap.groups.iter().find(|g| g.name == main && g.kind == "Selector")?;
        let is_node = |n: &str| snap.proxies.get(n).is_some_and(|p| !snap.groups.iter().any(|g| g.name == n) && !["Direct", "Reject", "RejectDrop", "Pass", "Compatible", "Dns"].contains(&p.kind.as_str()));
        let chosen = group.now.clone();
        if !is_node(&chosen) {
            return None;
        }
        let test_url = self.settings().core.test_url().to_string();
        if api.delay(&chosen, &test_url, 5000).await.ok().flatten().is_some() {
            self.node_failures.store(0, Ordering::SeqCst);
            return None;
        }
        if self.node_failures.fetch_add(1, Ordering::SeqCst) + 1 < 2 {
            return None;
        }
        self.node_failures.store(0, Ordering::SeqCst);
        let candidates: Vec<String> = group.all.iter().filter(|n| **n != chosen && is_node(n)).cloned().collect();
        let tested: Vec<(String, Option<u32>)> = futures_util::stream::iter(candidates)
            .map(|n| {
                let api = api.clone();
                let test_url = &test_url;
                async move {
                    let d = api.delay(&n, test_url, 5000).await.ok().flatten();
                    (n, d)
                }
            })
            .buffer_unordered(16)
            .collect()
            .await;
        let (best, _) = tested.into_iter().filter_map(|(n, d)| d.map(|d| (n, d))).min_by_key(|(_, d)| *d)?;
        api.select(&main, &best).await.ok()?;
        // Connections stuck on the dead node would keep hanging; start them over.
        let _ = api.close_all().await;
        Some((chosen, best))
    }

    /// Check one site through Kova (when connected) and directly. A site that opens only
    /// directly becomes a smart-routing suggestion to send it direct.
    pub async fn check_site(&self, url: &str) -> crate::sitecheck::SiteResult {
        use crate::sitecheck::Outcome;
        let r = crate::sitecheck::check(url, self.proxy_url().as_deref()).await;
        let works = |o: &Outcome| matches!(o, Outcome::Ok | Outcome::HttpError);
        if let Some(via) = &r.via_kova {
            let host = reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(str::to_string));
            if let (false, true, Some(domain)) = (works(&via.outcome), works(&r.direct.outcome), host.as_deref().and_then(kova_core::smart::rule_target)) {
                let reason = serde_json::to_value(&via.outcome).ok().and_then(|v| v.as_str().map(String::from)).unwrap_or_default();
                let s = kova_core::smart::Suggestion { domain, route: kova_core::smart::Route::Direct, reason, rule: String::new(), origin: kova_core::smart::Origin::SiteCheck, at: crate::store::now() };
                self.smart.suggest(s.clone());
                self.emit(Event::Suggested { domain: s.domain, route: s.route, rule: s.rule, origin: s.origin });
            }
        }
        r
    }

    // ------------------------------------------------------------------ connections

    /// The running Mihomo's controller, if Mihomo runs.
    pub async fn mihomo(&self) -> Option<Mihomo> {
        self.api().await
    }

    async fn api(&self) -> Option<Mihomo> {
        self.run.lock().await.as_ref().and_then(|r| r.api.clone())
    }

    pub async fn connections(&self) -> Result<ConnectionsSnapshot> {
        match self.api().await {
            Some(a) => a.connections().await,
            None => Ok(ConnectionsSnapshot::default()),
        }
    }

    pub async fn close_connection(&self, id: &str) -> Result<()> {
        if let Some(a) = self.api().await {
            a.close_connection(id).await?;
        }
        Ok(())
    }

    pub async fn close_all_connections(&self) -> Result<()> {
        if let Some(a) = self.api().await {
            a.close_all().await?;
        }
        Ok(())
    }

    // ------------------------------------------------------------------ cores

    pub async fn core_status(&self, check_latest: bool) -> Vec<CoreStatus> {
        let dir = self.paths.cores_dir();
        let (m, x) = tokio::join!(cores::status(&dir, CoreKind::Mihomo, check_latest), cores::status(&dir, CoreKind::Xray, check_latest));
        let mut out = vec![m, x];
        // Shoes only where it is published, and only shown, never fetched on its own.
        if cores::available(CoreKind::Shoes) && !cfg!(target_os = "android") {
            out.push(cores::status(&dir, CoreKind::Shoes, check_latest).await);
        }
        out
    }

    /// Install or update a core to its newest release, as the user asks in Settings.
    /// Restarts the connection if that core is in use.
    pub async fn install_core(&self, core: CoreKind) -> Result<String> {
        let tag = cores::install_latest(&self.paths.cores_dir(), core).await?;
        self.core_replaced(core).await?;
        Ok(tag)
    }

    /// Install the version of a core that Kova ships, for one that is missing (an
    /// AppImage carries none): the version this Kova was tested with, checked against
    /// its pinned checksum, not whatever is newest that day.
    pub async fn install_shipped_core(&self, core: CoreKind) -> Result<String> {
        let tag = cores::install_pinned(&self.paths.cores_dir(), core).await?;
        self.core_replaced(core).await?;
        Ok(tag)
    }

    /// Restart the connection if it runs on `core`, whose program was just replaced.
    async fn core_replaced(&self, core: CoreKind) -> Result<()> {
        let in_use = {
            let run = self.run.lock().await;
            run.as_ref().is_some_and(|r| r.front_kind == core || (core == CoreKind::Xray && r.sidecar.is_some()) || (core == CoreKind::Shoes && r.shoes.is_some()))
        };
        if in_use {
            self.restart().await?;
        }
        Ok(())
    }

    /// Whether the current connection runs Mihomo inside the Kova service.
    pub async fn uses_service(&self) -> bool {
        matches!(self.run.lock().await.as_ref().map(|r| &r.front), Some(Front::Service(_)))
    }

    /// Ports the running Xray front uses for metrics (0 when not applicable).
    pub async fn xray_metrics_port(&self) -> u16 {
        self.run.lock().await.as_ref().map(|r| r.metrics_port).unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::{Engine, parse_log};

    #[cfg(windows)]
    #[test]
    fn the_program_holding_a_port_is_named() {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        let pid = super::listener_pid("127.0.0.1".parse().unwrap(), port).expect("this process listens on the port");
        assert_eq!(pid, std::process::id());
        // A wildcard bind conflicts with a listener on any IPv4 address.
        assert_eq!(super::listener_pid("0.0.0.0".parse().unwrap(), port), Some(pid));
        let me = std::env::current_exe().unwrap().file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(super::port_owner("127.0.0.1", port), Some(me));
    }

    #[cfg(windows)]
    #[test]
    fn an_ipv6_listener_is_found() {
        // Skipped quietly where IPv6 is switched off.
        let Ok(l) = std::net::TcpListener::bind("[::]:0") else { return };
        let port = l.local_addr().unwrap().port();
        assert_eq!(super::listener_pid("::".parse().unwrap(), port), Some(std::process::id()));
        let me = std::env::current_exe().unwrap().file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(super::port_owner("::", port), Some(me));
    }

    #[cfg(windows)]
    #[test]
    fn a_listener_on_another_address_is_not_a_conflict() {
        // Skipped quietly where 127.0.0.2 can't be bound.
        let Ok(l) = std::net::TcpListener::bind("127.0.0.2:0") else { return };
        let port = l.local_addr().unwrap().port();
        assert_eq!(super::listener_pid("127.0.0.1".parse().unwrap(), port), None);
        assert_eq!(super::listener_pid("127.0.0.2".parse().unwrap(), port), Some(std::process::id()));
    }

    #[test]
    fn tun_failure_from_log_is_reported() {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::new(dir.path()).unwrap();
        let sink = engine.log_sink(false);
        sink("mihomo", r#"time="2026-10-20T11:59:13+03:00" level=error msg="Start TUN listening error: configure tun interface: set ipv4 address: The object already exists.""#);
        let err = engine.tun_error.read().unwrap().clone().expect("TUN failure recorded");
        assert!(err.contains("The object already exists"), "{err}");
        // Other cores' lines and ordinary lines are ignored.
        let other = Engine::new(tempfile::tempdir().unwrap().path()).unwrap();
        other.log_sink(false)("xray", "2026/10/20 12:00:00 [Error] TUN listening error: x");
        other.log_sink(false)("mihomo", r#"time="x" level=info msg="[TCP] a.example:443""#);
        assert!(other.tun_error.read().unwrap().is_none());
    }

    #[test]
    fn log_lines() {
        let (l, m) = parse_log("mihomo", r#"time="2026-10-20T12:00:00.1+03:00" level=warning msg="[TCP] dial Proxy (match Match/) to a.example:443 error: timeout""#);
        assert_eq!(l, "warning");
        assert!(m.starts_with("[TCP] dial Proxy"));
        let (l, m) = parse_log("xray", "2026/10/20 12:00:00.123456 [Info] infra/conf/serial: Reading config");
        assert_eq!(l, "info");
        assert_eq!(m, "infra/conf/serial: Reading config");
        let (l, m) = parse_log("xray", "2026/10/20 12:00:00.123456 from 127.0.0.1:5000 accepted tcp:a.example:443 [http-in -> X]");
        assert_eq!(l, "info");
        assert!(m.starts_with("from 127.0.0.1"));
    }

    #[test]
    fn providers_on_this_computer_are_refused() {
        use super::provider_host;
        for url in [
            "http://localhost/sub",
            "http://LOCALHOST./sub",
            "http://app.localhost:8080/sub",
            "http://127.0.0.1/sub",
            "http://127.9.8.7:81/sub",
            "http://[::1]/sub",
            "http://[::ffff:127.0.0.1]/sub",
            "http://169.254.169.254/latest/meta-data",
            "http://[fe80::1]/sub",
            "http://0.0.0.0/sub",
            "http://[::]/sub",
            "not a url",
        ] {
            assert_eq!(provider_host(url), None, "{url}");
        }
        // LAN addresses and names are allowed (a server at home), and the host is lower case.
        assert_eq!(provider_host("http://192.168.1.5:8080/sub").as_deref(), Some("192.168.1.5"));
        assert_eq!(provider_host("https://10.0.0.2/sub").as_deref(), Some("10.0.0.2"));
        assert_eq!(provider_host("https://Sub.Example.com/x?y=1").as_deref(), Some("sub.example.com"));
        assert_eq!(super::url_host("https://Sub.Example.com:8443/x").as_deref(), Some("sub.example.com"));
    }

    #[test]
    fn a_provider_on_this_computer_needs_a_subscription_there_too() {
        use super::provider_allowed;
        // A subscription on a public host: a provider on this computer is refused.
        assert!(!provider_allowed("http://127.0.0.1:9/p", Some("https://sub.example.com/s")));
        assert!(provider_allowed("https://sub.example.com/p", Some("https://sub.example.com/s")));
        assert!(provider_allowed("https://other.example.org/p", Some("https://sub.example.com/s")));
        // A converter running here: its own host is fine, another local address is not.
        assert!(provider_allowed("http://127.0.0.1:25500/p", Some("http://127.0.0.1:25500/sub?x=1")));
        assert!(!provider_allowed("http://localhost:25500/p", Some("http://127.0.0.1:25500/sub")));
        // A local profile is the user's own config: nothing is refused.
        assert!(provider_allowed("http://127.0.0.1:25500/p", None));
    }

    /// A refused provider's file is a list Kova reads as having a server.
    #[test]
    fn a_refused_providers_file_has_a_server() {
        assert!(kova_core::gen_mihomo::provider_has_servers(super::REFUSED_PROVIDER));
    }

    #[tokio::test]
    async fn a_refused_provider_is_replaced_by_a_server_that_cannot_connect() {
        let dir = tempfile::tempdir().unwrap();
        let e = Engine::new(dir.path()).unwrap();
        let home = dir.path().join("home");
        let body = "proxy-providers:
  Mine: {type: http, url: 'http://127.0.0.1:9/sub', interval: 3600}
proxy-groups:
  - {name: Sel, type: select, use: [Mine]}
rules:
  - MATCH,Sel
";
        let (out, files) = e.localize_providers(body, &home, false, Some("https://sub.example.com/s")).await;
        assert!(!out.contains("127.0.0.1") && !out.contains("http"), "{out}");
        assert!(out.contains("type: file") && out.contains("use:"), "{out}");
        let file = super::gen_mihomo::provider_file("Mine");
        assert_eq!(files, [(file.clone(), super::REFUSED_PROVIDER.to_string())]);
        assert_eq!(std::fs::read_to_string(home.join(file)).unwrap(), super::REFUSED_PROVIDER);
    }

    #[test]
    fn routing_data_is_due_by_age_without_a_marker_and_not_again_after_a_failure() {
        use super::{geodata_age, geodata_due_at};
        use kova_core::options::GeoSource;
        let day = 86_400;
        let now = 1_000 * day;
        let marker = |source: &GeoSource, age_days: i64| crate::geodata::Marker { source: source.id().to_string(), verified: true, updated_at: now - age_days * day };
        let d = GeoSource::Default;
        // The files that came with Xray, no marker: due by age, not before.
        assert!(!geodata_due_at(&d, None, Some(2 * day), now, 0));
        assert!(geodata_due_at(&d, None, Some(30 * day), now, 0));
        // Missing files are due, and another source than the default is wanted at once.
        assert!(geodata_due_at(&d, None, None, now, 0));
        assert!(geodata_due_at(&GeoSource::Runetfreedom, None, Some(day / 24), now, 0));
        // With a marker: by its age, its source, and the files being there.
        assert!(!geodata_due_at(&d, Some(&marker(&d, 1)), Some(day), now, 0));
        assert!(geodata_due_at(&d, Some(&marker(&d, 30)), Some(day), now, 0));
        assert!(geodata_due_at(&d, Some(&marker(&GeoSource::Runetfreedom, 0)), Some(0), now, 0));
        assert!(geodata_due_at(&d, Some(&marker(&d, 1)), None, now, 0));
        // After a failure, an hour's pause.
        assert!(!geodata_due_at(&d, None, None, now, now - 600));
        assert!(geodata_due_at(&d, None, None, now, now - 3_700));

        // The age is the older file's; a missing file is None.
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(geodata_age(dir.path(), now), None);
        let t = |days: u64| std::time::SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(now as u64 - days * 86_400);
        for (f, days) in [("geoip.dat", 3), ("geosite.dat", 10)] {
            let file = std::fs::File::create(dir.path().join(f)).unwrap();
            file.set_modified(t(days)).unwrap();
        }
        assert_eq!(geodata_age(dir.path(), now), Some(10 * day));
    }

    /// A provider that redirects to this computer fails closed like one that starts there:
    /// Mihomo must not be left to fetch the address itself.
    #[tokio::test]
    async fn a_provider_redirected_to_this_computer_fails_closed() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let host = l.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut s, _) = l.accept().await.unwrap();
            let mut buf = vec![0; 4096];
            let _ = s.read(&mut buf).await;
            let _ = s.write_all(b"HTTP/1.1 302 Found\r\nlocation: http://localhost:1/x\r\ncontent-length: 0\r\nconnection: close\r\n\r\n").await;
        });
        let dir = tempfile::tempdir().unwrap();
        let e = Engine::new(dir.path()).unwrap();
        let home = dir.path().join("home");
        let body = format!("proxy-providers:\n  Mine: {{type: http, url: 'http://{host}/p', interval: 3600}}\nproxy-groups:\n  - {{name: Sel, type: select, use: [Mine]}}\nrules:\n  - MATCH,Sel\n");
        // The subscription is on that same host (a converter here): the provider itself is allowed.
        let (out, files) = e.localize_providers(&body, &home, false, Some(&format!("http://{host}/sub"))).await;
        assert!(out.contains("type: file") && !out.contains("http://"), "{out}");
        assert_eq!(files, [(super::gen_mihomo::provider_file("Mine"), super::REFUSED_PROVIDER.to_string())]);
    }

    #[test]
    fn a_backup_leaves_out_the_installs_secrets() {
        let dir = tempfile::tempdir().unwrap();
        let e = Engine::new(dir.path()).unwrap();
        let s = e.settings();
        assert!(!s.core.secret.is_empty() && !s.core.local_password.is_empty() && !s.device_id.is_empty());
        let text = e.export_backup(None).unwrap();
        for v in [&s.core.secret, &s.core.local_password, &s.device_id] {
            assert!(!text.contains(v.as_str()));
        }
        let b = crate::backup::read(&text, None).unwrap();
        assert!(b.settings.core.secret.is_empty() && b.settings.core.local_password.is_empty() && b.settings.device_id.is_empty());
    }

    /// Backups made before the secrets were left out carry them; both kinds import, and
    /// this install keeps its own.
    #[tokio::test]
    async fn importing_a_backup_keeps_the_installs_secrets() {
        for with_secrets in [true, false] {
            let dir = tempfile::tempdir().unwrap();
            let e = Engine::new(dir.path()).unwrap();
            let before = e.settings();
            let mut settings = crate::store::Settings { system_proxy: false, ..Default::default() };
            if with_secrets {
                settings.core.secret = "old-secret".into();
                settings.core.local_password = "old-password".into();
                settings.device_id = "old-device".into();
            }
            let b = crate::backup::Backup { kova: "0.0.0".into(), created: 1, settings, profiles: vec![] };
            let text = crate::backup::write(&b, None).unwrap();
            e.import_backup(crate::backup::read(&text, None).unwrap(), true).await.unwrap();
            let after = e.settings();
            assert_eq!(after.core.secret, before.core.secret);
            assert_eq!(after.core.local_password, before.core.local_password);
            assert_eq!(after.device_id, before.device_id);
        }
    }

    /// Disconnect does not wait for a connect that is stuck: the connect gives up quietly.
    #[tokio::test]
    async fn a_disconnect_cancels_a_connect_in_progress() {
        use std::time::Duration;
        let dir = tempfile::tempdir().unwrap();
        let e = Engine::new(dir.path()).unwrap();
        let task = tokio::spawn({
            let e = e.clone();
            async move { e.start_with(e.stop_epoch(), |_| std::future::pending::<crate::Result<(super::Running, super::Plan)>>()).await }
        });
        tokio::time::timeout(Duration::from_secs(5), async {
            while !e.status().busy {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("the connect begins");
        tokio::time::timeout(Duration::from_secs(5), e.stop()).await.expect("disconnect does not wait").unwrap();
        tokio::time::timeout(Duration::from_secs(5), task).await.expect("the connect ends").unwrap().expect("quietly");
        let st = e.status();
        assert!(!st.busy && !st.running && st.error.is_none(), "{st:?}");
        assert!(e.run.lock().await.is_none());
    }

    /// A connect that began before a disconnect does not run after it (this is what keeps a
    /// disconnect in a restart's gap from being undone: `restart_at` hands its epoch to
    /// `start_at`), and a restart with nothing running starts nothing. A real restart
    /// needs a running core, so the gap itself is not exercised here.
    #[tokio::test]
    async fn a_connect_that_began_before_a_disconnect_does_not_run_after_it() {
        let dir = tempfile::tempdir().unwrap();
        let e = Engine::new(dir.path()).unwrap();
        // Without a profile any real start would fail with an error, so Ok means none began.
        let epoch = e.stop_epoch();
        e.stop().await.unwrap();
        e.restart_at(epoch).await.expect("no start, no error");
        e.start_at(epoch).await.expect("no start, no error");
        let st = e.status();
        assert!(!st.busy && !st.running && st.error.is_none(), "{st:?}");
        // From the current count on, a start really starts (and fails: no profile).
        assert!(e.start().await.is_err());
    }
}

#[cfg(test)]
mod suggestion_tests {
    use kova_core::options::{PROXY, Rule, RuleKind};
    use kova_core::smart::{Origin, Route, Suggestion};

    use super::Engine;

    #[tokio::test]
    async fn an_accepted_suggestion_goes_right_above_the_users_rule() {
        let dir = tempfile::tempdir().unwrap();
        let e = Engine::new(dir.path()).unwrap();
        let rule = |kind, value: &str, target: &str| Rule { kind, value: value.into(), target: target.into() };
        let mut s = e.settings();
        s.system_proxy = false;
        s.core.custom_rules = vec![rule(RuleKind::DomainSuffix, "a.example", PROXY), rule(RuleKind::Geosite, "category-games", "DIRECT")];
        e.update_settings(s).await.unwrap();
        e.smart.suggest(Suggestion { domain: "modrinth.com".into(), route: Route::Proxy, reason: "reset".into(), rule: "GeoSite(category-games)".into(), origin: Origin::User, at: 0 });
        assert_eq!(e.smart_suggestions().len(), 1);

        e.accept_suggestion("modrinth.com").await.unwrap();
        let r = e.settings().core.custom_rules;
        let order: Vec<&str> = r.iter().map(|x| x.value.as_str()).collect();
        assert_eq!(order, ["a.example", "modrinth.com", "category-games"]);
        assert_eq!(r[1].target, PROXY);
        assert!(e.smart_suggestions().is_empty());

        // A dismissed site is not suggested again this session.
        e.smart.suggest(Suggestion { domain: "x.example".into(), route: Route::Proxy, reason: "reset".into(), rule: "GeoSite(category-ru)".into(), origin: Origin::Preset, at: 0 });
        e.dismiss_suggestion("x.example");
        assert!(e.smart_suggestions().is_empty());
    }
}
