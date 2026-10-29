//! Setting and restoring the OS proxy.
//!
//! Before Kova changes the proxy it records the previous state in a marker file. A
//! clean disconnect restores it and deletes the marker; on the next launch after a
//! crash, [`recover`] finds the marker and restores it then.

use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::Error;

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ProxyState {
    pub enabled: bool,
    pub server: String,
    pub bypass: String,
    /// The desktop's own settings exactly as they were, where `enabled`/`server` can't
    /// hold them (see [`native`]): an automatic (PAC) setup on Linux, each network service
    /// of a Mac. Restoring puts these back instead.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub saved: Vec<Vec<String>>,
}

/// Saving and restoring the desktop's own proxy settings, as lists of strings: a
/// `["gnome", schema, key, value]` per GNOME setting (the value as gsettings prints it),
/// a `["kde", key, value]` per KDE one, and per Mac network service a
/// `["mac", service, kind, "Yes"/"No", host, port]` for each proxy kind plus a
/// `["mac", service, "bypass", domain…]`. Only reading command output and building
/// commands lives here, so it is tested on every platform.
// Each desktop uses its own part; Windows none.
#[allow(dead_code)]
pub(crate) mod native {
    /// GNOME's settings that Kova changes or that decide how they are used.
    pub const GNOME: &[(&str, &str)] = &[
        ("org.gnome.system.proxy", "ignore-hosts"),
        ("org.gnome.system.proxy", "autoconfig-url"),
        ("org.gnome.system.proxy.http", "host"),
        ("org.gnome.system.proxy.http", "port"),
        ("org.gnome.system.proxy.https", "host"),
        ("org.gnome.system.proxy.https", "port"),
        ("org.gnome.system.proxy.socks", "host"),
        ("org.gnome.system.proxy.socks", "port"),
        // Last, so it switches over once the rest is back.
        ("org.gnome.system.proxy", "mode"),
    ];

    /// KDE's: the kind of setup (0 none, 1 manual, 2 script, 3 automatic, 4 system) and
    /// the manual addresses Kova overwrites.
    pub const KDE: &[&str] = &["httpProxy", "httpsProxy", "socksProxy", "ProxyType"];

    /// A Mac's proxy kinds, with the networksetup words that read, set and switch each.
    pub const MAC: &[(&str, &str, &str, &str)] = &[
        ("web", "-getwebproxy", "-setwebproxy", "-setwebproxystate"),
        ("secure", "-getsecurewebproxy", "-setsecurewebproxy", "-setsecurewebproxystate"),
        ("socks", "-getsocksfirewallproxy", "-setsocksfirewallproxy", "-setsocksfirewallproxystate"),
    ];

    /// `networksetup -getwebproxy` and its siblings: (enabled, host, port).
    pub fn mac_proxy(output: &str) -> (bool, String, String) {
        let field = |k: &str| output.lines().find_map(|l| l.strip_prefix(k)).map(|v| v.trim().to_string()).unwrap_or_default();
        (field("Enabled:") == "Yes", field("Server:"), field("Port:"))
    }

    /// `networksetup -getproxybypassdomains`: one domain per line, or a sentence when
    /// there are none.
    pub fn mac_bypass(output: &str) -> Vec<String> {
        output.lines().map(str::trim).filter(|l| !l.is_empty() && !l.contains(' ')).map(String::from).collect()
    }

