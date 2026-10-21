//! Kova's own updates. On the desktop the updater installs a signed package in place; on
//! Android a newer APK from the same releases is downloaded for the system installer.

use std::sync::Arc;

use kova_engine::{Engine, Error};
use serde::Serialize;
use tauri::{AppHandle, Manager, State};

type R<T> = Result<T, Error>;

/// The repository releases come from. The desktop updater's own endpoint
/// (tauri.conf.json) is this repository's latest stable release.
const RELEASES: &str = "scarletjess77/kova";

#[derive(Serialize)]
pub struct UpdateInfo {
    version: String,
    date: Option<String>,
    body: Option<String>,
}

/// One published release and the asset of it this platform installs.
#[cfg_attr(desktop, allow(dead_code))]
struct Release {
    version: semver::Version,
    asset: String,
    date: Option<String>,
    body: Option<String>,
}

/// The newest published release with an asset `wanted` accepts. GitHub's "latest" link
/// skips pre-releases, so releases are listed and compared here; `beta` lets them count.
async fn newest_release(beta: bool, wanted: impl Fn(&str) -> bool) -> R<Option<Release>> {
    let c = kova_engine::fetch::client(None, kova_engine::fetch::APP_UA)?;
    let url = format!("https://api.github.com/repos/{RELEASES}/releases?per_page=30");
    let list: Vec<serde_json::Value> = c.get(url).header("Accept", "application/vnd.github+json").send().await?.error_for_status()?.json().await?;
    Ok(list
        .iter()
        .filter(|r| r["draft"].as_bool() != Some(true) && (beta || r["prerelease"].as_bool() != Some(true)))
        .filter_map(|r| {
            let version = semver::Version::parse(r["tag_name"].as_str()?.trim_start_matches('v')).ok()?;
            let asset = r["assets"].as_array()?.iter().find(|a| a["name"].as_str().is_some_and(&wanted))?;
            Some(Release {
                version,
                asset: asset["browser_download_url"].as_str()?.to_string(),
                date: r["published_at"].as_str().map(str::to_string),
                body: r["body"].as_str().map(str::to_string),
            })
        })
        .max_by(|a, b| a.version.cmp(&b.version)))
}

// ---------------------------------------------------------------------------- desktop

/// The update the last check found, kept for installing (and retrying).
#[cfg(desktop)]
#[derive(Default)]
pub struct PendingUpdate(tokio::sync::Mutex<Option<tauri_plugin_updater::Update>>);

#[cfg(desktop)]
fn update_error(e: tauri_plugin_updater::Error) -> Error {
    Error::Other(e.to_string())
}

/// Look for a newer Kova: the latest stable release, or with beta updates on, the
/// newest release of any kind. Either way only a version above this one counts.
#[cfg(desktop)]
#[tauri::command]
pub async fn check_update(app: AppHandle, engine: State<'_, Arc<Engine>>) -> R<Option<UpdateInfo>> {
    use tauri_plugin_updater::UpdaterExt;
    let mut builder = app.updater_builder();
    if engine.settings().beta_updates {
        if let Some(r) = newest_release(true, |name| name == "latest.json").await? {
            let url = tauri::Url::parse(&r.asset).map_err(|e| Error::Other(e.to_string()))?;
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
#[cfg(desktop)]
#[tauri::command]
pub async fn install_update(app: AppHandle, engine: State<'_, Arc<Engine>>) -> R<()> {
    use tauri::Emitter;
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

// ---------------------------------------------------------------------------- mobile

/// The APK the last check found.
#[cfg(mobile)]
#[derive(Default)]
pub struct PendingUpdate(std::sync::Mutex<Option<String>>);

#[cfg(mobile)]
#[tauri::command]
pub async fn check_update(app: AppHandle, engine: State<'_, Arc<Engine>>) -> R<Option<UpdateInfo>> {
    let current = semver::Version::parse(&app.package_info().version.to_string()).map_err(|e| Error::Other(e.to_string()))?;
    let found = newest_release(engine.settings().beta_updates, |name| name.ends_with(".apk")).await?.filter(|r| r.version > current);
    *app.state::<PendingUpdate>().0.lock().unwrap() = found.as_ref().map(|r| r.asset.clone());
    Ok(found.map(|r| UpdateInfo { version: r.version.to_string(), date: r.date, body: r.body }))
}

/// Hand the APK's link to the browser; Android's installer takes it from there and keeps
/// Kova's data, since every release is signed with the same key.
#[cfg(mobile)]
#[tauri::command]
pub async fn install_update(app: AppHandle) -> R<()> {
    use tauri_plugin_opener::OpenerExt;
    let url = app.state::<PendingUpdate>().0.lock().unwrap().clone().ok_or_else(|| Error::Other("Check for updates first".into()))?;
    app.opener().open_url(url, None::<&str>).map_err(|e| Error::Other(e.to_string()))
}
