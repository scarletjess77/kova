//! Desktop shell: exposes the engine to the UI as commands and events, and owns the
//! tray icon and window lifecycle.

mod i18n;

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use kova_core::CoreKind;
use kova_core::options::Mode;
use kova_engine::cores::CoreStatus;
use kova_engine::engine::ProfilePatch;
use kova_engine::mihomo_api::{ConnectionsSnapshot, ProxySnapshot};
use kova_engine::store::{Profile, Settings};
use kova_engine::{Engine, Error, Event, NodeView, Status};
use serde::Serialize;
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri_plugin_updater::UpdaterExt;
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WindowEvent, Wry};

use crate::i18n::tr;

type Eng<'a> = State<'a, Arc<Engine>>;
type R<T> = Result<T, Error>;

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
    refresh_tray(&app);
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
    refresh_tray(&app);
    Ok(engine.settings())
}

#[tauri::command]
fn dismiss_suggestion(engine: Eng, domain: String) {
    engine.dismiss_suggestion(&domain);
}

// ---------------------------------------------------------------------------- updates

/// The repository releases come from. The updater's own endpoint (tauri.conf.json) is
/// this repository's latest stable release.
const RELEASES: &str = "scarletjess77/kova";

/// The update the last check found, kept for installing (and retrying).
#[derive(Default)]
struct PendingUpdate(tokio::sync::Mutex<Option<tauri_plugin_updater::Update>>);

#[derive(Serialize)]
struct UpdateInfo {
    version: String,
    date: Option<String>,
    body: Option<String>,
}

fn update_error(e: tauri_plugin_updater::Error) -> Error {
    Error::Other(e.to_string())
}

/// The update manifest of the newest published release, betas included. GitHub's
/// "latest" link skips pre-releases, so the beta channel looks them up itself.
async fn newest_manifest() -> R<Option<String>> {
    let c = kova_engine::fetch::client(None, kova_engine::fetch::APP_UA)?;
    let url = format!("https://api.github.com/repos/{RELEASES}/releases?per_page=30");
    let list: Vec<serde_json::Value> = c.get(url).header("Accept", "application/vnd.github+json").send().await?.error_for_status()?.json().await?;
    let newest = list
        .iter()
        .filter(|r| r["draft"].as_bool() != Some(true))
        .filter_map(|r| {
            let version = semver::Version::parse(r["tag_name"].as_str()?.trim_start_matches('v')).ok()?;
            let manifest = r["assets"].as_array()?.iter().find(|a| a["name"] == "latest.json")?;
            Some((version, manifest["browser_download_url"].as_str()?.to_string()))
        })
        .max_by(|a, b| a.0.cmp(&b.0));
    Ok(newest.map(|(_, url)| url))
}

/// Look for a newer Kova: the latest stable release, or with beta updates on, the
/// newest release of any kind. Either way only a version above this one counts.
#[tauri::command]
async fn check_update(app: AppHandle, engine: Eng<'_>) -> R<Option<UpdateInfo>> {
    let mut builder = app.updater_builder();
    if engine.settings().beta_updates {
        if let Some(manifest) = newest_manifest().await? {
            let url = tauri::Url::parse(&manifest).map_err(|e| Error::Other(e.to_string()))?;
            builder = builder.endpoints(vec![url]).map_err(update_error)?;
        }
    }
    let update = builder.build().map_err(update_error)?.check().await.map_err(update_error)?;
    let info = update.as_ref().map(|u| UpdateInfo { version: u.version.clone(), date: u.date.map(|d| d.to_string()), body: u.body.clone() });
    *app.state::<PendingUpdate>().0.lock().await = update;
    Ok(info)
}

/// Download, install and restart, reporting progress (0..1) as `update-progress`.
/// Disconnects first: the installer closes Kova without the usual shutdown, which would
/// leave the system proxy pointing at it.
#[tauri::command]
async fn install_update(app: AppHandle, engine: Eng<'_>) -> R<()> {
    let update = app.state::<PendingUpdate>().0.lock().await.clone().ok_or_else(|| Error::Other("Check for updates first".into()))?;
    if engine.status().running {
        engine.stop().await?;
    }
    let (progress, mut got, mut total) = (app.clone(), 0u64, 0u64);
    update
        .download_and_install(
            move |chunk, len| {
                got += chunk as u64;
                total = len.unwrap_or(total);
                let _ = progress.emit("update-progress", if total > 0 { got as f64 / total as f64 } else { 0.0 });
            },
            || {},
        )
        .await
        .map_err(update_error)?;
    // Windows has already closed Kova to run the installer; elsewhere start the new one.
    app.restart()
}

#[tauri::command]
async fn connect(engine: Eng<'_>) -> R<()> {
    engine.start().await
}

#[tauri::command]
async fn disconnect(engine: Eng<'_>) -> R<()> {
    engine.stop().await
}