    /// The networksetup commands that put a Mac's saved settings back.
    pub fn mac_restore(saved: &[Vec<String>]) -> Vec<Vec<String>> {
        let mut out = Vec::new();
        for e in saved.iter().filter(|e| e.first().is_some_and(|t| t == "mac")) {
            let [_, svc, kind, rest @ ..] = e.as_slice() else { continue };
            if kind == "bypass" {
                let mut cmd = vec!["-setproxybypassdomains".to_string(), svc.clone()];
                if rest.is_empty() {
                    cmd.push("Empty".into());
                } else {
                    cmd.extend(rest.iter().cloned());
                }
                out.push(cmd);
                continue;
            }
            let (Some((_, _, set, state)), [on, host, port]) = (MAC.iter().find(|m| m.0 == kind), rest) else { continue };
            // Setting an address also switches the proxy on.
            if !host.is_empty() {
                out.push(vec![set.to_string(), svc.clone(), host.clone(), port.clone()]);
            }
            if on != "Yes" || host.is_empty() {
                out.push(vec![state.to_string(), svc.clone(), "off".into()]);
            }
        }
        out
    }

    /// The gsettings commands that put GNOME's saved settings back, in the order saved.
    pub fn gnome_restore(saved: &[Vec<String>]) -> Vec<Vec<String>> {
        saved
            .iter()
            .filter_map(|e| match e.as_slice() {
                [t, schema, key, value] if t == "gnome" => Some(vec!["set".into(), schema.clone(), key.clone(), value.clone()]),
                _ => None,
            })
            .collect()
    }

    /// KDE's saved settings as (key, value), in the order saved.
    pub fn kde_restore(saved: &[Vec<String>]) -> Vec<(String, String)> {
        saved
            .iter()
            .filter_map(|e| match e.as_slice() {
                [t, key, value] if t == "kde" => Some((key.clone(), value.clone())),
                _ => None,
            })
            .collect()
    }
}

#[derive(Serialize, Deserialize)]
struct Marker {
    previous: ProxyState,
    ours: String,
}

pub fn enable(marker: &Path, host_port: &str, bypass: &str) -> Result<(), Error> {
    // Keep the original "previous" if we are re-applying over our own setting.
    let previous = match std::fs::read(marker).ok().and_then(|b| serde_json::from_slice::<Marker>(&b).ok()) {
        Some(m) => m.previous,
        None => imp::get()?,
    };
    let previous = not_ours(previous, host_port);
    let m = Marker { previous, ours: host_port.to_string() };
    crate::store::write_atomic(marker, &serde_json::to_vec(&m).expect("json"))?;
    #[cfg(windows)]
    imp::arm_restore(marker);
    imp::set(&ProxyState { enabled: true, server: host_port.to_string(), bypass: bypass.to_string(), saved: Vec::new() })
}

pub fn disable(marker: &Path) -> Result<(), Error> {
    let Some(m) = std::fs::read(marker).ok().and_then(|b| serde_json::from_slice::<Marker>(&b).ok()) else {
        return Ok(());
    };
    // Only undo our own change; if the user pointed the proxy elsewhere meanwhile, leave it.
    let now = imp::get()?;
    if now.enabled && now.server == m.ours {
        imp::set(&not_ours(m.previous, &m.ours))?;
    }
    let _ = std::fs::remove_file(marker);
    #[cfg(windows)]
    imp::disarm_restore();
    Ok(())
}

/// Give the proxy back on disconnect, like [`disable`], and also when the marker is gone
/// but the proxy still points at Kova (`host_port`): then it is turned off, as nothing
/// would answer there and the computer would have no internet.
pub fn release(marker: &Path, host_port: &str) -> Result<(), Error> {
    disable(marker)?;
    let now = imp::get()?;
    if now.enabled && now.server == host_port {
        imp::set(&ProxyState { enabled: false, saved: Vec::new(), ..now })?;
    }
    Ok(())
}

/// The state to give back, never Kova's own address: one saved while the proxy already
/// pointed at Kova (a marker lost, two Kovas at once) would leave it pointing at a port
/// nothing serves after disconnecting. That is turned off instead.
fn not_ours(previous: ProxyState, ours: &str) -> ProxyState {
    if previous.enabled && previous.server == ours { ProxyState { enabled: false, saved: Vec::new(), ..previous } } else { previous }
}

