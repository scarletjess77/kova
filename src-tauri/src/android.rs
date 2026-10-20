//! Android: the VPN. An Android app can capture traffic only through its VpnService,
//! which lives in the Kotlin part (gen/android/.../VpnPlugin.kt, KovaVpnService.kt).
//! Connecting asks the user for permission (once), has the service open the VPN
//! interface, and runs Mihomo on that interface.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use kova_engine::{Engine, Error};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use tauri::plugin::{Builder, PluginHandle, TauriPlugin};
use tauri::{AppHandle, Emitter, Manager, Wry};

struct Vpn(PluginHandle<Wry>);

pub fn plugin() -> TauriPlugin<Wry> {
    Builder::new("vpn")
        .setup(|app, api| {
            app.manage(Vpn(api.register_android_plugin("app.kova.client", "VpnPlugin")?));
            Ok(())
        })
        .build()
}

async fn call<T: DeserializeOwned>(app: &AppHandle, command: &str, payload: impl Serialize) -> Result<T, Error> {
    let vpn = app.state::<Vpn>();
    vpn.0.run_mobile_plugin_async(command, payload).await.map_err(|e| Error::Other(e.to_string()))
}

#[derive(Deserialize)]
struct Granted {
    granted: bool,
}

#[derive(Deserialize)]
struct Tunnel {
    fd: i32,
}

#[derive(Deserialize)]
struct Active {
    active: bool,
}

#[derive(Serialize)]
struct TunnelArgs {
    mtu: u32,
    ipv6: bool,
}

/// Connecting and disconnecting run one at a time: a second tap while the permission
/// dialog is up must not open a second interface.
static BUSY: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
/// Each connection's watcher stops when a newer connection starts.
static SESSION: AtomicU64 = AtomicU64::new(0);

pub async fn connect(app: &AppHandle, engine: &Arc<Engine>) -> Result<(), Error> {
    let _one = BUSY.lock().await;
    if engine.status().running {
        return Ok(());
    }
    let g: Granted = call(app, "prepare", ()).await?;
    if !g.granted {
        return Err(Error::Other("Kova needs permission to set up a VPN to connect".into()));
    }
    let s = engine.settings();
    let t: Tunnel = call(app, "establish", TunnelArgs { mtu: s.core.tun.mtu.into(), ipv6: s.core.ipv6 }).await?;
    // An interface still held from a connection that ended on its own is closed first.
    if let Some(old) = engine.take_tun_fd() {
        close_fd(old);
    }
    engine.set_tun_fd(Some(t.fd));
    if let Err(e) = engine.start().await {
        close(app, engine).await;
        return Err(e);
    }
    // Mihomo takes up the interface a moment after it starts. Should that fail, the
    // VPN would hold all traffic with nothing behind it: disconnect and say why.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let st = engine.status();
    if st.running && !st.tun_active {
        let why = st.warning.unwrap_or_else(|| "the VPN interface could not be used".into());
        let _ = shutdown(app, engine).await;
        return Err(Error::Core(why));
    }
    let session = SESSION.fetch_add(1, Ordering::SeqCst) + 1;
    watch(app.clone(), engine.clone(), session);
    Ok(())
}

pub async fn disconnect(app: &AppHandle, engine: &Arc<Engine>) -> Result<(), Error> {
    let _one = BUSY.lock().await;
    shutdown(app, engine).await
}

async fn shutdown(app: &AppHandle, engine: &Arc<Engine>) -> Result<(), Error> {
    let stopped = engine.stop().await;
    close(app, engine).await;
    stopped
}

fn close_fd(fd: i32) {
    unsafe {
        libc::close(fd);
    }
}

/// Close the interface and stop the service.
async fn close(app: &AppHandle, engine: &Arc<Engine>) {
    if let Some(fd) = engine.take_tun_fd() {
        close_fd(fd);
    }
    let _: Result<serde_json::Value, _> = call(app, "stop", ()).await;
}

/// The VPN can end without Kova being asked (another VPN app taking over, the user
/// turning Kova off in the system settings), and the core can stop on its own (a crash,
/// or a restart after a settings change that failed). Either way an interface with
/// nothing behind it would hold all traffic: notice it and turn the VPN off. Each sign
/// has to hold twice in a row, so a core restarting for a settings change doesn't count.
fn watch(app: AppHandle, engine: Arc<Engine>, session: u64) {
    tauri::async_runtime::spawn(async move {
        let (mut core_down, mut vpn_gone) = (0, 0);
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            if SESSION.load(Ordering::SeqCst) != session || !engine.has_tun_fd() {
                return;
            }
            let st = engine.status();
            core_down = if !st.running && !st.busy { core_down + 1 } else { 0 };
            vpn_gone = if matches!(call(&app, "status", ()).await, Ok(Active { active: false })) { vpn_gone + 1 } else { 0 };
            if core_down < 2 && vpn_gone < 2 {
                continue;
            }
            let _one = BUSY.lock().await;
            if SESSION.load(Ordering::SeqCst) != session {
                return;
            }
            let _ = shutdown(&app, &engine).await;
            let msg = if vpn_gone >= 2 {
                "The VPN was turned off by Android or another app".to_string()
            } else {
                format!("Kova stopped, so the VPN was turned off: {}", st.error.unwrap_or_else(|| "the core is not running".into()))
            };
            let _ = app.emit("toast", msg);
            return;
        }
    });
}
