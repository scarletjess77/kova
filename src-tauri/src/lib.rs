//! The app shell: exposes the engine to the UI as commands and events. The tray and
//! window live in `desktop`, the Android VPN in `android`.

#[cfg(target_os = "android")]
mod android;
mod backup;
#[cfg(desktop)]
mod desktop;
mod i18n;
mod links;
mod logfile;
mod receive;
mod updates;

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use kova_core::CoreKind;
use kova_engine::cores::CoreStatus;
use kova_engine::engine::ProfilePatch;
use kova_engine::mihomo_api::{ConnectionsSnapshot, ProxySnapshot};
use kova_engine::store::{Profile, Settings};
use kova_engine::{Engine, Error, Event, NodeView, Status};
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State};

use crate::i18n::tr;

type Eng<'a> = State<'a, Arc<Engine>>;
type R<T> = Result<T, Error>;

/// Keep what shows outside the window (the desktop tray) in step with a change.
fn refresh_ui(app: &AppHandle) {
    #[cfg(desktop)]
    desktop::refresh_tray(app);
    #[cfg(not(desktop))]
    let _ = app;
}

/// Connect: on Android through the system VPN, elsewhere directly.
async fn start(app: &AppHandle, engine: &Arc<Engine>) -> R<()> {
    #[cfg(target_os = "android")]
    return android::connect(app, engine).await;
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        engine.start().await
    }
}

async fn stop(app: &AppHandle, engine: &Arc<Engine>) -> R<()> {
    #[cfg(target_os = "android")]
    return android::disconnect(app, engine).await;
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        engine.stop().await
    }
}

// ---------------------------------------------------------------------------- commands

#[derive(Serialize)]
struct Snapshot {
    status: Status,
    settings: Settings,
    profiles: Vec<Profile>,
    data_dir: String,
}

#[tauri::command]
fn get_state(engine: Eng) -> Snapshot {
    Snapshot { status: engine.status(), settings: engine.settings(), profiles: engine.profiles(), data_dir: engine.paths.root.display().to_string() }
}

/// The system's display language, for the "Match system" language setting.
#[tauri::command]
fn system_language() -> String {
    i18n::system_language()
}

/// The operating system ("windows", "android", ...), so the UI can leave out what
/// doesn't exist there.
#[tauri::command]
fn platform() -> &'static str {
    std::env::consts::OS
}

#[tauri::command]
fn learned_sites(engine: Eng) -> Vec<kova_core::smart::Learned> {
    engine.learned_sites()
}

#[tauri::command]
async fn forget_learned(engine: Eng<'_>, domain: String) -> R<()> {
    engine.forget_learned(&domain).await
}

#[tauri::command]
async fn keep_learned(app: AppHandle, engine: Eng<'_>, domain: String) -> R<Settings> {
    engine.keep_learned(&domain).await?;
    refresh_ui(&app);
    Ok(engine.settings())
}

#[tauri::command]
async fn clear_learned(engine: Eng<'_>) -> R<()> {
    engine.clear_learned().await
}

#[tauri::command]
fn smart_suggestions(engine: Eng) -> Vec<kova_core::smart::Suggestion> {
    engine.smart_suggestions()
}

#[tauri::command]
async fn accept_suggestion(app: AppHandle, engine: Eng<'_>, domain: String) -> R<Settings> {
    engine.accept_suggestion(&domain).await?;
    refresh_ui(&app);
    Ok(engine.settings())
}

#[tauri::command]
fn dismiss_suggestion(engine: Eng, domain: String) {
    engine.dismiss_suggestion(&domain);
}

/// What kind of device: `{ tv, camera }`. Only Android has TVs and cameras Kova uses.
#[tauri::command]
async fn device_info(app: AppHandle) -> serde_json::Value {
    #[cfg(target_os = "android")]
    if let Ok(v) = android::device(&app).await {
        return v;
    }
    let _ = app;
    serde_json::json!({ "tv": false, "camera": false })
}

/// Why isn't it working? Step by step; see `kova_engine::diag`.
#[tauri::command]
async fn diagnose(engine: Eng<'_>) -> R<Vec<kova_engine::diag::Check>> {
    Ok(engine.diagnose().await)
}

#[tauri::command]
async fn privacy_check(engine: Eng<'_>) -> R<Vec<kova_engine::diag::Check>> {
    Ok(engine.privacy_check().await)
}

#[derive(Serialize)]
struct Speeds {
    through_kova: Option<f64>,
    direct: Option<f64>,
}

#[tauri::command]
async fn speed_test(engine: Eng<'_>) -> R<Speeds> {
    let (through_kova, direct) = engine.speed_test().await;
    Ok(Speeds { through_kova, direct })
}

/// This device's address on the local network, for sharing the proxy.
#[tauri::command]
fn lan_address() -> Option<String> {
    receive::lan_ip().map(|ip| ip.to_string())
}