/// Whether the Kova that set the proxy is still serving it: something answers on its
/// port. A restore at sign-in then leaves it alone.
pub fn still_served(marker: &Path) -> bool {
    let Some(m) = std::fs::read(marker).ok().and_then(|b| serde_json::from_slice::<Marker>(&b).ok()) else { return false };
    let Ok(addr) = m.ours.parse::<std::net::SocketAddr>() else { return false };
    std::net::TcpStream::connect_timeout(&addr, std::time::Duration::from_millis(400)).is_ok()
}

/// Whether the system proxy is still the one Kova set (`host_port`). Another program
/// may have changed it while Kova runs.
pub fn is_ours(host_port: &str) -> bool {
    imp::get().is_ok_and(|now| now.enabled && now.server == host_port)
}

/// Call at startup: undo a proxy left behind by a crash.
pub fn recover(marker: &Path) -> Result<bool, Error> {
    if !marker.exists() {
        return Ok(false);
    }
    disable(marker)?;
    Ok(true)
}

pub fn current() -> Result<ProxyState, Error> {
    imp::get()
}

#[cfg(windows)]
mod imp {
    use super::ProxyState;
    use crate::Error;
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_WRITE};

    const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";
    const RUN_ONCE: &str = r"Software\Microsoft\Windows\CurrentVersion\RunOnce";
    const RESTORE: &str = "KovaRestoreProxy";

    /// Kova may not get to give the proxy back: Windows shutting down or the user
    /// signing out, a crash, ending it in Task Manager. Until Kova runs again the
    /// computer would have no internet, so while it holds the proxy Windows is asked to
    /// run `kova.exe --restore-proxy` once at the next sign-in; giving the proxy back
    /// takes the request away.
    pub fn arm_restore(marker: &std::path::Path) {
        let (Ok(exe), Some(root)) = (std::env::current_exe(), marker.parent().and_then(std::path::Path::parent)) else { return };
        if let Ok((k, _)) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN_ONCE) {
            let _ = k.set_value(RESTORE, &format!("\"{}\" --restore-proxy \"{}\"", exe.display(), root.display()));
        }
    }

    pub fn disarm_restore() {
        if let Ok(k) = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN_ONCE, KEY_READ | KEY_WRITE) {
            let _ = k.delete_value(RESTORE);
        }
    }

    pub fn get() -> Result<ProxyState, Error> {
        let k = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(KEY, KEY_READ)?;
        Ok(ProxyState {
            enabled: k.get_value::<u32, _>("ProxyEnable").unwrap_or(0) == 1,
            server: k.get_value("ProxyServer").unwrap_or_default(),
            bypass: k.get_value("ProxyOverride").unwrap_or_default(),
            saved: Vec::new(),
        })
    }

    pub fn set(s: &ProxyState) -> Result<(), Error> {
        let k = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(KEY, KEY_READ | KEY_WRITE)?;
        k.set_value("ProxyEnable", &(s.enabled as u32))?;
        k.set_value("ProxyServer", &s.server)?;
        k.set_value("ProxyOverride", &s.bypass)?;
        // Tell running WinINet clients (Edge, Chrome, most apps) to re-read the settings.
        unsafe {
            use windows_sys::Win32::Networking::WinInet::{INTERNET_OPTION_REFRESH, INTERNET_OPTION_SETTINGS_CHANGED, InternetSetOptionW};
            InternetSetOptionW(std::ptr::null(), INTERNET_OPTION_SETTINGS_CHANGED, std::ptr::null(), 0);
            InternetSetOptionW(std::ptr::null(), INTERNET_OPTION_REFRESH, std::ptr::null(), 0);
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
mod imp {
    use super::{ProxyState, native};
    use crate::Error;
    use std::process::Command;

    fn services() -> Vec<String> {
        let out = Command::new("networksetup").arg("-listallnetworkservices").output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
        out.lines().skip(1).filter(|l| !l.starts_with('*')).map(String::from).collect()
    }

    fn run(args: &[&str]) {
        let _ = Command::new("networksetup").args(args).status();
    }

    fn read(args: &[&str]) -> String {
        Command::new("networksetup").args(args).output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default()
    }

    /// The first service's web proxy (what `disable` compares with Kova's), and every
    /// service's settings to restore them one by one.
    pub fn get() -> Result<ProxyState, Error> {
        let services = services();
        let mut saved = Vec::new();
        for svc in &services {
            for (kind, get, _, _) in native::MAC {
                let (on, host, port) = native::mac_proxy(&read(&[get, svc]));
                saved.push(vec!["mac".into(), svc.clone(), kind.to_string(), if on { "Yes" } else { "No" }.into(), host, port]);
            }
            let mut bypass = vec!["mac".to_string(), svc.clone(), "bypass".into()];
            bypass.extend(native::mac_bypass(&read(&["-getproxybypassdomains", svc])));
            saved.push(bypass);
        }
        let Some(svc) = services.first() else { return Ok(ProxyState::default()) };
        let (enabled, host, port) = native::mac_proxy(&read(&["-getwebproxy", svc]));
        Ok(ProxyState { enabled, server: format!("{host}:{port}"), bypass: String::new(), saved })
    }

    pub fn set(s: &ProxyState) -> Result<(), Error> {
        if !s.saved.is_empty() {
            for cmd in native::mac_restore(&s.saved) {
                let args: Vec<&str> = cmd.iter().map(String::as_str).collect();
                run(&args);
            }
            return Ok(());
        }
        let (host, port) = s.server.rsplit_once(':').unwrap_or((&s.server, "0"));
        let bypass: Vec<&str> = s.bypass.split(';').filter(|b| !b.is_empty() && *b != "<local>").collect();
        for svc in services() {
            if s.enabled {
                run(&["-setwebproxy", &svc, host, port]);
                run(&["-setsecurewebproxy", &svc, host, port]);
                run(&["-setsocksfirewallproxy", &svc, host, port]);
                let mut args = vec!["-setproxybypassdomains", svc.as_str()];
                args.extend(bypass.iter().copied());
                run(&args);
            } else {
                run(&["-setwebproxystate", &svc, "off"]);
                run(&["-setsecurewebproxystate", &svc, "off"]);
                run(&["-setsocksfirewallproxystate", &svc, "off"]);
            }
        }
        Ok(())
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    //! GNOME (gsettings) and KDE (kwriteconfig) desktops.
    use super::{ProxyState, native};
    use crate::Error;
    use std::process::Command;

    /// A setting as gsettings prints it (`'auto'`, `['localhost']`, `8080`), which is
    /// also what `gsettings set` takes back.
    fn gs_raw(args: &[&str]) -> Option<String> {
        Command::new("gsettings").args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    }

    fn gs(args: &[&str]) -> Option<String> {
        gs_raw(args).map(|v| v.trim_matches('\'').to_string())
    }

    fn kde_read(key: &str) -> Option<String> {
        ["kreadconfig6", "kreadconfig5"].iter().find_map(|tool| {
            Command::new(tool)
                .args(["--file", "kioslaverc", "--group", "Proxy Settings", "--key", key])
                .output()
                .ok()
                .filter(|o| o.status.success())
                .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        })
    }

    /// KDE's setting, as `set` writes it: `ProxyType` 1 and `httpProxy` "http://host port".
    fn kde() -> Option<ProxyState> {
        let kind = kde_read("ProxyType")?;
        let http = kde_read("httpProxy").unwrap_or_default();
        let (host, port) = http.trim_start_matches("http://").split_once(' ').unwrap_or((http.as_str(), ""));
        Some(ProxyState { enabled: kind == "1", server: format!("{host}:{}", port.trim()), ..Default::default() })
    }

    /// Both desktops' settings as they are, to put back exactly: an automatic or script
    /// setup included, which `enabled` alone would turn into "none".
    fn snapshot() -> Vec<Vec<String>> {
        let mut saved = Vec::new();
        for (schema, key) in native::GNOME {
            if let Some(v) = gs_raw(&["get", schema, key]) {
                saved.push(vec!["gnome".into(), schema.to_string(), key.to_string(), v]);
            }
        }
        if kde_read("ProxyType").is_some() {
            for key in native::KDE {
                saved.push(vec!["kde".into(), key.to_string(), kde_read(key).unwrap_or_default()]);
            }
        }
        saved
    }

    pub fn get() -> Result<ProxyState, Error> {
        let saved = snapshot();
        // Without GNOME's settings (a plain KDE desktop) the state is KDE's; reading
        // nothing there would look like "off", and disconnecting would never undo Kova's.
        let Some(mode) = gs(&["get", "org.gnome.system.proxy", "mode"]) else {
            return Ok(ProxyState { saved, ..kde().unwrap_or_default() });
        };
        let host = gs(&["get", "org.gnome.system.proxy.http", "host"]).unwrap_or_default();
        let port = gs(&["get", "org.gnome.system.proxy.http", "port"]).unwrap_or_default();
        Ok(ProxyState { enabled: mode == "manual", server: format!("{host}:{port}"), bypass: String::new(), saved })
    }

    fn kde_write(key: &str, value: &str) -> bool {
        ["kwriteconfig6", "kwriteconfig5"]
            .iter()
            .any(|tool| Command::new(tool).args(["--file", "kioslaverc", "--group", "Proxy Settings", "--key", key, value]).status().is_ok_and(|st| st.success()))
    }

    pub fn set(s: &ProxyState) -> Result<(), Error> {
        if !s.saved.is_empty() {
            for cmd in native::gnome_restore(&s.saved) {
                let args: Vec<&str> = cmd.iter().map(String::as_str).collect();
                gs_raw(&args);
            }
            for (key, value) in native::kde_restore(&s.saved) {
                kde_write(&key, &value);
            }
            return Ok(());
        }
        let (host, port) = s.server.rsplit_once(':').unwrap_or((&s.server, "0"));
        if s.enabled {
            for scheme in ["http", "https", "socks"] {
                let schema = format!("org.gnome.system.proxy.{scheme}");
                gs(&["set", &schema, "host", host]);
                gs(&["set", &schema, "port", port]);
            }
            let ignore: Vec<String> = s.bypass.split(';').filter(|b| !b.is_empty() && *b != "<local>").map(|b| format!("'{b}'")).collect();
            gs(&["set", "org.gnome.system.proxy", "ignore-hosts", &format!("[{}]", ignore.join(", "))]);
            gs(&["set", "org.gnome.system.proxy", "mode", "manual"]);
        } else {
            gs(&["set", "org.gnome.system.proxy", "mode", "none"]);
        }
        for tool in ["kwriteconfig6", "kwriteconfig5"] {
            let ok = Command::new(tool)
                .args(["--file", "kioslaverc", "--group", "Proxy Settings", "--key", "ProxyType", if s.enabled { "1" } else { "0" }])
                .status()
                .is_ok_and(|st| st.success());
            if ok {
                if s.enabled {
                    for key in ["httpProxy", "httpsProxy", "socksProxy"] {
                        let _ = Command::new(tool).args(["--file", "kioslaverc", "--group", "Proxy Settings", "--key", key, &format!("http://{host} {port}")]).status();
                    }
                }
                break;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::native::*;
    use super::*;

    fn v(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn a_macs_services_are_put_back_one_by_one() {
        let on = "Enabled: Yes\nServer: corp.example\nPort: 3128\nAuthenticated Proxy Enabled: 0\n";
        assert_eq!(mac_proxy(on), (true, "corp.example".into(), "3128".into()));
        let never = "Enabled: No\nServer: \nPort: 0\nAuthenticated Proxy Enabled: 0\n";
        assert_eq!(mac_proxy(never), (false, String::new(), "0".into()));
        assert_eq!(mac_bypass("*.local\n169.254/16\n"), v(&["*.local", "169.254/16"]));
        assert!(mac_bypass("There aren't any bypass domains set on Wi-Fi.\n").is_empty());

        let saved = vec![
            v(&["mac", "Wi-Fi", "web", "No", "", "0"]),
            v(&["mac", "Wi-Fi", "secure", "No", "old.example", "8080"]),
            v(&["mac", "Wi-Fi", "bypass"]),
            v(&["mac", "USB LAN", "web", "Yes", "corp.example", "3128"]),
            v(&["mac", "USB LAN", "bypass", "*.local"]),
        ];
        assert_eq!(
            mac_restore(&saved),
            vec![
                v(&["-setwebproxystate", "Wi-Fi", "off"]),
                // The old address comes back, still switched off.
                v(&["-setsecurewebproxy", "Wi-Fi", "old.example", "8080"]),
                v(&["-setsecurewebproxystate", "Wi-Fi", "off"]),
                v(&["-setproxybypassdomains", "Wi-Fi", "Empty"]),
                // A service that had its own proxy on keeps it.
                v(&["-setwebproxy", "USB LAN", "corp.example", "3128"]),
                v(&["-setproxybypassdomains", "USB LAN", "*.local"]),
            ]
        );
    }

    #[test]
    fn an_automatic_linux_setup_comes_back_as_it_was() {
        let saved = vec![
            v(&["gnome", "org.gnome.system.proxy", "ignore-hosts", "['localhost', '127.0.0.0/8']"]),
            v(&["gnome", "org.gnome.system.proxy", "autoconfig-url", "'http://wpad.example/proxy.pac'"]),
            v(&["gnome", "org.gnome.system.proxy", "mode", "'auto'"]),
            v(&["kde", "httpProxy", ""]),
            v(&["kde", "ProxyType", "2"]),
            v(&["mac", "Wi-Fi", "bypass"]),
        ];
        assert_eq!(
            gnome_restore(&saved),
            vec![
                v(&["set", "org.gnome.system.proxy", "ignore-hosts", "['localhost', '127.0.0.0/8']"]),
                v(&["set", "org.gnome.system.proxy", "autoconfig-url", "'http://wpad.example/proxy.pac'"]),
                v(&["set", "org.gnome.system.proxy", "mode", "'auto'"]),
            ]
        );
        assert_eq!(kde_restore(&saved), vec![("httpProxy".to_string(), String::new()), ("ProxyType".to_string(), "2".to_string())]);
        assert_eq!(GNOME.last(), Some(&("org.gnome.system.proxy", "mode")), "the mode switches over last");
        assert_eq!(KDE.last(), Some(&"ProxyType"));
    }

    #[test]
    fn kovas_own_address_is_never_given_back() {
        let ours = "127.0.0.1:7890";
        let saved = ProxyState { enabled: true, server: ours.into(), bypass: "localhost".into(), saved: Vec::new() };
        let back = not_ours(saved, ours);
        assert!(!back.enabled);
        assert_eq!(back.bypass, "localhost");
        let other = ProxyState { enabled: true, server: "10.0.0.1:3128".into(), ..Default::default() };
        assert_eq!(not_ours(other.clone(), ours), other);
    }

    #[test]
    fn markers_from_before_keep_working() {
        // A marker written by an earlier Kova has no saved settings.
        let old: ProxyState = serde_json::from_str(r#"{"enabled":false,"server":":","bypass":""}"#).unwrap();
        assert!(old.saved.is_empty());
        let text = serde_json::to_string(&ProxyState::default()).unwrap();
        assert!(!text.contains("saved"), "nothing extra where there is nothing saved: {text}");
    }
}
