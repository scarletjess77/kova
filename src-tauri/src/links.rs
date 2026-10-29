//! `kova://` links, such as the "Add to Kova" button on a provider's subscription page.
//! They open the Add profile dialog with the address filled in; nothing is imported
//! until the user presses Import.

use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_deep_link::DeepLinkExt;

/// The last link's address, kept until the interface takes it: a link that starts Kova
/// arrives before the interface is ready to show it.
#[derive(Default)]
pub struct Pending(Mutex<Option<String>>);

pub fn setup(app: &tauri::App) {
    app.manage(Pending::default());
    // Linux only learns of kova:// links from an installed package's desktop entry; an
    // AppImage has none, and one that was moved points at the old place. Registering at
    // every start makes links open this Kova wherever it is.
    #[cfg(target_os = "linux")]
    if let Err(e) = app.deep_link().register_all() {
        tracing::warn!("could not register kova:// links: {e}");
    }
    let handle = app.handle().clone();
    app.deep_link().on_open_url(move |e| {
        for url in e.urls() {
            received(&handle, url.as_str());
        }
    });
    if let Ok(Some(urls)) = app.deep_link().get_current() {
        for url in urls {
            received(app.handle(), url.as_str());
        }
    }
}

fn received(app: &AppHandle, link: &str) {
    // The address is a subscription's access key, so it is never logged.
    let Some(input) = kova_core::links::from_app_link(link) else {
        tracing::warn!("ignored a kova:// link that carries nothing to import");
        return;
    };
    *app.state::<Pending>().0.lock().unwrap() = Some(input);
    let _ = app.emit("import-link", ());
    #[cfg(desktop)]
    crate::desktop::show_main(app);
}

/// The address from the last link, once.
#[tauri::command]
pub fn take_import_link(pending: tauri::State<'_, Pending>) -> Option<String> {
    pending.0.lock().unwrap().take()
}