/// Serve the "add from another device" page; see `receive`.
#[tauri::command]
async fn receive_start(app: AppHandle, engine: Eng<'_>) -> R<receive::Receiving> {
    let ru = i18n::lang(&engine.settings()) == i18n::Lang::Ru;
    receive::start(app, engine.inner().clone(), ru).await
}

#[tauri::command]
fn receive_stop() {
    receive::stop();
}

/// Android: the installed apps, for choosing which go through the VPN.
#[tauri::command]
async fn vpn_apps(app: AppHandle) -> R<serde_json::Value> {
    #[cfg(target_os = "android")]
    return android::apps(&app).await;
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Err(Error::Other("Only on Android".into()))
    }
}

/// Android: open the system's VPN settings (always-on VPN).
#[tauri::command]
async fn open_vpn_settings(app: AppHandle) -> R<()> {
    #[cfg(target_os = "android")]
    return android::open_vpn_settings(&app).await;
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Err(Error::Other("Only on Android".into()))
    }
}

#[tauri::command]
async fn connect(app: AppHandle, engine: Eng<'_>) -> R<()> {
    start(&app, engine.inner()).await
}

#[tauri::command]
async fn disconnect(app: AppHandle, engine: Eng<'_>) -> R<()> {
    stop(&app, engine.inner()).await
}

#[tauri::command]
async fn save_settings(app: AppHandle, engine: Eng<'_>, settings: Settings) -> R<Settings> {
    engine.update_settings(settings).await?;
    refresh_ui(&app);
    Ok(engine.settings())
}

#[tauri::command]
async fn add_profile(engine: Eng<'_>, input: String, name: Option<String>) -> R<Profile> {
    engine.add_profile(&input, name.filter(|n| !n.trim().is_empty())).await
}

#[tauri::command]
async fn update_profile(engine: Eng<'_>, id: String) -> R<Profile> {
    engine.update_profile(&id).await
}

#[tauri::command]
async fn update_all_profiles(engine: Eng<'_>) -> R<Vec<(String, String)>> {
    Ok(engine.update_all().await)
}

#[tauri::command]
async fn remove_profile(engine: Eng<'_>, id: String) -> R<()> {
    engine.remove_profile(&id).await
}

#[tauri::command]
async fn set_active_profile(engine: Eng<'_>, id: String) -> R<()> {
    engine.set_active_profile(&id).await
}

#[tauri::command]
async fn edit_profile(engine: Eng<'_>, id: String, patch: serde_json::Value) -> R<Profile> {
    let patch: ProfilePatch = serde_json::from_value(patch).map_err(|e| Error::Other(e.to_string()))?;
    engine.edit_profile(&id, patch).await
}

#[tauri::command]
fn get_profile_content(engine: Eng, id: String) -> R<String> {
    engine.profile_content(&id)
}

#[tauri::command]
async fn save_profile_content(engine: Eng<'_>, id: String, text: String) -> R<()> {
    engine.save_profile_content(&id, &text).await
}

#[tauri::command]
fn profile_nodes(engine: Eng, id: String) -> R<Vec<NodeView>> {
    engine.profile_nodes(&id)
}

#[tauri::command]
async fn get_proxies(engine: Eng<'_>) -> R<ProxySnapshot> {
    engine.proxies().await
}

#[tauri::command]
async fn select_proxy(app: AppHandle, engine: Eng<'_>, group: String, name: String) -> R<()> {
    engine.select(&group, &name).await?;
    refresh_ui(&app);
    Ok(())
}

#[tauri::command]
async fn check_quality(engine: Eng<'_>) -> R<kova_engine::quality::Quality> {
    engine.check_quality().await
}

#[tauri::command]
async fn test_delays(engine: Eng<'_>, names: Option<Vec<String>>) -> R<HashMap<String, Option<u32>>> {
    engine.test_delays(names).await
}

#[tauri::command]
async fn get_connections(engine: Eng<'_>) -> R<ConnectionsSnapshot> {
    engine.connections().await
}

#[tauri::command]
async fn close_connection(engine: Eng<'_>, id: String) -> R<()> {
    engine.close_connection(&id).await
}

#[tauri::command]
async fn close_all_connections(engine: Eng<'_>) -> R<()> {
    engine.close_all_connections().await
}

#[tauri::command]
async fn core_status(engine: Eng<'_>, check_latest: bool) -> R<Vec<CoreStatus>> {
    Ok(engine.core_status(check_latest).await)
}

#[tauri::command]
async fn install_core(app: AppHandle, engine: Eng<'_>, core: CoreKind) -> R<String> {
    let tag = engine.install_core(core).await?;
    let _ = app.emit("cores-changed", ());
    Ok(tag)
}

