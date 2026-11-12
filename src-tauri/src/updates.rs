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
#[derive(Clone)]
struct Release {
    version: semver::Version,
    asset: String,
    /// The asset's SHA-256 as GitHub publishes it (`sha256:…`), when it does.
    digest: Option<String>,
    date: Option<String>,
    body: Option<String>,
}

/// The newest published release (with `beta`, pre-releases too) that has the file
/// `asset(version)`. Asks GitHub's API first, which also gives the file's checksum and the
/// notes; when the API refuses (it allows 60 requests an hour per address, and everyone on
/// a VPN server shares its address), the public releases feed and the file's own address,
/// which have no such limit, answer instead.
async fn newest_release(beta: bool, asset: impl Fn(&semver::Version) -> String) -> R<Option<Release>> {
    match from_api(beta, &asset).await {
        Ok(found) => Ok(found),
        Err(e) => {
            tracing::info!("GitHub's API didn't answer ({e}); looking at the releases feed");
            from_feed(beta, &asset).await
        }
    }
}

async fn from_api(beta: bool, asset: &impl Fn(&semver::Version) -> String) -> R<Option<Release>> {
    let c = kova_engine::fetch::client(None, kova_engine::fetch::APP_UA)?;
    let url = format!("https://api.github.com/repos/{RELEASES}/releases?per_page=30");
    let list: Vec<serde_json::Value> = c.get(url).header("Accept", "application/vnd.github+json").send().await?.error_for_status()?.json().await?;
    Ok(list
        .iter()
        .filter(|r| r["draft"].as_bool() != Some(true) && (beta || r["prerelease"].as_bool() != Some(true)))
        .filter_map(|r| {
            let version = semver::Version::parse(r["tag_name"].as_str()?.trim_start_matches('v')).ok()?;
            let name = asset(&version);
            let a = r["assets"].as_array()?.iter().find(|a| a["name"].as_str() == Some(name.as_str()))?;
            Some(Release {
                asset: a["browser_download_url"].as_str()?.to_string(),
                digest: a["digest"].as_str().and_then(|d| d.strip_prefix("sha256:")).map(str::to_string),
                date: r["published_at"].as_str().map(str::to_string),
                body: r["body"].as_str().map(str::to_string),
                version,
            })
        })
        .max_by(|a, b| a.version.cmp(&b.version)))
}

/// Versions named in the releases feed, newest first; with `beta`, pre-release ones too.
fn feed_versions(feed: &str, beta: bool) -> Vec<semver::Version> {
    let mut versions: Vec<semver::Version> = feed
        .split("/releases/tag/v")
        .skip(1)
        .filter_map(|rest| {
            let tag: String = rest.chars().take_while(|c| !matches!(c, '"' | '\'' | '<' | ' ')).collect();
            semver::Version::parse(&tag).ok()
        })
        .filter(|v| beta || v.pre.is_empty())
        .collect();
    versions.sort();
    versions.dedup();
    versions.reverse();
    versions
}

async fn from_feed(beta: bool, asset: &impl Fn(&semver::Version) -> String) -> R<Option<Release>> {
    let c = kova_engine::fetch::client(None, kova_engine::fetch::APP_UA)?;
    let feed = c.get(format!("https://github.com/{RELEASES}/releases.atom")).send().await?.error_for_status()?.text().await?;
    // The feed lists tags, including those of releases still being prepared, whose files
    // aren't public yet: the newest version whose file can be fetched is the one.
    for version in feed_versions(&feed, beta).into_iter().take(6) {
        let url = format!("https://github.com/{RELEASES}/releases/download/v{version}/{}", asset(&version));
        if c.head(&url).send().await.is_ok_and(|r| r.status().is_success()) {
            return Ok(Some(Release { version, asset: url, digest: None, date: None, body: None }));
        }
    }
    Ok(None)
}

/// The SHA-256 a checksum text names, in lowercase: a bare digest as GitHub's API gives
/// it, or a `sha256sum` line (`<digest>  <file>`) as the `.sha256` file next to the APK
/// holds. `None` for anything else, such as an error page.
#[cfg_attr(desktop, allow(dead_code))]
fn sha256_from_text(text: &str) -> Option<String> {
    let digest = text.split_whitespace().next()?;
    (digest.len() == 64 && digest.bytes().all(|b| b.is_ascii_hexdigit())).then(|| digest.to_ascii_lowercase())
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
    if engine.settings().beta_updates
        && let Some(r) = newest_release(true, |_| "latest.json".to_string()).await? {
            let url = tauri::Url::parse(&r.asset).map_err(|e| Error::Other(e.to_string()))?;
            builder = builder.endpoints(vec![url]).map_err(update_error)?;
        }
    // Through Kova's proxy when connected, like the download below.
    if let Some(p) = engine.proxy_url() {
        builder = builder.proxy(tauri::Url::parse(&p).map_err(|e| Error::Other(e.to_string()))?);
    }
    let update = builder.build().map_err(update_error)?.check().await.map_err(update_error)?;
    let mut info = update.as_ref().map(|u| UpdateInfo { version: u.version.clone(), date: u.date.map(|d| d.to_string()), body: u.body.clone() });
    // latest.json is written by the build, before the release notes are; take them from
    // the release itself, keeping latest.json's only if GitHub doesn't answer.
    if let Some(i) = info.as_mut()
        && let Some(notes) = release_notes(engine.proxy_url().as_deref(), &i.version).await
    {
        i.body = Some(notes);
    }
    *app.state::<PendingUpdate>().0.lock().await = update;
    Ok(info)
}

