//! Desktop: the tray icon and its menu, and the main window.

use std::sync::Arc;

use kova_core::options::Mode;
use kova_engine::store::{Profile, Settings};
use kova_engine::{Engine, Status};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{App, AppHandle, Emitter, Manager, Wry};

use crate::i18n::{self, tr};

/// Create the tray icon and show the window unless Kova starts in the tray.
pub fn setup(app: &App, engine: &Arc<Engine>) -> tauri::Result<()> {
    let handle = app.handle().clone();
    let menu = tray_menu(&handle, &engine.status(), &engine.settings(), &engine.profiles(), None)?;
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
    if minimized {
        crate::note_window_shown(false);
    } else {
        show_main(&handle);
    }
    Ok(())
}

/// Most entries a submenu lists; a subscription with hundreds of servers stays usable.
const MAX_ITEMS: usize = 40;

/// The main group's name, current choice and members, while connected.
type Choices = Option<(String, String, Vec<String>)>;

fn tray_menu(app: &AppHandle, st: &Status, s: &Settings, profiles: &[Profile], servers: Choices) -> tauri::Result<Menu<Wry>> {
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
    let menu = Menu::with_items(app, &[&toggle])?;
    // Picking a server or a profile without opening the window.
    if let Some((_, now, all)) = servers.filter(|(_, _, all)| all.len() > 1) {
        let items = all.iter().take(MAX_ITEMS).map(|n| CheckMenuItem::with_id(app, format!("node:{n}"), n, true, *n == now, None::<&str>)).collect::<tauri::Result<Vec<_>>>()?;
        let refs: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = items.iter().map(|i| i as _).collect();
        menu.append(&Submenu::with_items(app, t("Server"), true, &refs)?)?;
    }
    if profiles.len() > 1 {
        let items = profiles.iter().take(MAX_ITEMS).map(|p| CheckMenuItem::with_id(app, format!("profile:{}", p.id), &p.name, true, s.active_profile.as_deref() == Some(p.id.as_str()), None::<&str>)).collect::<tauri::Result<Vec<_>>>()?;
        let refs: Vec<&dyn tauri::menu::IsMenuItem<Wry>> = items.iter().map(|i| i as _).collect();
        menu.append(&Submenu::with_items(app, t("Profile"), true, &refs)?)?;
    }
    for item in [&sep1 as &dyn tauri::menu::IsMenuItem<Wry>, &mode, &sys, &tun, &sep2, &show, &quit] {
        menu.append(item)?;
    }
    Ok(menu)
}

pub fn refresh_tray(app: &AppHandle) {
    // The server list comes from the running core, so the menu is built once it answers.
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        let engine = app.state::<Arc<Engine>>().inner().clone();
        let servers = if engine.status().running { engine.main_choices().await } else { None };
        set_tray(&app, &engine, servers);
    });
}

fn set_tray(app: &AppHandle, engine: &Engine, servers: Choices) {
    let (st, s) = (engine.status(), engine.settings());
    if let Some(tray) = app.tray_by_id("main") {
        if let Ok(menu) = tray_menu(app, &st, &s, &engine.profiles(), servers) {
            let _ = tray.set_menu(Some(menu));
        }
        let l = i18n::lang(&s);
        let tip = if st.running { tr(l, "Kova: connected ({})", &[st.core.map(|c| c.label()).unwrap_or("")]) } else { tr(l, "Kova: disconnected", &[]) };
        let _ = tray.set_tooltip(Some(tip));
    }
}

pub fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
        crate::note_window_shown(true);
    }
}

pub fn window_action(app: &AppHandle, action: &str) {
    let Some(w) = app.get_webview_window("main") else { return };
    let _ = match action {
        "minimize" => w.minimize().inspect(|_| crate::note_window_shown(false)),
        "maximize" => w.is_maximized().and_then(|m| if m { w.unmaximize() } else { w.maximize() }),
        "hide" => w.hide().inspect(|_| crate::note_window_shown(false)),
        _ => Ok(()),
    };
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
            other if other.starts_with("node:") => match engine.main_choices().await {
                Some((group, _, _)) => engine.select(&group, &other["node:".len()..]).await,
                None => Ok(()),
            },
            other if other.starts_with("profile:") => engine.set_active_profile(&other["profile:".len()..]).await,
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