#[derive(Serialize)]
struct ServiceView {
    /// The platform has a Kova service at all.
    supported: bool,
    installed: bool,
    running: bool,
    version: Option<String>,
    app_version: String,
    core_version: Option<String>,
    /// The running service is a different build from this app (compared by executable
    /// hash, so same-version rebuilds count too).
    outdated: bool,
}

#[tauri::command]
async fn service_status(app: AppHandle) -> ServiceView {
    let app_version = env!("CARGO_PKG_VERSION").to_string();
    #[cfg(windows)]
    let _ = &app;
    #[cfg(windows)]
    {
        let state = tokio::task::spawn_blocking(kova_service::query).await.ok().flatten();
        let hello = kova_engine::service::hello().await;
        let own = tokio::task::spawn_blocking(kova_engine::service::own_exe_sha256).await.ok().flatten();
        let outdated = match (hello.as_ref().and_then(|h| h.exe_sha256.as_ref()), own) {
            (Some(theirs), Some(ours)) => *theirs != ours,
            // A service too old to report its hash is outdated by definition.
            (None, _) => hello.is_some(),
            _ => false,
        };
        ServiceView {
            supported: true,
            installed: state.is_some(),
            running: hello.is_some(),
            version: hello.as_ref().and_then(|h| h.version.clone()),
            app_version,
            core_version: hello.and_then(|h| h.core_version),
            outdated,
        }
    }
    #[cfg(target_os = "linux")]
    {
        let ours = kova_engine::cores::binary_path(&app.state::<Arc<Engine>>().paths.cores_dir(), kova_core::CoreKind::Mihomo);
        let granted = kova_engine::tun_linux::granted().is_some();
        let outdated = tokio::task::spawn_blocking(move || kova_engine::tun_linux::outdated(&ours)).await.unwrap_or(false);
        ServiceView { supported: true, installed: granted, running: granted, version: None, app_version, core_version: None, outdated }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = &app;
        ServiceView { supported: false, installed: false, running: false, version: None, app_version, core_version: None, outdated: false }
    }
}

/// Run `--service-install` / `--service-uninstall` elevated (one UAC prompt).
#[cfg(windows)]
async fn elevated(arg: &'static str) -> R<()> {
    let code = tokio::task::spawn_blocking(move || kova_service::run_elevated(arg)).await.map_err(|e| Error::Other(e.to_string()))?.map_err(Error::Other)?;
    if code == 0 {
        return Ok(());
    }
    Err(Error::Other(kova_service::describe_exit(code)))
}

