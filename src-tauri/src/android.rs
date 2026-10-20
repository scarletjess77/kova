//! Android: the VPN. An Android app can capture traffic only through its VpnService,
//! which lives in the Kotlin part (gen/android/.../VpnPlugin.kt, KovaVpnService.kt).
//! Connecting asks the user for permission (once), has the service open the VPN
//! interface, and runs Mihomo on that interface.

use std::sync::Arc;
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

pub async fn connect(app: &AppHandle, engine: &Arc<Engine>) -> Result<(), Error> {
    if engine.status().running {
        return Ok(());
    }
    let g: Granted = call(app, "prepare", ()).await?;
    if !g.granted {
        return Err(Error::Other("Kova needs permission to set up a VPN to connect".into()));
    }
    let s = engine.settings();
    let t: Tunnel = call(app, "establish", TunnelArgs { mtu: s.core.tun.mtu.into(), ipv6: s.core.ipv6 }).await?;
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
        let _ = disconnect(app, engine).await;
        return Err(Error::Core(why));
    }
    watch(app.clone(), engine.clone());
    Ok(())
}

pub async fn disconnect(app: &AppHandle, engine: &Arc<Engine>) -> Result<(), Error> {
    let stopped = engine.stop().await;
    close(app, engine).await;
    stopped
}

/// Close the interface and stop the service.
async fn close(app: &AppHandle, engine: &Arc<Engine>) {
    if let Some(fd) = engine.take_tun_fd() {
        unsafe {
            libc::close(fd);
        }
    }
    let _: Result<serde_json::Value, _> = call(app, "stop", ()).await;
}

/// Android can end the VPN without asking Kova: another VPN app taking over, or the
/// user turning Kova off in the system settings. Notice it and disconnect cleanly.
fn watch(app: AppHandle, engine: Arc<Engine>) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(2)).await;
            if !engine.has_tun_fd() {
                return;
            }
            if let Ok(Active { active: false }) = call(&app, "status", ()).await {
                let _ = disconnect(&app, &engine).await;
                let _ = app.emit("toast", "The VPN was turned off by Android or another app");
                return;
            }
        }
    });
}
