//! The app shell: exposes the engine to the UI as commands and events. The tray and
//! window live in `desktop`, the Android VPN in `android`.

#[cfg(target_os = "android")]
mod android;
#[cfg(desktop)]
mod desktop;
mod i18n;
mod links;
mod receive;
mod updates;

use std::collections::HashMap;
use std::sync::Arc;
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
async fn select_proxy(engine: Eng<'_>, group: String, name: String) -> R<()> {
    engine.select(&group, &name).await
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
async fn service_status() -> ServiceView {
    let app_version = env!("CARGO_PKG_VERSION").to_string();
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
    #[cfg(not(windows))]
    ServiceView { supported: false, installed: false, running: false, version: None, app_version, core_version: None, outdated: false }
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
    #[cfg(not(windows))]
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
    #[cfg(not(windows))]
    {
        let _ = engine;
        Ok(())
    }
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
    if let Some(v) = first {
        if v["success"].as_bool() == Some(true) {
            let s = |k: &str| v[k].as_str().unwrap_or_default().to_string();
            let net = format!("AS{} · {}", v["connection"]["asn"].as_u64().unwrap_or(0), v["connection"]["isp"].as_str().unwrap_or_default());
            return Ok(ExitInfo { ip: s("ip"), country: s("country"), country_code: s("country_code"), city: s("city"), network: net });
        }
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
        if st.installed.is_none() {
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
    if s.connect_on_launch && s.active_profile.is_some() {
        if let Err(e) = start(&app, &engine).await {
            let _ = app.emit("toast", tr(l, "Could not connect: {}", &[&e.to_string()]));
        }
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
    // A chosen node that stops answering is swapped for a working one.
    tauri::async_runtime::spawn({
        let (app, engine) = (app.clone(), engine.clone());
        async move {
            loop {
                tokio::time::sleep(Duration::from_secs(60)).await;
                if !engine.status().running {
                    continue;
                }
                if let Some((from, to)) = engine.check_node().await {
                    let l = i18n::lang(&engine.settings());
                    let _ = app.emit("toast", tr(l, "{} stopped answering; switched to {}", &[&from, &to]));
                }
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
        if engine.geodata_due() {
            if let Err(e) = engine.refresh_geodata_files(false).await {
                tracing::warn!("routing data update failed: {e}");
            }
        }
        tokio::time::sleep(Duration::from_secs(15 * 60)).await;
    }
}

fn forward_events(app: AppHandle, engine: Arc<Engine>) {
    let mut rx = engine.subscribe();
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(ev) => {
                    let is_status = matches!(ev, Event::Status(_));
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

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into())).init();

    let builder = tauri::Builder::default();
    #[cfg(desktop)]
    let builder = builder
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| desktop::show_main(app)))
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--minimized"])))
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .on_window_event(|w, e| {
            // Closing the window keeps Kova running in the tray, like other proxy clients.
            if let tauri::WindowEvent::CloseRequested { api, .. } = e {
                let _ = w.hide();
                api.prevent_close();
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
            get_connections,
            close_connection,
            close_all_connections,
            core_status,
            install_core,
            exit_info,
            read_text_file,
            geodata_info,
            update_geodata,
            service_status,
            default_check_sites,
            check_site,
            service_install,
            service_uninstall,
            window_action,
            quit,
        ])
        .build(tauri::generate_context!())
        .expect("failed to build Kova");

    app.run(|app, e| {
        if let RunEvent::Exit = e {
            // Always give the system proxy (or the VPN) back, whatever path led here.
            let engine = app.state::<Arc<Engine>>().inner().clone();
            let app = app.clone();
            tauri::async_runtime::block_on(async move {
                let _ = stop(&app, &engine).await;
            });
        }
    });
}