#[tauri::command]
async fn service_install(engine: Eng<'_>) -> R<()> {
    #[cfg(windows)]
    {
        elevated("--service-install").await?;
        // Give the service a moment to open its pipe, then let TUN use it.
        for _ in 0..20 {
            if kova_engine::service::hello().await.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
        if engine.status().running && engine.settings().core.tun.enabled {
            engine.restart().await?;
        }
        Ok(())
    }
    #[cfg(target_os = "linux")]
    {
        let mihomo = kova_engine::cores::binary_path(&engine.paths.cores_dir(), kova_core::CoreKind::Mihomo);
        tokio::task::spawn_blocking(move || kova_engine::tun_linux::grant(&mihomo)).await.map_err(|e| Error::Other(e.to_string()))??;
        if engine.status().running && engine.settings().core.tun.enabled {
            engine.restart().await?;
        }
        Ok(())
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = engine;
        Err(Error::Other("The Kova service is only available on Windows for now".into()))
    }
}

#[tauri::command]
async fn service_uninstall(engine: Eng<'_>) -> R<()> {
    #[cfg(windows)]
    {
        // Disconnect first if the connection runs inside the service (TUN may even have
        // failed there, so tun_active alone doesn't tell).
        if engine.uses_service().await {
            engine.stop().await?;
        }
        elevated("--service-uninstall").await
    }
    #[cfg(target_os = "linux")]
    {
        if engine.status().running && engine.settings().core.tun.enabled {
            engine.stop().await?;
        }
        tokio::task::spawn_blocking(kova_engine::tun_linux::revoke).await.map_err(|e| Error::Other(e.to_string()))?
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        let _ = engine;
        Ok(())
    }
}

/// Programs running now, for choosing an app to route.
#[tauri::command]
async fn running_programs() -> Vec<String> {
    tokio::task::spawn_blocking(kova_engine::programs::running).await.unwrap_or_default()
}

#[tauri::command]
fn default_check_sites() -> Vec<kova_engine::sitecheck::Site> {
    kova_engine::sitecheck::default_sites()
}

#[tauri::command]
async fn check_site(engine: Eng<'_>, url: String) -> R<kova_engine::sitecheck::SiteResult> {
    let url = url.trim();
    if !(url.starts_with("http://") || url.starts_with("https://")) {
        return Err(Error::Other("Enter an address starting with https://".into()));
    }
    Ok(engine.check_site(url).await)
}

/// Off the main thread: the first look at the Russian lists reads ~90 MB.
#[tauri::command]
async fn geodata_info(engine: Eng<'_>) -> R<kova_engine::geodata::GeoInfo> {
    let engine = engine.inner().clone();
    tokio::task::spawn_blocking(move || engine.geodata_info()).await.map_err(|e| Error::Other(e.to_string()))
}

#[tauri::command]
async fn update_geodata(engine: Eng<'_>) -> R<kova_engine::geodata::Marker> {
    engine.update_geodata().await
}

#[derive(Serialize, Default)]
struct ExitInfo {
    ip: String,
    country: String,
    country_code: String,
    city: String,
    network: String,
}

/// Public IP as seen through the proxy (or directly when disconnected).
#[tauri::command]
async fn exit_info(engine: Eng<'_>) -> R<ExitInfo> {
    let mut b = kova_engine::fetch::builder().timeout(Duration::from_secs(8));
    b = match engine.proxy_url() {
        Some(p) => b.proxy(reqwest::Proxy::all(p)?),
        None => b.no_proxy(),
    };
    let c = b.build()?;
    // Any failure of the first service (blocked, down, bad answer) falls through to the second.
    let first = match c.get("https://ipwho.is/").send().await {
        Ok(r) => r.json::<serde_json::Value>().await.ok(),
        Err(_) => None,
    };
    if let Some(v) = first
        && v["success"].as_bool() == Some(true) {
            let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
            let net = format!("AS{} · {}", v["connection"]["asn"].as_u64().unwrap_or(0), v["connection"]["isp"].as_str().unwrap_or_default());
            return Ok(ExitInfo { ip: s("ip"), country: s("country"), country_code: s("country_code"), city: s("city"), network: net });
        }
    let second = async { c.get("https://ipinfo.io/json").send().await?.json::<serde_json::Value>().await };
    let v = second.await.map_err(|e| {
        let mut why = kova_engine::fetch::describe(&e);
        if let Some(n) = kova_engine::fetch::trusted_certificates() {
            why.push_str(&format!(" ({n} trusted certificates on this device)"));
        }
        Error::Other(why)
    })?;
    let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
    Ok(ExitInfo { ip: s("ip"), country: s("country"), country_code: s("country"), city: s("city"), network: s("org") })
}

#[derive(Serialize)]
struct Crash {
    /// Unix seconds.
    time: i64,
    text: String,
}

/// What the last crash left in the logs folder, for the report.
#[tauri::command]
fn last_crash() -> Option<Crash> {
    logfile::last_crash().map(|(time, text)| Crash { time, text })
}

/// Whether Kova crashed since the window last asked: said once at start.
#[tauri::command]
fn crash_notice() -> bool {
    logfile::crash_is_new()
}

/// Android: ask to be left out of battery saving.
#[tauri::command]
async fn allow_background(app: AppHandle) -> R<()> {
    #[cfg(target_os = "android")]
    return android::allow_background(&app).await;
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Err(Error::Other("Only on Android".into()))
    }
}

/// Android: hide or show the status and navigation bars.
#[tauri::command]
async fn set_full_screen(app: AppHandle, on: bool) -> R<()> {
    #[cfg(target_os = "android")]
    return android::full_screen(&app, on).await;
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, on);
        Err(Error::Other("Only on Android".into()))
    }
}

/// Android: the phone maker's autostart settings.
#[tauri::command]
async fn open_autostart(app: AppHandle) -> R<()> {
    #[cfg(target_os = "android")]
    return android::open_autostart(&app).await;
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Err(Error::Other("Only on Android".into()))
    }
}

// ---------------------------------------------------------------- look pictures

/// Pictures for the look live here, one per slot: `<slot>-<hash>.<ext>`.
fn look_dir(engine: &Engine) -> std::path::PathBuf {
    engine.paths.root.join("look")
}

/// A slot name: letters, digits and dashes only, so it can't leave the folder.
fn valid_slot(slot: &str) -> bool {
    !slot.is_empty() && slot.len() <= 40 && slot.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// The picture formats a webview shows, told by their first bytes.
fn picture_ext(bytes: &[u8]) -> Option<&'static str> {
    if bytes.starts_with(&[0x89, b'P', b'N', b'G']) {
        Some("png")
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        Some("jpg")
    } else if bytes.starts_with(b"GIF8") {
        Some("gif")
    } else if bytes.len() > 12 && &bytes[..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some("webp")
    } else {
        None
    }
}

fn remove_slot_pictures(dir: &std::path::Path, slot: &str) {
    let prefix = format!("{slot}-");
    if let Ok(entries) = std::fs::read_dir(dir) {
        for e in entries.flatten() {
            if e.file_name().to_string_lossy().starts_with(&prefix) {
                let _ = std::fs::remove_file(e.path());
            }
        }
    }
}

