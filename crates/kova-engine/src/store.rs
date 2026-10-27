//! On-disk state: settings, profiles and their contents.
//!
//! ```text
//! <root>/settings.json
//! <root>/profiles.json          index, in display order
//! <root>/profiles/<id>.txt      raw subscription body or local config
//! <root>/cores/                 mihomo, xray, geodata
//! <root>/runtime/               generated configs, Mihomo home (cache.db, geodata)
//! ```

use std::path::{Path, PathBuf};

use kova_core::compat::CorePreference;
use kova_core::options::RuntimeOptions;
use kova_core::subscription::{Format, UserInfo};
use serde::{Deserialize, Serialize};

use crate::Error;

#[derive(Debug, Clone)]
pub struct Paths {
    pub root: PathBuf,
}

impl Paths {
    pub fn new(root: impl Into<PathBuf>) -> std::io::Result<Self> {
        let p = Self { root: root.into() };
        for d in [p.profiles_dir(), p.cores_dir(), p.runtime_dir()] {
            std::fs::create_dir_all(d)?;
        }
        Ok(p)
    }
    pub fn settings(&self) -> PathBuf {
        self.root.join("settings.json")
    }
    pub fn profiles_index(&self) -> PathBuf {
        self.root.join("profiles.json")
    }
    pub fn profiles_dir(&self) -> PathBuf {
        self.root.join("profiles")
    }
    pub fn profile_body(&self, id: &str) -> PathBuf {
        self.profiles_dir().join(format!("{id}.txt"))
    }
    pub fn cores_dir(&self) -> PathBuf {
        self.root.join("cores")
    }
    pub fn runtime_dir(&self) -> PathBuf {
        self.root.join("runtime")
    }
    /// Sites smart routing learned.
    pub fn learned(&self) -> PathBuf {
        self.root.join("learned.json")
    }
    /// Marker written while Kova owns the system proxy, so a crash can be undone.
    pub fn proxy_marker(&self) -> PathBuf {
        self.runtime_dir().join("system-proxy.json")
    }
}

/// Write via a temp file and rename, so a crash never leaves half a file.
pub fn write_atomic(path: &Path, data: &[u8]) -> std::io::Result<()> {
    let tmp = temp_path(path);
    std::fs::write(&tmp, data)?;
    std::fs::rename(&tmp, path).inspect_err(|_| {
        let _ = std::fs::remove_file(&tmp);
    })
}

/// A temporary name beside `path` that no concurrent writer will also pick.
pub fn temp_path(path: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let n = NEXT.fetch_add(1, Ordering::Relaxed);
    let name = path.file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
    path.with_file_name(format!(".{name}.{}.{n}.tmp", std::process::id()))
}

pub(crate) fn read_json<T: for<'de> Deserialize<'de> + Default>(path: &Path) -> T {
    match std::fs::read(path) {
        Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
            tracing::warn!("{} is unreadable ({e}); using defaults", path.display());
            T::default()
        }),
        Err(_) => T::default(),
    }
}

pub(crate) fn write_json<T: Serialize>(path: &Path, v: &T) -> Result<(), Error> {
    write_atomic(path, &serde_json::to_vec_pretty(v).expect("serialisable"))?;
    Ok(())
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    System,
    Oled,
    Dim,
    Warm,
    Contrast,
    Nord,
    Catppuccin,
    Dracula,
    Gruvbox,
    Solarized,
    /// Last, because it also stands in for a theme a later version added, so the
    /// settings still load after going back to an older Kova.
    #[default]
    #[serde(other)]
    Dark,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub core: RuntimeOptions,
    pub system_proxy: bool,
    pub proxy_bypass: String,
    pub active_profile: Option<String>,
    pub connect_on_launch: bool,
    pub start_minimized: bool,
    pub launch_at_login: bool,
    pub auto_update_cores: bool,
    pub theme: Theme,
    /// Interface language: "system" (follow Windows), "en" or "ru".
    pub language: String,
    /// Node chosen for Xray-fronted profiles (`None` = Auto).
    pub xray_selection: Option<String>,
    /// Chosen member per `profile-id/group`, re-applied after every start.
    pub selections: std::collections::HashMap<String, String>,
    /// Sites the user added to the site check, as URLs.
    pub check_sites: Vec<String>,
    /// Look for a new Kova version on launch.
    pub check_updates: bool,
    /// Also update to beta versions (pre-releases). On by default in a beta build.
    pub beta_updates: bool,
    /// Android: which apps go through the VPN.
    pub vpn_apps: VpnApps,
    /// Move to the fastest working node when the chosen one stops answering.
    pub auto_switch: bool,
    /// A random value made once per install and sent to subscription servers, which
    /// count devices by it. Not derived from the hardware.
    pub device_id: String,
    /// Android: the server (or group) of the main group to use on mobile data, where
    /// whitelists apply; Wi-Fi keeps the one chosen as usual. `None`: the same on both.
    pub mobile_server: Option<String>,
}

