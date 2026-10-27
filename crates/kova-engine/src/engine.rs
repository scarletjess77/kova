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
use kova_core::options::{AUTO, FALLBACK, Mode, PROXY, Rule, RuleKind, RuntimeOptions, SmartRuntime, TEST_URL};
use kova_core::subscription::{self, Format};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use tokio::sync::{Mutex, broadcast};
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

/// Editable profile fields; `None` leaves a field as it is.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProfilePatch {
    pub name: Option<String>,
    pub core: Option<CorePreference>,
    pub url: Option<String>,
    pub user_agent: Option<String>,
    pub update_hours: Option<u32>,
    pub via_proxy: Option<bool>,
}

#[derive(Clone, Copy, PartialEq)]
enum Purpose {
    Main,
    /// A short-lived instance on spare ports used only to measure latency.
    Tester,
}

/// The core that owns the ports: a child process, or Mihomo inside the Kova service.
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
    front: Front,
    front_kind: CoreKind,
    sidecar: Option<CoreProcess>,
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
    /// The kill switch is holding: the connection dropped and the system proxy stays on
    /// until it is back or the user disconnects.
    holding: std::sync::atomic::AtomicBool,
    events: broadcast::Sender<Event>,
    me: Weak<Engine>,
}

fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0").and_then(|l| l.local_addr()).map(|a| a.port()).unwrap_or(0)
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

/// The certificate SHA-256 of each node whose link turns certificate checks off, asked
/// with `xray tls ping`, for Xray to pin (it no longer accepts "allowInsecure"). Best
/// effort and all at once: a server that doesn't answer gets no pin, and Xray then
/// checks its certificate normally.
async fn cert_pins<'a>(xray: &Path, nodes: impl Iterator<Item = &'a Node>) -> HashMap<String, String> {
    let pings = nodes.filter(|n| n.tls.as_ref().is_some_and(|t| t.insecure && t.reality.is_none())).map(|n| async move {
        let name = n.tls.as_ref().and_then(|t| t.sni.clone()).filter(|s| !s.is_empty()).unwrap_or_else(|| n.server.clone());
        // The server's address, when it isn't what the certificate is asked for.
        let ip = if name == n.server {
            None
        } else if let Ok(ip) = n.server.parse::<std::net::IpAddr>() {
            Some(ip)
        } else {
            tokio::net::lookup_host((n.server.as_str(), n.port)).await.ok().and_then(|mut a| a.next()).map(|a| a.ip())
        };
        let mut cmd = std::process::Command::new(xray);
        crate::process::hide_window(&mut cmd);
        cmd.args(["tls", "ping"]);
        if let Some(ip) = ip {
            cmd.args(["-ip", &ip.to_string()]);
        }
        let host = if name.contains(':') { format!("[{name}]") } else { name };
        cmd.arg(format!("{host}:{}", n.port));
        let out = tokio::time::timeout(Duration::from_secs(6), tokio::process::Command::from(cmd).kill_on_drop(true).output()).await.ok()?.ok()?;
        // The last one is the handshake made with the name.
        let text = String::from_utf8_lossy(&out.stdout);
        let pin = text.lines().filter(|l| l.contains("Cert's leaf SHA256:")).filter_map(|l| l.split_whitespace().last()).last()?.to_string();
        Some((n.name.clone(), pin))
    });
    futures_util::future::join_all(pings).await.into_iter().flatten().collect()
}