/// Keeps a picture for a look slot (the raw bytes, the slot in the `slot` header) in
/// Kova's own folder, so it stays when the original goes. Returns its file name.
#[tauri::command]
fn look_picture_save(engine: Eng<'_>, request: tauri::ipc::Request<'_>) -> R<String> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err(Error::Other("No picture was sent".into()));
    };
    let slot = request.headers().get("slot").and_then(|v| v.to_str().ok()).unwrap_or_default();
    if !valid_slot(slot) {
        return Err(Error::Other("Unknown place for a picture".into()));
    }
    if bytes.len() > 10 * 1024 * 1024 {
        return Err(Error::Other("That picture is larger than 10 MB".into()));
    }
    let ext = picture_ext(bytes).ok_or_else(|| Error::Other("Use a PNG, JPG, WebP or GIF picture".into()))?;
    let dir = look_dir(&engine);
    std::fs::create_dir_all(&dir)?;
    remove_slot_pictures(&dir, slot);
    // A new name for a new picture, so the interface never shows an old copy.
    let hash = {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        bytes.hash(&mut h);
        h.finish()
    };
    let name = format!("{slot}-{:012x}.{ext}", hash & 0xFFFF_FFFF_FFFF);
    std::fs::write(dir.join(&name), bytes)?;
    Ok(name)
}

/// A kept picture's bytes, by the name `look_picture_save` gave it.
#[tauri::command]
fn look_picture(engine: Eng<'_>, name: String) -> R<tauri::ipc::Response> {
    let ok = name.split_once('.').is_some_and(|(stem, ext)| valid_slot(stem) && ["png", "jpg", "gif", "webp"].contains(&ext));
    if !ok {
        return Err(Error::Other("Unknown picture".into()));
    }
    Ok(tauri::ipc::Response::new(std::fs::read(look_dir(&engine).join(name))?))
}

#[tauri::command]
fn look_picture_remove(engine: Eng<'_>, slot: String) -> R<()> {
    if valid_slot(&slot) {
        remove_slot_pictures(&look_dir(&engine), &slot);
    }
    Ok(())
}

/// Writes a file the user chose a place for in the save dialog (a picture template): the
/// raw bytes, the path percent-encoded in the `path` header.
#[tauri::command]
fn save_chosen_file(request: tauri::ipc::Request<'_>) -> R<()> {
    let tauri::ipc::InvokeBody::Raw(bytes) = request.body() else {
        return Err(Error::Other("Nothing to save".into()));
    };
    let path = request.headers().get("path").and_then(|v| v.to_str().ok()).unwrap_or_default();
    let path = percent_decode(path);
    if path.is_empty() || bytes.len() > 20 * 1024 * 1024 {
        return Err(Error::Other("Nothing to save".into()));
    }
    std::fs::write(path, bytes)?;
    Ok(())
}

/// Saves a text file such as a theme: to `path` on the desktop (the window's save dialog
/// picked it), through Android's own picker there. False if the picker was cancelled.
#[tauri::command]
async fn save_text_document(app: AppHandle, path: Option<String>, name: String, text: String) -> R<bool> {
    #[cfg(target_os = "android")]
    {
        let _ = path;
        android::save_document(&app, &name, &text).await
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, name);
        let path = path.ok_or_else(|| Error::Other("Choose where to save it".into()))?;
        kova_engine::store::write_atomic(std::path::Path::new(&path), text.as_bytes())?;
        Ok(true)
    }
}

