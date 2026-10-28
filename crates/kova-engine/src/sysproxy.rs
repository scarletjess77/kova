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
    let m = Marker { previous, ours: host_port.to_string() };
    crate::store::write_atomic(marker, &serde_json::to_vec(&m).expect("json"))?;
    imp::set(&ProxyState { enabled: true, server: host_port.to_string(), bypass: bypass.to_string() })
}

pub fn disable(marker: &Path) -> Result<(), Error> {
    let Some(m) = std::fs::read(marker).ok().and_then(|b| serde_json::from_slice::<Marker>(&b).ok()) else {
        return Ok(());
    };
    // Only undo our own change; if the user pointed the proxy elsewhere meanwhile, leave it.
    let now = imp::get()?;
    if now.enabled && now.server == m.ours {
        imp::set(&m.previous)?;
    }
    let _ = std::fs::remove_file(marker);
    Ok(())
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

    pub fn get() -> Result<ProxyState, Error> {
        let k = RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(KEY, KEY_READ)?;
        Ok(ProxyState {
            enabled: k.get_value::<u32, _>("ProxyEnable").unwrap_or(0) == 1,
            server: k.get_value("ProxyServer").unwrap_or_default(),
            bypass: k.get_value("ProxyOverride").unwrap_or_default(),
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
    use super::ProxyState;
    use crate::Error;
    use std::process::Command;

    fn services() -> Vec<String> {
        let out = Command::new("networksetup").arg("-listallnetworkservices").output().map(|o| String::from_utf8_lossy(&o.stdout).to_string()).unwrap_or_default();
        out.lines().skip(1).filter(|l| !l.starts_with('*')).map(String::from).collect()
    }

    fn run(args: &[&str]) {
        let _ = Command::new("networksetup").args(args).status();
    }

    pub fn get() -> Result<ProxyState, Error> {
        let Some(svc) = services().into_iter().next() else { return Ok(ProxyState::default()) };
        let out = Command::new("networksetup").args(["-getwebproxy", &svc]).output()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let field = |k: &str| text.lines().find_map(|l| l.strip_prefix(k)).map(|v| v.trim().to_string()).unwrap_or_default();
        Ok(ProxyState { enabled: field("Enabled:") == "Yes", server: format!("{}:{}", field("Server:"), field("Port:")), bypass: String::new() })
    }

    pub fn set(s: &ProxyState) -> Result<(), Error> {
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
    use super::ProxyState;
    use crate::Error;
    use std::process::Command;

    fn gs(args: &[&str]) -> Option<String> {
        Command::new("gsettings").args(args).output().ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().trim_matches('\'').to_string())
    }

    /// KDE's setting, as `set` writes it: `ProxyType` 1 and `httpProxy` "http://host port".
    fn kde() -> Option<ProxyState> {
        let read = |key: &str| {
            ["kreadconfig6", "kreadconfig5"].iter().find_map(|tool| {
                Command::new(tool)
                    .args(["--file", "kioslaverc", "--group", "Proxy Settings", "--key", key])
                    .output()
                    .ok()
                    .filter(|o| o.status.success())
                    .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            })
        };
        let kind = read("ProxyType")?;
        let http = read("httpProxy").unwrap_or_default();
        let (host, port) = http.trim_start_matches("http://").split_once(' ').unwrap_or((http.as_str(), ""));
        Some(ProxyState { enabled: kind == "1", server: format!("{host}:{}", port.trim()), bypass: String::new() })
    }

    pub fn get() -> Result<ProxyState, Error> {
        // Without GNOME's settings (a plain KDE desktop) the state is KDE's; reading
        // nothing there would look like "off", and disconnecting would never undo Kova's.
        let Some(mode) = gs(&["get", "org.gnome.system.proxy", "mode"]) else {
            return Ok(kde().unwrap_or_default());
        };
        let host = gs(&["get", "org.gnome.system.proxy.http", "host"]).unwrap_or_default();
        let port = gs(&["get", "org.gnome.system.proxy.http", "port"]).unwrap_or_default();
        Ok(ProxyState { enabled: mode == "manual", server: format!("{host}:{port}"), bypass: String::new() })
    }

    pub fn set(s: &ProxyState) -> Result<(), Error> {
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