/// Fail early with a clear message instead of letting a core start without its listener.
fn port_available(addr: &str, port: u16) -> Result<()> {
    std::net::TcpListener::bind((addr, port)).map(drop).map_err(|_| {
        Error::Other(format!("Port {port} is already used by another program. Close it or choose another port in Settings"))
    })
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
    if let Some(start) = line.find('[') {
        if let Some(end) = line[start..].find(']') {
            let lvl = line[start + 1..start + end].to_ascii_lowercase();
            if ["debug", "info", "warning", "error"].contains(&lvl.as_str()) {
                return (lvl, line[start + end + 1..].trim().to_string());
            }
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
            holding: Default::default(),
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
        if !self.status().running {
            self.set_status(|s| s.mixed_port = new.core.mixed_port);
            return Ok(());
        }
        let mut core_changed = old.core != new.core;
        if core_changed && (RuntimeOptions { mode: new.core.mode, ..old.core.clone() }) == new.core {
            // Only the routing mode changed: Mihomo can switch live.
            let api = self.run.lock().await.as_ref().and_then(|r| r.api.clone());
            // The Android build of Mihomo refuses config changes over its API; then the
            // core restarts with the new mode like for any other change.
            if let Some(api) = api {
                core_changed = api.set_mode(new.core.mode).await.is_err();
            }
        }
        if core_changed {
            self.restart().await?;
        } else if old.system_proxy != new.system_proxy || old.proxy_bypass != new.proxy_bypass {
            self.apply_system_proxy()?;
        }
        Ok(())
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
            sysproxy::disable(&marker)
        }
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
            let sub = fetch::subscription(input, None, self.fetch_proxy(true).as_deref(), &self.settings().device_id).await?;
            let parsed = subscription::parse(&sub.body)?;
            let p = Profile {
                id: id.clone(),
                name: name.or(sub.title).unwrap_or_else(|| host_of(input)),
                source: Source::Remote { url: input.to_string(), user_agent: None, update_hours: sub.update_hours.unwrap_or(12), via_proxy: true },
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
        let Source::Remote { url, user_agent, via_proxy, .. } = &p.source else {
            return Ok(p);
        };
        let sub = fetch::subscription(url, user_agent.as_deref(), self.fetch_proxy(*via_proxy).as_deref(), &self.settings().device_id).await?;
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
        let _ = std::fs::remove_file(self.paths.profile_body(id));
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
        if self.profiles.read().unwrap().get(id).is_none() {
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
            if let Source::Remote { url, user_agent, update_hours, via_proxy } = &mut p.source {
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
        if self.settings().active_profile.as_deref() == Some(id) && self.status().running {
            self.restart().await?;
        }
        Ok(())
    }

    pub fn profile_nodes(&self, id: &str) -> Result<Vec<NodeView>> {
        let body = self.profile_content(id)?;
        Ok(subscription::parse(&body)?.nodes.iter().map(NodeView::of).collect())
    }

    fn active(&self) -> Result<(Profile, String, Vec<Node>)> {
        let id = self.settings().active_profile.ok_or_else(|| Error::Other("Add a profile first".into()))?;
        let p = self.profiles.read().unwrap().get(&id).cloned().ok_or_else(|| Error::Other("The active profile was removed. Choose another one".into()))?;
        let body = self.profile_content(&id)?;
        let nodes = subscription::parse(&body)?.nodes;
        Ok((p, body, nodes))
    }

    // ------------------------------------------------------------------ running

    pub async fn start(&self) -> Result<()> {
        let mut run = self.run.lock().await;
        if run.is_some() {
            return Ok(());
        }
        self.set_status(|s| {
            s.busy = true;
            s.error = None;
        });
        let opts = self.settings().core;
        match self.launch(Purpose::Main, opts.clone()).await {
            Ok((r, plan)) => {
                self.holding.store(false, std::sync::atomic::Ordering::Relaxed);
                let kind = r.front_kind;
                let sidecar = r.sidecar.is_some();
                let api = r.api.clone();
                let main_group = r.main_group.clone();
                // TUN can fail after launch decided to use it (another VPN holds the
                // adapter address, a driver problem); Mihomo then keeps running without it.
                let tun_error = self.tun_error.read().unwrap().clone();
                let tun_active = r.tun_active && tun_error.is_none();
                let warning = tun_error.or_else(|| r.warning.clone());
                *run = Some(r);
                drop(run);
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
                if let Some(api) = api {
                    // Mihomo's GLOBAL group starts on DIRECT, so Global mode would bypass the
                    // proxy. Point it at the main group; a choice the user made in GLOBAL is
                    // restored right after and wins.
                    let _ = api.select("GLOBAL", &main_group).await;
                    self.restore_selections(&api).await;
                    // Connected on mobile data: its own server, over the saved choice.
                    if self.on_mobile.load(std::sync::atomic::Ordering::Relaxed) {
                        if let Some(server) = self.settings().mobile_server {
                            let _ = api.select(&main_group, &server).await;
                        }
                    }
                }
                if let Err(e) = self.apply_system_proxy() {
                    self.emit(Event::Log { source: "kova".into(), level: "warning".into(), message: format!("could not set the system proxy: {e}") });
                }
                if !self.has_geodata() {
                    self.fetch_geodata_through_proxy();
                }
                Ok(())
            }
            Err(e) => {
                drop(run);
                // A failed restart must not leave the OS proxy pointing at a dead port,
                // unless the kill switch holds it there on purpose.
                if !self.holding.load(std::sync::atomic::Ordering::Relaxed) {
                    let _ = sysproxy::disable(&self.paths.proxy_marker());
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

    pub async fn stop(&self) -> Result<()> {
        // Disconnecting lifts the kill switch.
        self.holding.store(false, std::sync::atomic::Ordering::Relaxed);
        let taken = self.run.lock().await.take();
        self.set_status(|s| s.busy = true);
        let proxy = sysproxy::disable(&self.paths.proxy_marker());
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
        self.emit(Event::Traffic { up: 0, down: 0, up_total: 0, down_total: 0 });
        proxy
    }

    /// The configs written for the cores hold the servers' keys in plain text, which the
    /// cores need while they run; take them away once they stop. Kept: the cores' own
    /// caches next to them.
    fn remove_run_configs(&self) {
        for sub in ["main", "tester", "check"] {
            let dir = self.paths.runtime_dir().join(sub);
            for f in [dir.join("xray.json"), dir.join("xray-sidecar.json"), dir.join("config.yaml"), dir.join("mihomo").join("config.yaml")] {
                let _ = std::fs::remove_file(f);
            }
        }
    }

    /// Re-apply config. The system proxy stays pointed at us across the short gap.
    pub async fn restart(&self) -> Result<()> {
        if let Some(r) = self.run.lock().await.take() {
            r.shutdown().await;
        }
        self.set_status(|s| s.running = false);
        self.start().await
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
    /// times with growing pauses while the connection stays up.
    fn fetch_geodata_through_proxy(&self) {
        let me = self.me.clone();
        tokio::spawn(async move {
            for attempt in 1..=5u64 {
                let Some(e) = me.upgrade() else { return };
                if !e.status().running {
                    return;
                }
                match e.refresh_geodata_files(false).await {
                    Ok(_) => {
                        let _ = e.restart().await;
                        e.emit(Event::Notice { text: "Routing data downloaded through the server; every rule works now".into() });
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
        let max_age = crate::geodata::max_age(&source);
        match crate::geodata::marker(&self.paths.cores_dir()) {
            // Files that shipped with Xray are fine for the default source.
            None => source != kova_core::options::GeoSource::Default,
            Some(m) => m.source != source.id() || now() - m.updated_at > max_age,
        }
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
        opts.controller_port = free_port();
        opts.xray_metrics_port = free_port();
        let mut warning = None;
        if quiet {
            opts.mixed_port = free_port();
            opts.tun.enabled = false;
            opts.mode = Mode::Rule;
            opts.without_geodata = !self.has_geodata();
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
            Plan { front: CoreKind::Mihomo, bridged: Vec::new(), dropped: Vec::new() }
        } else {
            compat::plan(&nodes, core)
        };
        // Testing always runs through Mihomo, which can measure every node it runs.
        if quiet && plan.front == CoreKind::Xray {
            plan = compat::plan(&nodes, CorePreference::Auto);
        }
        let xray_bin = cores::binary_path(&cores_dir, CoreKind::Xray);
        #[allow(unused_mut)]
        let mut mihomo_bin = cores::binary_path(&cores_dir, CoreKind::Mihomo);
        let asset_env: [(&str, &Path); 1] = [("XRAY_LOCATION_ASSET", cores_dir.as_path())];

        if plan.front == CoreKind::Xray {
            if opts.tun.enabled {
                opts.tun.enabled = false;
                warning = Some("TUN mode needs the Mihomo core; this profile runs on Xray only".to_string());
            }
            if !quiet {
                let socks = opts.mixed_port.checked_add(1).ok_or_else(|| Error::Other("Xray profiles also use the port above the proxy port; choose a proxy port below 65535".into()))?;
                port_available(opts.listen_addr(), socks)?;
            }
            let selection = match self.settings().xray_selection {
                Some(n) => Selection::Node(n),
                None => Selection::Auto,
            };
            opts.cert_pins = cert_pins(&xray_bin, nodes.iter()).await;
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
                front: Front::Local(front),
                front_kind: CoreKind::Xray,
                sidecar: None,
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
        let mut via_service = !quiet && std::env::var_os("KOVA_FORCE_SERVICE").is_some();
        // Android: TUN is the VPN interface the system gave Kova, which needs no rights.
        let tun_fd = if quiet { None } else { *self.tun_fd.lock().unwrap() };
        if cfg!(target_os = "android") {
            opts.tun.enabled = tun_fd.is_some();
        }
        if let Some(fd) = tun_fd {
            opts.tun.enabled = true;
            opts.tun_fd = Some(fd);
        } else if opts.tun.enabled && !crate::process::is_elevated() {
            #[cfg(target_os = "linux")]
            match crate::tun_linux::granted() {
                // The copy of Mihomo with network rights runs TUN.
                Some(granted) if !quiet => mihomo_bin = granted,
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
                let bridged: Vec<(&Node, u16)> = nodes.iter().filter(|n| plan.bridged.contains(&n.name)).map(|n| (n, free_port())).collect();
                for (n, port) in &bridged {
                    bridges.insert(n.name.clone(), *port);
                }
                if anti_dpi {
                    opts.anti_dpi_port = Some(free_port());
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
                opts.cert_pins = cert_pins(&xray_bin, bridged.iter().map(|(n, _)| *n)).await;
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

        // Smart routing needs Kova's rules in the config; an imported Clash profile has
        // them only when the user applies them.
        if !quiet && opts.smart_routing && (profile.format != Format::Clash || opts.apply_rules_to_clash_profiles) {
            let lists_url = self.smart.lists_url().await?;
            // A subscription's own rules exist only in an imported Clash profile.
            let profile_rules = opts.smart_profile_rules && profile.format == Format::Clash;
            let scope = gen_mihomo::smart_scope(&opts, profile_rules);
            let password = opts.local_password.clone();
            opts.smart = Some(SmartRuntime { lists_url, direct_port: free_port(), proxy_port: free_port(), password, scope });
        }
        let home = run_dir.join("mihomo");
        std::fs::create_dir_all(&home)?;
        self.seed_geodata(&home);
        let yaml = if profile.format == Format::Clash {
            gen_mihomo::merge_clash(&body, &opts).map_err(|e| Error::Config(e.to_string()))?
        } else {
            gen_mihomo::from_nodes(&nodes, &plan, &bridges, &opts)
        };
        let cfg = home.join("config.yaml");
        std::fs::write(&cfg, &yaml)?;
        let (h, c) = (home.to_string_lossy().to_string(), cfg.to_string_lossy().to_string());
        let mut log_poller = None;
        let mut front = if via_service {
            use crate::service::{Request, call};
            // Only show log lines from this run.
            let since = call(&Request::Status, Duration::from_secs(5)).await?.log_seq;
            let tail = Arc::new(std::sync::Mutex::new(Vec::new()));
            log_poller = Some(self.spawn_service_logs(since, tail.clone()));
            // The first start may download Mihomo and the routing data into the service.
            let start = Request::Start { config: yaml, geodata: opts.geodata.clone(), owner_pid: std::process::id() };
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
        let mut r = Running {
            front,
            front_kind: CoreKind::Mihomo,
            sidecar,
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
        s.core.custom_rules.push(rule);
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
        if let Some(api) = api {
            if self.settings().core.smart_routing {
                crate::smart::refresh(&api).await;
            }
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
                                up_total = v["upTotal"].as_u64().unwrap_or(up_total + up);
                                down_total = v["downTotal"].as_u64().unwrap_or(down_total + down);
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
                    v["stats"]["inbound"].as_object().map(|m| m.values().map(|x| x[dir].as_u64().unwrap_or(0)).sum()).unwrap_or(0)
                };
                let (up_total, down_total) = (sum("uplink"), sum("downlink"));
                let up = last_up.map(|l| up_total.saturating_sub(l)).unwrap_or(0);
                let down = last_down.map(|l| down_total.saturating_sub(l)).unwrap_or(0);
                last_up = Some(up_total);
                last_down = Some(down_total);
                engine.emit(Event::Traffic { up, down, up_total, down_total });
                if let Some(obs) = v["observatory"].as_object() {
                    let delays = obs.iter().map(|(k, o)| (k.clone(), o["delay"].as_u64().filter(|_| o["alive"].as_bool().unwrap_or(false)).map(|d| d as u32))).collect::<HashMap<_, _>>();
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
                let mut guard = engine.run.lock().await;
                let died = match guard.as_mut() {
                    Some(r) => match r.front.exited().await {
                        Some(m) => Some(m),
                        None => r.sidecar.as_mut().and_then(|s| s.exited()),
                    },
                    None => return,
                };
                if let Some(msg) = died {
                    let r = guard.take();
                    drop(guard);
                    let kill_switch = engine.settings().kill_switch;
                    if kill_switch {
                        engine.holding.store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    // Clean up in a task of its own: shutdown() aborts every task in
                    // `r.tasks`, including this watchdog, which would cancel the cleanup.
                    tokio::spawn(async move {
                        if !kill_switch {
                            let _ = sysproxy::disable(&engine.paths.proxy_marker());
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
                            engine.reconnect_after_drop().await;
                        }
                    });
                    return;
                }
            }
        })
    }

    /// The kill switch: try to bring the connection back a few times while apps stay
    /// offline. Stops as soon as the user disconnects (which lifts the hold).
    async fn reconnect_after_drop(&self) {
        use std::sync::atomic::Ordering::Relaxed;
        for wait in [2, 5, 10] {
            tokio::time::sleep(Duration::from_secs(wait)).await;
            if !self.holding.load(Relaxed) || self.status().running {
                return;
            }
            if self.start().await.is_ok() {
                self.emit(Event::Notice { text: "The connection dropped and is back; the kill switch kept apps offline meanwhile".into() });
                return;
            }
        }
        if self.holding.load(Relaxed) {
            self.set_status(|s| s.error = Some("The connection dropped. The kill switch keeps apps offline until you connect again or disconnect".into()));
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
        let results: Vec<(String, Option<u32>)> = futures_util::stream::iter(names)
            .map(|n| {
                let api = api.clone();
                async move {
                    let d = api.delay(&n, TEST_URL, 5000).await.ok().flatten();
                    (n, d)
                }
            })
            .buffer_unordered(24)
            .collect()
            .await;
        if let Some(t) = tester {
            t.shutdown().await;
        }
        let map: HashMap<String, Option<u32>> = results.into_iter().collect();
        self.delays.write().unwrap().extend(map.clone());
        self.emit(Event::Delays { delays: map.clone() });
        Ok(map)
    }

    /// Android: the screen went off or on.
    pub fn set_quiet(&self, quiet: bool) {
        self.quiet.store(quiet, std::sync::atomic::Ordering::Relaxed);
    }

    /// Android: the phone moved between Wi-Fi and mobile data. With a server set for
    /// mobile data, switch to it there and back to the usual choice on Wi-Fi. Neither
    /// switch is saved as the user's choice, which stays the Wi-Fi one.
    pub async fn network_changed(&self, mobile: bool) {
        use std::sync::atomic::Ordering::Relaxed;
        if self.on_mobile.swap(mobile, Relaxed) == mobile {
            return;
        }
        let Some(server) = self.settings().mobile_server else { return };
        let (api, main) = {
            let run = self.run.lock().await;
            match run.as_ref() {
                Some(r) => (r.api.clone(), r.main_group.clone()),
                None => return,
            }
        };
        let Some(api) = api else { return };
        let target = if mobile {
            server
        } else {
            self.settings().selections.get(&self.selection_key(&main)).cloned().unwrap_or_else(|| AUTO.to_string())
        };
        if api.select(&main, &target).await.is_ok() {
            // Connections made on the other network keep their old route; start them over.
            let _ = api.close_all().await;
            let text = if mobile { format!("Mobile data: switched to {target}") } else { format!("Wi-Fi: back to {target}") };
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
        if api.delay(&chosen, TEST_URL, 5000).await.ok().flatten().is_some() {
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
                async move {
                    let d = api.delay(&n, TEST_URL, 5000).await.ok().flatten();
                    (n, d)
                }
            })
            .buffer_unordered(16)
            .collect()
            .await;
        let (best, _) = tested.into_iter().filter_map(|(n, d)| d.map(|d| (n, d))).min_by_key(|(_, d)| *d)?;
        self.select(&main, &best).await.ok()?;
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
        vec![m, x]
    }

    /// Install or update a core. Restarts the connection if that core is in use.
    pub async fn install_core(&self, core: CoreKind) -> Result<String> {
        let tag = cores::install_latest(&self.paths.cores_dir(), core).await?;
        let in_use = {
            let run = self.run.lock().await;
            run.as_ref().is_some_and(|r| r.front_kind == core || (core == CoreKind::Xray && r.sidecar.is_some()))
        };
        if in_use {
            self.restart().await?;
        }
        Ok(tag)
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