fn percent_decode(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%'
            && i + 2 < b.len()
            && let Some(v) = std::str::from_utf8(&b[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 3;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Read a user-chosen config file for import (size-capped).
#[tauri::command]
fn read_text_file(path: String) -> R<String> {
    let meta = std::fs::metadata(&path)?;
    if meta.len() > 8 * 1024 * 1024 {
        return Err(Error::Other("That file is larger than 8 MB; it is unlikely to be a config".into()));
    }
    Ok(std::fs::read_to_string(&path)?)
}

#[tauri::command]
fn window_action(app: AppHandle, action: String) {
    #[cfg(desktop)]
    desktop::window_action(&app, &action);
    #[cfg(not(desktop))]
    let _ = (app, action);
}

#[tauri::command]
async fn quit(app: AppHandle, engine: Eng<'_>) -> R<()> {
    let _ = stop(&app, engine.inner()).await;
    app.exit(0);
    Ok(())
}

// ---------------------------------------------------------------------------- background

async fn background(app: AppHandle, engine: Arc<Engine>) {
    // Cores are required to do anything; fetch any that are missing.
    let l = i18n::lang(&engine.settings());
    for st in engine.core_status(false).await {
        // Shoes is fetched only when a profile needs it (see the engine).
        if st.installed.is_none() && st.core != kova_core::CoreKind::Shoes {
            let name = st.core.label();
            let _ = app.emit("toast", tr(l, "Downloading {}…", &[name]));
            match engine.install_core(st.core).await {
                Ok(tag) => {
                    let _ = app.emit("toast", tr(l, "{} {} installed", &[name, &tag]));
                }
                Err(e) => {
                    let _ = app.emit("toast", tr(l, "Could not install {}: {}", &[name, &e.to_string()]));
                }
            }
            let _ = app.emit("cores-changed", ());
        }
    }
    let s = engine.settings();
    if s.connect_on_launch && s.active_profile.is_some()
        && let Err(e) = start(&app, &engine).await {
            let _ = app.emit("toast", tr(l, "Could not connect: {}", &[&e.to_string()]));
        }
    // A subscription about to run out is announced a few days ahead, once a day.
    tauri::async_runtime::spawn({
        let (app, engine) = (app.clone(), engine.clone());
        async move {
            loop {
                warn_subscriptions(&app, &engine);
                tokio::time::sleep(Duration::from_secs(24 * 3600)).await;
            }
        }
    });
    // A chosen node that stops answering is swapped for a working one, and a system proxy
    // another program took over is pointed out (once each time it happens).
    tauri::async_runtime::spawn({
        let (app, engine) = (app.clone(), engine.clone());
        async move {
            let mut proxy_lost = false;
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                if !engine.status().running {
                    proxy_lost = false;
                    continue;
                }
                let l = i18n::lang(&engine.settings());
                if let Some((from, to)) = engine.check_node().await {
                    let _ = app.emit("toast", tr(l, "{} stopped answering; switched to {}", &[&from, &to]));
                }
                let lost = engine.system_proxy_lost();
                if lost && !proxy_lost {
                    let _ = app.emit("toast", tr(l, "Another program changed the system proxy, so apps no longer go through Kova. Turn System proxy off and on to take it back", &[]));
                }
                proxy_lost = lost;
            }
        }
    });
    // Subscription and routing-data auto-update.
    loop {
        for id in engine.profiles_due() {
            if let Err(e) = engine.update_profile(&id).await {
                tracing::warn!("auto-update of profile {id} failed: {e}");
            }
        }
        if engine.geodata_due()
            && let Err(e) = engine.refresh_geodata_files(false).await {
                tracing::warn!("routing data update failed: {e}");
            }
        tokio::time::sleep(Duration::from_secs(15 * 60)).await;
    }
}

/// Whether the window is on screen (desktop) and the page says it is visible (any
/// platform). While either is false, the per-second traffic and memory figures aren't
/// sent: a window in the tray or an app in the background would only redraw unseen.
static WINDOW_SHOWN: AtomicBool = AtomicBool::new(true);
static PAGE_VISIBLE: AtomicBool = AtomicBool::new(true);

#[cfg_attr(not(desktop), allow(dead_code))]
pub(crate) fn note_window_shown(shown: bool) {
    WINDOW_SHOWN.store(shown, Ordering::Relaxed);
}

#[tauri::command]
fn page_visible(visible: bool) {
    PAGE_VISIBLE.store(visible, Ordering::Relaxed);
}

fn ui_watching() -> bool {
    WINDOW_SHOWN.load(Ordering::Relaxed) && PAGE_VISIBLE.load(Ordering::Relaxed)
}

fn forward_events(app: AppHandle, engine: Arc<Engine>) {
    let mut rx = engine.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    // The reset on disconnect always goes, so a window shown later has no stale speed.
                    let reset = matches!(ev, Event::Traffic { up: 0, down: 0, up_total: 0, down_total: 0 });
                    if matches!(ev, Event::Traffic { .. } | Event::Memory { .. }) && !reset && !ui_watching() {
                        continue;
                    }
                    // The tray lists the profiles and the servers, and shows the status.
                    let is_status = matches!(ev, Event::Status(_) | Event::Profiles);
                    let _ = app.emit("engine", &ev);
                    if is_status {
                        refresh_ui(&app);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });
}

/// Notify about subscriptions that end within three days or have used 90% of their
/// traffic: as a system notification, so it shows even with Kova in the background.
fn warn_subscriptions(app: &AppHandle, engine: &Arc<Engine>) {
    use tauri_plugin_notification::NotificationExt;
    // Notifications go through Kova's window on Android; without one they wait.
    #[cfg(target_os = "android")]
    if !android::window_open() {
        return;
    }
    let l = i18n::lang(&engine.settings());
    let now = kova_engine::store::now();
    for p in engine.profiles() {
        let Some(u) = p.usage else { continue };
        let left_days = u.expire.map(|e| (e - now) as f64 / 86400.0);
        let text = match left_days {
            Some(d) if d < 0.0 => Some(tr(l, "{} has expired", &[&p.name])),
            Some(d) if d <= 3.0 => Some(tr(l, "{} ends in {} days", &[&p.name, &(d.ceil() as i64).max(1).to_string()])),
            _ if u.total > 0 && (u.upload + u.download) as f64 >= u.total as f64 * 0.9 => {
                let pct = ((u.upload + u.download) as f64 / u.total as f64 * 100.0).min(100.0) as i64;
                Some(tr(l, "{} has used {}% of its traffic", &[&p.name, &pct.to_string()]))
            }
            _ => None,
        };
        if let Some(text) = text {
            let _ = app.notification().builder().title("Kova").body(&text).show();
        }
    }
}

/// A new install starts with the routing presets for where its user likely is, by the
/// system language: Russian, Chinese and Iranian sites direct where they are local.
fn first_run_presets(engine: &Arc<Engine>) {
    use kova_core::options::Preset;
    let lang = i18n::system_language().to_ascii_lowercase();
    let local = if lang.starts_with("ru") {
        Some(Preset::DirectRu)
    } else if lang.starts_with("zh") {
        Some(Preset::DirectCn)
    } else if lang.starts_with("fa") {
        Some(Preset::DirectIr)
    } else {
        None
    };
    let mut s = engine.settings();
    s.core.presets = local.into_iter().chain([Preset::BlockAds]).collect();
    let engine = engine.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(e) = engine.update_settings(s).await {
            tracing::warn!("could not set the first presets: {e}");
        }
    });
}