/// Which apps the Android VPN covers, by package name.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct VpnApps {
    pub mode: VpnAppsMode,
    pub apps: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum VpnAppsMode {
    /// Every app.
    #[default]
    All,
    /// Only the listed apps.
    Only,
    /// Every app but the listed ones.
    Except,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            core: RuntimeOptions::default(),
            system_proxy: true,
            proxy_bypass: "localhost;127.*;10.*;172.16.*;172.17.*;172.18.*;172.19.*;172.2*;172.30.*;172.31.*;192.168.*;*.local;<local>".into(),
            active_profile: None,
            connect_on_launch: false,
            start_minimized: false,
            launch_at_login: false,
            auto_update_cores: true,
            theme: Theme::Dark,
            language: "system".into(),
            xray_selection: None,
            selections: Default::default(),
            check_sites: Vec::new(),
            check_updates: true,
            beta_updates: env!("CARGO_PKG_VERSION").contains('-'),
            vpn_apps: VpnApps::default(),
            auto_switch: true,
            device_id: String::new(),
            mobile_server: None,
        }
    }
}

impl Settings {
    pub fn load(paths: &Paths) -> Self {
        let mut s: Settings = read_json(&paths.settings());
        if s.core.secret.is_empty() || s.core.local_password.is_empty() || s.device_id.is_empty() {
            for v in [&mut s.core.secret, &mut s.core.local_password, &mut s.device_id] {
                if v.is_empty() {
                    *v = uuid::Uuid::new_v4().simple().to_string();
                }
            }
            let _ = s.save(paths);
        }
        s
    }
    pub fn save(&self, paths: &Paths) -> Result<(), Error> {
        write_json(&paths.settings(), self)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum Source {
    Remote {
        url: String,
        #[serde(default)]
        user_agent: Option<String>,
        /// Hours between automatic updates; 0 = off.
        #[serde(default = "twelve")]
        update_hours: u32,
        #[serde(default = "yes")]
        via_proxy: bool,
    },
    Local,
}

fn twelve() -> u32 {
    12
}
fn yes() -> bool {
    true
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub source: Source,
    pub format: Format,
    #[serde(default)]
    pub core: CorePreference,
    #[serde(default)]
    pub node_count: usize,
    #[serde(default)]
    pub usage: Option<UserInfo>,
    /// Unix seconds of the last successful update or edit.
    #[serde(default)]
    pub updated_at: i64,
    #[serde(default)]
    pub warnings: Vec<String>,
    #[serde(default)]
    pub support_url: Option<String>,
    /// The provider's latest message, from the last update.
    #[serde(default)]
    pub announce: Option<String>,
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct ProfileIndex {
    pub profiles: Vec<Profile>,
}

impl ProfileIndex {
    pub fn load(paths: &Paths) -> Self {
        read_json(&paths.profiles_index())
    }
    pub fn save(&self, paths: &Paths) -> Result<(), Error> {
        write_json(&paths.profiles_index(), self)
    }
    pub fn get(&self, id: &str) -> Option<&Profile> {
        self.profiles.iter().find(|p| p.id == id)
    }
    pub fn get_mut(&mut self, id: &str) -> Option<&mut Profile> {
        self.profiles.iter_mut().find(|p| p.id == id)
    }
}

pub fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::Theme;

    #[test]
    fn unknown_theme_loads_as_dark() {
        assert_eq!(serde_json::from_str::<Theme>("\"nord\"").unwrap(), Theme::Nord);
        assert_eq!(serde_json::from_str::<Theme>("\"dark\"").unwrap(), Theme::Dark);
        assert_eq!(serde_json::from_str::<Theme>("\"aurora\"").unwrap(), Theme::Dark);
    }
}