/// The notes of the release tagged `v{version}`, if GitHub answers.
#[cfg(desktop)]
async fn release_notes(proxy: Option<&str>, version: &str) -> Option<String> {
    let c = kova_engine::fetch::client(proxy, kova_engine::fetch::APP_UA).ok()?;
    let url = format!("https://api.github.com/repos/{RELEASES}/releases/tags/v{version}");
    let r: serde_json::Value = c.get(url).header("Accept", "application/vnd.github+json").send().await.ok()?.error_for_status().ok()?.json().await.ok()?;
    r["body"].as_str().map(str::to_string).filter(|b| !b.trim().is_empty())
}

/// Download, install and restart, reporting progress (0..1) as `update-progress`.
/// Downloads while still connected, through Kova's proxy: GitHub's downloads are often
/// slow or blocked directly where Kova is needed. Then disconnects, because the installer
/// closes Kova without the usual shutdown, which would leave the system proxy pointing at it.
#[cfg(desktop)]
#[tauri::command]
pub async fn install_update(app: AppHandle, engine: State<'_, Arc<Engine>>) -> R<()> {
    use tauri::Emitter;
    let mut update = app.state::<PendingUpdate>().0.lock().await.clone().ok_or_else(|| Error::Other("Check for updates first".into()))?;
    update.proxy = engine.proxy_url().and_then(|p| tauri::Url::parse(&p).ok());
    let mut attempts = 0;
    let bytes = loop {
        attempts += 1;
        let (progress, mut got, mut total) = (app.clone(), 0u64, 0u64);
        let result = update
            .download(
                move |chunk, len| {
                    got += chunk as u64;
                    total = len.unwrap_or(total);
                    let _ = progress.emit("update-progress", if total > 0 { got as f64 / total as f64 } else { 0.0 });
                },
                || {},
            )
            .await;
        match result {
            Ok(bytes) => break bytes,
            Err(e) if attempts < 3 => tracing::warn!("update download failed, trying again: {e}"),
            Err(e) if update.proxy.is_none() => {
                tracing::warn!("update download failed: {e}");
                return Err(Error::Other("Could not download the update. Connect Kova and try again".into()));
            }
            Err(e) => return Err(update_error(e)),
        }
    };
    if engine.status().running {
        engine.stop().await?;
    }
    update.install(bytes).map_err(update_error)?;
    // Windows has already closed Kova to run the installer; elsewhere start the new one.
    app.restart()
}

// ---------------------------------------------------------------------------- mobile

/// The APK the last check found.
#[cfg(mobile)]
#[derive(Default)]
pub struct PendingUpdate(std::sync::Mutex<Option<Release>>);

#[cfg(mobile)]
#[tauri::command]
pub async fn check_update(app: AppHandle, engine: State<'_, Arc<Engine>>) -> R<Option<UpdateInfo>> {
    let current = semver::Version::parse(&app.package_info().version.to_string()).map_err(|e| Error::Other(e.to_string()))?;
    // Since 0.1.18 one APK serves every device, 64-bit phones and 32-bit phones and TVs.
    let found = newest_release(engine.settings().beta_updates, |v| format!("Kova_{v}_Android.apk")).await?.filter(|r| r.version > current);
    let info = found.as_ref().map(|r| UpdateInfo { version: r.version.to_string(), date: r.date.clone(), body: r.body.clone() });
    *app.state::<PendingUpdate>().0.lock().unwrap() = found;
    Ok(info)
}