// ---------------------------------------------------------------------------- entry

/// `--restore-proxy [data folder]`: give the system proxy back if a Kova that ended
/// without its usual shutdown (Windows shutting down, a crash, ended in Task Manager or
/// by the uninstaller) left it pointing at itself. Kova does this on its next start too;
/// Windows runs it at the next sign-in (see `sysproxy`) and the uninstaller runs it,
/// because Kova may not start again. A Kova that is running and serving the proxy is
/// left alone. `None` without the argument.
///
/// The uninstaller runs as administrator: when that is another account than the one
/// using Kova, this sees the administrator's settings and has nothing to restore.
#[cfg(windows)]
pub fn restore_proxy_cli() -> Option<i32> {
    let mut args = std::env::args().skip(1);
    if args.next().as_deref() != Some("--restore-proxy") {
        return None;
    }
    // The folder given, else where Tauri keeps Kova's data on Windows; read only, nothing
    // is created.
    let Some(root) = args.next().map(Into::into).or_else(|| std::env::var_os("KOVA_DATA_DIR")).or_else(|| std::env::var_os("APPDATA").map(|a| std::path::Path::new(&a).join("app.kova.client").into_os_string())) else {
        return Some(0);
    };
    let marker = kova_engine::store::Paths { root: root.into() }.proxy_marker();
    if kova_engine::sysproxy::still_served(&marker) {
        return Some(0);
    }
    Some(match kova_engine::sysproxy::recover(&marker) {
        Ok(_) => 0,
        Err(e) => {
            eprintln!("could not restore the system proxy: {e}");
            1
        }
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    {
        use tracing_subscriber::prelude::*;
        // To the console, and to the log file once the data folder is known (`logfile`).
        tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
            .with(tracing_subscriber::fmt::layer())
            .with(tracing_subscriber::fmt::layer().with_ansi(false).with_writer(|| logfile::Writer))
            .init();
    }

    let builder = tauri::Builder::default();
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| desktop::show_main(app)))
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--minimized"])))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .on_window_event(|w, e| {
            // Closing the window keeps Kova running in the tray, like other proxy clients.
            match e {
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    let _ = w.hide();
                    note_window_shown(false);
                    api.prevent_close();
                }
                // Minimising and restoring, from the title bar or the taskbar.
                tauri::WindowEvent::Resized(_) => note_window_shown(!w.is_minimized().unwrap_or(false) && w.is_visible().unwrap_or(true)),
                _ => {}
            }
        });
    #[cfg(target_os = "android")]
    let builder = builder.plugin(android::plugin()).plugin(tauri_plugin_barcode_scanner::init());
    let app = builder
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_deep_link::init())
        .setup(|app| {
            let dir: std::path::PathBuf = std::env::var_os("KOVA_DATA_DIR").map(Into::into).unwrap_or(app.path().app_data_dir()?);
            logfile::start(&dir);
            tracing::info!("Kova {} starting", env!("CARGO_PKG_VERSION"));
            let first_run = !kova_engine::store::Paths::new(&dir).map(|p| p.settings().exists()).unwrap_or(true);
            let engine = Engine::new(dir).map_err(|e| e.to_string())?;
            if first_run {
                first_run_presets(&engine);
            }
            // Cores and routing data that came with the installer, for a first start
            // where GitHub can't be reached.
            #[cfg(desktop)]
            if let Ok(res) = app.path().resource_dir() {
                let adopted = kova_engine::cores::adopt_bundled(&res.join("cores"), &engine.paths.cores_dir());
                if !adopted.is_empty() {
                    tracing::info!("installed from the installer: {}", adopted.join(", "));
                }
            }
            app.manage(engine.clone());
            #[cfg(desktop)]
            desktop::setup(app, &engine)?;
            app.manage(updates::PendingUpdate::default());
            app.manage(backup::Opened::default());
            links::setup(app);
            let handle = app.handle().clone();
            forward_events(handle.clone(), engine.clone());
            #[cfg(target_os = "android")]
            android::listen_for_actions(handle.clone(), engine.clone());
            tauri::async_runtime::spawn(background(handle, engine));
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            page_visible,
            links::take_import_link,
            system_language,
            platform,
            updates::check_update,
            vpn_apps,
            open_vpn_settings,
            device_info,
            diagnose,
            privacy_check,
            speed_test,
            lan_address,
            receive_start,
            receive_stop,
            updates::install_update,
            learned_sites,
            forget_learned,
            keep_learned,
            clear_learned,
            smart_suggestions,
            accept_suggestion,
            dismiss_suggestion,
            connect,
            disconnect,
            save_settings,
            add_profile,
            update_profile,
            update_all_profiles,
            remove_profile,
            set_active_profile,
            edit_profile,
            get_profile_content,
            save_profile_content,
            profile_nodes,
            get_proxies,
            select_proxy,
            test_delays,
            check_quality,
            look_picture_save,
            look_picture,
            look_picture_remove,
            save_chosen_file,
            save_text_document,
            get_connections,
            close_connection,
            close_all_connections,
            core_status,
            install_core,
            exit_info,
            read_text_file,
            last_crash,
            crash_notice,
            allow_background,
            open_autostart,
            set_full_screen,
            backup::backup_save,
            backup::backup_open,
            backup::backup_restore,
            backup::pick_text_file,
            geodata_info,
            update_geodata,
            service_status,
            default_check_sites,
            running_programs,
            check_site,
            service_install,
            service_uninstall,
            window_action,
            quit,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Kova");

    app.run(|app, e| {
        // Android: the window was closed (swiped away from the recent apps). While
        // connected Kova keeps running without it, as the VPN does; opened again, it gets
        // a new window (`android::reopen_window`).
        #[cfg(target_os = "android")]
        if let RunEvent::ExitRequested { code: None, api, .. } = &e
            && app.state::<Arc<Engine>>().has_tun_fd()
        {
            tracing::info!("window closed; staying connected");
            api.prevent_exit();
            return;
        }
        if let RunEvent::Exit = e {
            // Always give the system proxy (or the VPN) back, whatever path led here.
            let engine = app.state::<Arc<Engine>>().inner().clone();
            // Android: Kova's window was closed while disconnected and Tauri exits with it.
            // Stop the cores and close the interface (in case); the VPN service
            // ends with the process. Then end the process at once: Tauri would call
            // `exit`, which runs the native libraries' cleanup while Android's own threads
            // still use them, and that crashes.
            #[cfg(target_os = "android")]
            {
                tauri::async_runtime::block_on(async {
                    let _ = tokio::time::timeout(std::time::Duration::from_secs(3), engine.stop()).await;
                });
                if let Some(fd) = engine.take_tun_fd() {
                    android::close_fd(fd);
                }
                android::end_process();
            }
            #[cfg(not(target_os = "android"))]
            {
                let app = app.clone();
                tauri::async_runtime::block_on(async move {
                    let _ = stop(&app, &engine).await;
                });
            }
        }
    });
}

#[cfg(test)]
mod look_picture_tests {
    use super::{percent_decode, picture_ext, valid_slot};

    #[test]
    fn pictures_are_told_by_their_first_bytes() {
        assert_eq!(picture_ext(b"\x89PNG\r\n\x1a\nrest"), Some("png"));
        assert_eq!(picture_ext(&[0xFF, 0xD8, 0xFF, 0xE0]), Some("jpg"));
        assert_eq!(picture_ext(b"GIF89a..."), Some("gif"));
        assert_eq!(picture_ext(b"RIFF\0\0\0\0WEBPVP8 "), Some("webp"));
        assert_eq!(picture_ext(b"<svg xmlns="), None, "SVG can carry scripts: not taken");
        assert_eq!(picture_ext(b"MZ\x90\0"), None);
    }

    #[test]
    fn slot_names_stay_inside_the_folder() {
        assert!(valid_slot("app") && valid_slot("tab-proxies") && valid_slot("app-phone"));
        assert!(!valid_slot("") && !valid_slot("../x") && !valid_slot("a/b") && !valid_slot("App") && !valid_slot("a.b"));
    }

    #[test]
    fn paths_come_back_from_percent_encoding() {
        assert_eq!(percent_decode("C%3A%5CUsers%5C%D0%AF%5Ct.png"), r"C:\Users\Я\t.png");
        assert_eq!(percent_decode("100%"), "100%");
    }
}