#[tauri::command]
async fn save_settings(app: AppHandle, engine: Eng<'_>, settings: Settings) -> R<Settings> {
    engine.update_settings(settings).await?;
    refresh_tray(&app);
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
    let st = engine.status();
    let mut b = reqwest::Client::builder().timeout(Duration::from_secs(8));
    b = if st.running { b.proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{}", st.mixed_port))?) } else { b.no_proxy() };
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
    let v: serde_json::Value = c.get("https://ipinfo.io/json").send().await?.json().await?;
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
    let Some(w) = app.get_webview_window("main") else { return };
    let _ = match action.as_str() {
        "minimize" => w.minimize(),
        "maximize" => w.is_maximized().and_then(|m| if m { w.unmaximize() } else { w.maximize() }),
        "hide" => w.hide(),
        _ => Ok(()),
    };
}

#[tauri::command]
async fn quit(app: AppHandle, engine: Eng<'_>) -> R<()> {
    let _ = engine.stop().await;
    app.exit(0);
    Ok(())
}

// ---------------------------------------------------------------------------- tray

fn tray_menu(app: &AppHandle, st: &Status, s: &Settings) -> tauri::Result<Menu<Wry>> {
    let l = i18n::lang(s);
    let t = |en| tr(l, en, &[]);
    let toggle = MenuItem::with_id(app, "toggle", t(if st.running { "Disconnect" } else { "Connect" }), !st.busy, None::<&str>)?;
    let m = s.core.mode;
    let rule = CheckMenuItem::with_id(app, "mode-rule", t("Rule"), true, m == Mode::Rule, None::<&str>)?;
    let global = CheckMenuItem::with_id(app, "mode-global", t("Global"), true, m == Mode::Global, None::<&str>)?;
    let direct = CheckMenuItem::with_id(app, "mode-direct", t("Direct"), true, m == Mode::Direct, None::<&str>)?;
    let mode = Submenu::with_items(app, t("Routing mode"), true, &[&rule, &global, &direct])?;
    let sys = CheckMenuItem::with_id(app, "sysproxy", t("System proxy"), true, s.system_proxy, None::<&str>)?;
    let tun = CheckMenuItem::with_id(app, "tun", t("TUN mode"), true, s.core.tun.enabled, None::<&str>)?;
    let show = MenuItem::with_id(app, "show", t("Open Kova"), true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", t("Quit Kova"), true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;
    Menu::with_items(app, &[&toggle, &sep1, &mode, &sys, &tun, &sep2, &show, &quit])
}

fn refresh_tray(app: &AppHandle) {
    let engine = app.state::<Arc<Engine>>();
    let (st, s) = (engine.status(), engine.settings());
    if let Some(tray) = app.tray_by_id("main") {
        if let Ok(menu) = tray_menu(app, &st, &s) {
            let _ = tray.set_menu(Some(menu));
        }
        let l = i18n::lang(&s);
        let tip = if st.running { tr(l, "Kova: connected ({})", &[st.core.map(|c| c.label()).unwrap_or("")]) } else { tr(l, "Kova: disconnected", &[]) };
        let _ = tray.set_tooltip(Some(tip));
    }
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn on_menu(app: &AppHandle, id: &str) {
    let app = app.clone();
    let id = id.to_string();
    tauri::async_runtime::spawn(async move {
        let engine = app.state::<Arc<Engine>>().inner().clone();
        let mut s = engine.settings();
        let result = match id.as_str() {
            "toggle" => {
                if engine.status().running { engine.stop().await } else { engine.start().await }
            }
            "mode-rule" | "mode-global" | "mode-direct" => {
                s.core.mode = match id.as_str() {
                    "mode-global" => Mode::Global,
                    "mode-direct" => Mode::Direct,
                    _ => Mode::Rule,
                };
                engine.update_settings(s).await
            }
            "sysproxy" => {
                s.system_proxy = !s.system_proxy;
                engine.update_settings(s).await
            }
            "tun" => {
                s.core.tun.enabled = !s.core.tun.enabled;
                engine.update_settings(s).await
            }
            "show" => {
                show_main(&app);
                Ok(())
            }
            "quit" => {
                let _ = engine.stop().await;
                app.exit(0);
                Ok(())
            }
            _ => Ok(()),
        };
        if let Err(e) = result {
            let _ = app.emit("toast", e.to_string());
        }
        let _ = app.emit("settings-changed", engine.settings());
        refresh_tray(&app);
    });
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
        if let Err(e) = engine.start().await {
            let _ = app.emit("toast", tr(l, "Could not connect: {}", &[&e.to_string()]));
        }
    }
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
                        refresh_tray(&app);
                    }
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });
}

// ---------------------------------------------------------------------------- entry

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt().with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into())).init();

    let app = tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, Some(vec!["--minimized"])))
        .plugin(tauri_plugin_clipboard_manager::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .setup(|app| {
            let dir = std::env::var_os("KOVA_DATA_DIR").map(Into::into).unwrap_or(app.path().app_data_dir()?);
            let engine = Engine::new(dir).map_err(|e| e.to_string())?;
            app.manage(engine.clone());
            app.manage(PendingUpdate::default());

            let handle = app.handle().clone();
            let menu = tray_menu(&handle, &engine.status(), &engine.settings())?;
            TrayIconBuilder::with_id("main")
                .icon(app.default_window_icon().cloned().expect("bundled icon"))
                .tooltip("Kova")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, e| on_menu(app, e.id().as_ref()))
                .on_tray_icon_event(|tray, e| {
                    if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = e {
                        show_main(tray.app_handle());
                    }
                })
                .build(app)?;

            let minimized = std::env::args().any(|a| a == "--minimized") || engine.settings().start_minimized;
            if !minimized {
                show_main(&handle);
            }
            forward_events(handle.clone(), engine.clone());
            tauri::async_runtime::spawn(background(handle, engine));
            Ok(())
        })
        .on_window_event(|w, e| {
            // Closing the window keeps Kova running in the tray, like other proxy clients.
            if let WindowEvent::CloseRequested { api, .. } = e {
                let _ = w.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_state,
            system_language,
            check_update,
            install_update,
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
            // Always give the system proxy back, whatever path led here.
            let engine = app.state::<Arc<Engine>>().inner().clone();
            tauri::async_runtime::block_on(async move {
                let _ = engine.stop().await;
            });
        }
    });
}