/// Download the APK, reporting progress (0..1) as `update-progress`, check it against
/// the digest GitHub publishes (or the `.sha256` file of the release), and hand it to
/// Android's installer, which asks the user to confirm. Android installs it over Kova,
/// keeping its data, only when it is signed with the same key as the installed one.
#[cfg(mobile)]
#[tauri::command]
pub async fn install_update(app: AppHandle, engine: State<'_, Arc<Engine>>) -> R<()> {
    use std::time::Duration;
    use tauri::Emitter;
    let r = app.state::<PendingUpdate>().0.lock().unwrap().clone().ok_or_else(|| Error::Other("Check for updates first".into()))?;
    // An 80 MB file on mobile data takes minutes, so there is no limit on the whole
    // download, only on silence. Through Kova's proxy when connected: GitHub's downloads
    // are often slow or blocked directly where Kova is needed.
    let mut b = kova_engine::fetch::builder().user_agent(kova_engine::fetch::APP_UA).connect_timeout(Duration::from_secs(15)).read_timeout(Duration::from_secs(30));
    b = match engine.proxy_url() {
        Some(p) => b.proxy(reqwest::Proxy::all(p)?),
        None => b.no_proxy(),
    };
    let c = b.build()?;
    let mut data: Vec<u8> = Vec::new();
    let mut total = 0u64;
    // A dropped connection resumes where it stopped, a few times over.
    let mut attempts = 0;
    loop {
        attempts += 1;
        let mut req = c.get(&r.asset);
        if !data.is_empty() {
            req = req.header(reqwest::header::RANGE, format!("bytes={}-", data.len()));
        }
        let step: R<()> = async {
            let mut resp = req.send().await?.error_for_status()?;
            if resp.status() != reqwest::StatusCode::PARTIAL_CONTENT {
                // The server sent the whole file again.
                data.clear();
                total = resp.content_length().unwrap_or(0);
            } else if total == 0 {
                total = data.len() as u64 + resp.content_length().unwrap_or(0);
            }
            while let Some(chunk) = resp.chunk().await? {
                data.extend_from_slice(&chunk);
                if total > 0 {
                    let _ = app.emit("update-progress", (data.len() as f64 / total as f64).min(0.99));
                }
            }
            Ok(())
        }
        .await;
        match step {
            Ok(()) if total == 0 || data.len() as u64 >= total => break,
            Ok(()) if attempts < 5 => continue,
            Err(_) if attempts < 5 => tokio::time::sleep(Duration::from_secs(2)).await,
            Ok(()) => return Err(Error::Other("The download ended early. Try again".into())),
            Err(e) => return Err(e),
        }
    }
    // Nothing is handed to the installer without a checksum: GitHub's own digest, or
    // where the API didn't answer, the `.sha256` file published next to the APK.
    let expected = match r.digest.as_deref().and_then(sha256_from_text) {
        Some(d) => d,
        None => match c.get(format!("{}.sha256", r.asset)).send().await.and_then(|resp| resp.error_for_status()) {
            Ok(resp) => sha256_from_text(&resp.text().await?),
            Err(_) => None,
        }
        .ok_or_else(|| Error::Other(format!("Could not get the checksum of Kova {}, so it was not installed. Try again later", r.version)))?,
    };
    let actual = kova_engine::fetch::sha256_hex(&data);
    if actual != expected {
        return Err(Error::Other(format!("Kova {} failed its checksum check (expected {expected}, got {actual})", r.version)));
    }
    let dir = app.path().app_cache_dir().map_err(|e| Error::Other(e.to_string()))?.join("updates");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir)?;
    let path = dir.join(format!("Kova-{}.apk", r.version));
    std::fs::write(&path, &data)?;
    let _ = app.emit("update-progress", 1.0);
    crate::android::install_apk(&app, &path).await
}

#[cfg(test)]
mod tests {
    use super::{feed_versions, sha256_from_text};

    #[test]
    fn a_checksum_is_a_whole_sha256_or_nothing() {
        let hex = "9F86D081884C7D659A2FEAA0C55AD015A3BF4F1B2B0B822CD15D6C15B0F00A08";
        let lower = hex.to_ascii_lowercase();
        assert_eq!(sha256_from_text(hex).as_deref(), Some(lower.as_str()), "GitHub's digest");
        assert_eq!(sha256_from_text(&format!("{lower}  Kova_0.2.6_Android.apk\n")).as_deref(), Some(lower.as_str()), "a sha256sum line");
        assert_eq!(sha256_from_text("<html>Not Found</html>"), None);
        assert_eq!(sha256_from_text(&lower[..63]), None, "too short");
        assert_eq!(sha256_from_text(""), None);
    }

    #[test]
    fn versions_come_from_the_feed_newest_first() {
        let feed = r#"<entry><link rel="alternate" type="text/html" href="https://github.com/scarletjess77/kova/releases/tag/v0.1.32"/></entry>
<entry><link href="https://github.com/scarletjess77/kova/releases/tag/v0.1.29-beta.1"/></entry>
<entry><link href="https://github.com/scarletjess77/kova/releases/tag/v0.1.31"/><id>tag:github.com,2008:Repository/1/v0.1.31</id></entry>
<entry><link href="https://github.com/scarletjess77/kova/releases/tag/nightly"/></entry>"#;
        let names = |beta| feed_versions(feed, beta).iter().map(|v| v.to_string()).collect::<Vec<_>>();
        assert_eq!(names(false), ["0.1.32", "0.1.31"]);
        assert_eq!(names(true), ["0.1.32", "0.1.31", "0.1.29-beta.1"]);
    }
}
