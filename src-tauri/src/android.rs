//! Android: the VPN. An Android app can capture traffic only through its VpnService,
//! which lives in the Kotlin part (gen/android/.../VpnPlugin.kt, KovaVpnService.kt).
//! Connecting asks the user for permission (once), has the service open the VPN
//! interface, and runs Mihomo on that interface.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use kova_engine::store::VpnAppsMode;
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

/// What `call` answers while Kova has no window.
const NO_WINDOW: &str = "Kova's window is closed";

async fn call<T: DeserializeOwned>(app: &AppHandle, command: &str, payload: impl Serialize) -> Result<T, Error> {
    // A plugin call runs on Kova's window; with none (closed from the recent apps while
    // the VPN keeps Kova running) the webview library panics and the app dies.
    if !window_open() {
        return Err(Error::Other(NO_WINDOW.into()));
    }
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
    /// The DNS servers of the network the phone is on.
    #[serde(default)]
    dns: Vec<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Active {
    active: bool,
    #[serde(default = "yes")]
    screen_on: bool,
}

fn yes() -> bool {
    true
}

#[derive(Serialize)]
struct Details {
    node: String,
    speeds: String,
}

fn speed(bytes: u64) -> String {
    if bytes >= 1024 * 1024 {
        format!("{:.1} MB/s", bytes as f64 / 1024.0 / 1024.0)
    } else {
        format!("{} KB/s", bytes / 1024)
    }
}

#[derive(Serialize)]
struct TunnelArgs {
    mtu: u32,
    /// "all", "only" or "except", for `apps`.
    mode: &'static str,
    apps: Vec<String>,
    /// What the VPN's notification shows: the profile.
    label: String,
}

#[derive(Deserialize)]
struct Action {
    action: String,
}

/// Connecting and disconnecting run one at a time: a second tap while the permission
/// dialog is up must not open a second interface.
static BUSY: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
/// Each connection's watcher stops when a newer connection starts.
static SESSION: AtomicU64 = AtomicU64::new(0);
/// The node last shown in the notification.
static LAST_NODE: std::sync::Mutex<String> = std::sync::Mutex::new(String::new());

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
    let mode = match s.vpn_apps.mode {
        VpnAppsMode::All => "all",
        VpnAppsMode::Only => "only",
        VpnAppsMode::Except => "except",
    };
    let label = engine.profiles().into_iter().find(|p| Some(&p.id) == s.active_profile.as_ref()).map(|p| p.name).unwrap_or_default();
    let args = TunnelArgs { mtu: s.core.tun.mtu.into(), mode, apps: s.vpn_apps.apps.clone(), label };
    let t: Tunnel = call(app, "establish", args).await?;
    // An interface still held from a connection that ended on its own is closed first.
    if let Some(old) = engine.take_tun_fd() {
        close_fd(old);
    }
    engine.set_tun_fd(Some(t.fd));
    engine.set_network_dns(t.dns).await;
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

/// Whether Kova has a window now, asked from Android directly: `MainActivity.isOpen()`
/// through the app's class loader, which needs no window.
pub(crate) fn window_open() -> bool {
    use jni::objects::{GlobalRef, JClass, JObject, JValue};
    static CLASS: std::sync::OnceLock<Option<GlobalRef>> = std::sync::OnceLock::new();
    let ctx = ndk_context::android_context();
    let Ok(vm) = (unsafe { jni::JavaVM::from_raw(ctx.vm().cast()) }) else { return false };
    let Ok(mut env) = vm.attach_current_thread_as_daemon() else { return false };
    let class = CLASS.get_or_init(|| {
        let context = unsafe { JObject::from_raw(ctx.context().cast()) };
        let found = (|| {
            let loader = env.call_method(&context, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?.l()?;
            let name = env.new_string("app.kova.client.MainActivity")?;
            let class = env.call_method(&loader, "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;", &[JValue::Object(&name)])?.l()?;
            env.new_global_ref(class)
        })();
        if found.is_err() {
            let _ = env.exception_clear();
        }
        found.ok()
    });
    let Some(class) = class else { return false };
    let class: &JClass = class.as_obj().into();
    match env.call_static_method(class, "isOpen", "()Z", &[]).and_then(|v| v.z()) {
        Ok(open) => open,
        Err(_) => {
            let _ = env.exception_clear();
            false
        }
    }
}

pub(crate) fn close_fd(fd: i32) {
    unsafe {
        libc::close(fd);
    }
}

/// End the process without running exit handlers, which crash with Android's threads
/// still running.
pub(crate) fn end_process() -> ! {
    unsafe { libc::_exit(0) }
}

/// Kova's window was closed while connected and Kova kept running for the VPN; now it is
/// opened again: make the page anew in the new window.
fn reopen_window(app: &AppHandle) -> Result<(), Error> {
    if app.get_webview_window("main").is_some() {
        return Ok(());
    }
    let config = app.config().app.windows.first().cloned().ok_or_else(|| Error::Other("no window in the config".into()))?;
    tauri::WebviewWindowBuilder::from_config(app, &config).and_then(|w| w.build()).map_err(|e| Error::Other(e.to_string()))?;
    tracing::info!("window opened again");
    Ok(())
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
        let (mut core_down, mut vpn_gone, mut quiet, mut tick) = (0, 0, false, 0u32);
        engine.set_quiet(false);
        let mut events = engine.subscribe();
        let (mut up, mut down) = (0, 0);
        loop {
            // Less often while the screen is off: nobody is looking, and it saves battery.
            tokio::time::sleep(Duration::from_secs(if quiet { 6 } else { 2 })).await;
            if SESSION.load(Ordering::SeqCst) != session || !engine.has_tun_fd() {
                return;
            }
            while let Ok(ev) = events.try_recv() {
                if let kova_engine::Event::Traffic { up: u, down: d, .. } = ev {
                    (up, down) = (u, d);
                }
            }
            let st = engine.status();
            core_down = if !st.running && !st.busy { core_down + 1 } else { 0 };
            let state: Option<Active> = call(&app, "status", ()).await.ok();
            vpn_gone = if matches!(state, Some(Active { active: false, .. })) { vpn_gone + 1 } else { 0 };
            if let Some(a) = &state
                && quiet == a.screen_on
            {
                quiet = !a.screen_on;
                engine.set_quiet(quiet);
            }
            // The notification shows the node and the speeds while someone can see it.
            tick += 1;
            if !quiet && st.running && core_down == 0 && vpn_gone == 0 {
                let node = if tick % 5 == 1 { engine.current_node().await } else { None };
                let details = Details { node: node.unwrap_or_else(|| LAST_NODE.lock().unwrap().clone()), speeds: format!("↓ {}  ↑ {}", speed(down), speed(up)) };
                *LAST_NODE.lock().unwrap() = details.node.clone();
                let _: Result<serde_json::Value, _> = call(&app, "details", details).await;
            }
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

/// The apps installed on the phone that can use the network, for choosing which go
/// through the VPN: package, label, icon (a data URL) and whether it is a system app.
pub async fn apps(app: &AppHandle) -> Result<serde_json::Value, Error> {
    call(app, "apps", ()).await
}

/// Android's VPN settings, where Kova can be made the always-on VPN.
pub async fn open_vpn_settings(app: &AppHandle) -> Result<(), Error> {
    call::<serde_json::Value>(app, "openVpnSettings", ()).await.map(|_| ())
}

/// Requests from outside Kova's window: the Quick Settings tile, the notification's
/// Disconnect button, and Android starting the VPN on its own (always-on). The Kotlin
/// side answers a waiting call when one comes, so this keeps one call waiting.
pub fn listen_for_actions(app: AppHandle, engine: Arc<Engine>) {
    tauri::async_runtime::spawn(async move {
        loop {
            let a: Action = match call(&app, "nextAction", ()).await {
                Ok(a) => a,
                Err(e) => {
                    // No window: requests wait in Kova's Kotlin side until it is back.
                    if e.to_string() != NO_WINDOW {
                        tracing::warn!("waiting for VPN actions failed: {e}");
                    }
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }
            };
            let done = match a.action.as_str() {
                "connect" => connect(&app, &engine).await,
                "disconnect" => disconnect(&app, &engine).await,
                "network:mobile" | "network:wifi" => {
                    engine.network_changed(a.action == "network:mobile").await;
                    Ok(())
                }
                "network:back" => {
                    engine.network_restored().await;
                    Ok(())
                }
                "window" => reopen_window(&app),
                dns if dns.starts_with("dns:") => {
                    engine.set_network_dns(dns[4..].split(',').map(String::from).collect()).await;
                    Ok(())
                }
                _ => Ok(()),
            };
            if let Err(e) = done {
                let _ = app.emit("toast", e.to_string());
            }
        }
    });
}

#[derive(Serialize)]
struct ApkArgs {
    path: String,
}

/// Hand a downloaded APK to Android's installer. The first time, Android asks to let
/// Kova install apps; the settings page for that opens instead and this fails with a
/// note to try again.
pub async fn install_apk(app: &AppHandle, path: &std::path::Path) -> Result<(), Error> {
    call::<serde_json::Value>(app, "installApk", ApkArgs { path: path.to_string_lossy().into() }).await.map(|_| ())
}

/// Whether this is a TV, whether it has a camera, whether battery saving leaves Kova
/// alone, and the maker.
pub async fn device(app: &AppHandle) -> Result<serde_json::Value, Error> {
    call(app, "device", ()).await
}

/// Hide or show the status and navigation bars.
pub async fn full_screen(app: &AppHandle, on: bool) -> Result<(), Error> {
    call::<serde_json::Value>(app, "fullScreen", serde_json::json!({ "on": on })).await.map(|_| ())
}

/// Ask to be left out of battery saving.
pub async fn allow_background(app: &AppHandle) -> Result<(), Error> {
    call::<serde_json::Value>(app, "allowBackground", ()).await.map(|_| ())
}

/// The maker's autostart settings, or Kova's app settings.
pub async fn open_autostart(app: &AppHandle) -> Result<(), Error> {
    call::<serde_json::Value>(app, "openAutostart", ()).await.map(|_| ())
}

#[derive(Serialize)]
struct DocumentArgs {
    name: String,
    text: String,
}

#[derive(Deserialize)]
struct Saved {
    saved: bool,
}

#[derive(Deserialize)]
struct Opened {
    #[serde(default)]
    text: Option<String>,
}

/// Save `text` as `name` where the user picks; false if they cancel.
pub async fn save_document(app: &AppHandle, name: &str, text: &str) -> Result<bool, Error> {
    call::<Saved>(app, "saveDocument", DocumentArgs { name: name.into(), text: text.into() }).await.map(|s| s.saved)
}

/// The text of a file the user picks; `None` if they cancel.
pub async fn open_document(app: &AppHandle) -> Result<Option<String>, Error> {
    call::<Opened>(app, "openDocument", ()).await.map(|o| o.text)
}
