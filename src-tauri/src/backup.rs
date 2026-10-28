//! Saving Kova's profiles and settings to a file and restoring them; the format is
//! `kova_engine::backup`. On the desktop the window picks the path; Android writes and
//! reads through its own file picker.

use std::sync::Arc;

use kova_engine::backup::Backup;
use kova_engine::{Engine, Error};
use serde::Serialize;
use tauri::{AppHandle, State};

type R<T> = Result<T, Error>;

/// The backup `backup_open` read, waiting for `backup_restore`, so an encrypted one is
/// opened (slow on purpose) only once.
#[derive(Default)]
pub struct Opened(std::sync::Mutex<Option<Backup>>);

#[derive(Serialize)]
pub struct Summary {
    needs_password: bool,
    kova: Option<String>,
    created: Option<i64>,
    profiles: Vec<String>,
}

#[derive(Serialize)]
pub struct Restored {
    added: usize,
    skipped: usize,
}

fn blocking_error(e: tokio::task::JoinError) -> Error {
    Error::Other(e.to_string())
}

/// Save a backup, encrypted with `password` when one is given: to `path` on the
/// desktop, where the user picks on Android. False if they cancelled the picker.
#[tauri::command]
pub async fn backup_save(app: AppHandle, engine: State<'_, Arc<Engine>>, password: Option<String>, path: Option<String>, name: String) -> R<bool> {
    let engine = engine.inner().clone();
    // Deriving the key takes a moment on purpose; keep it off the async workers.
    let text = tokio::task::spawn_blocking(move || engine.export_backup(password.as_deref())).await.map_err(blocking_error)??;
    #[cfg(target_os = "android")]
    {
        let _ = path;
        crate::android::save_document(&app, &name, &text).await
    }
    #[cfg(not(target_os = "android"))]
    {
        let _ = (app, name);
        let path = path.ok_or_else(|| Error::Other("Choose where to save the backup".into()))?;
        kova_engine::store::write_atomic(std::path::Path::new(&path), text.as_bytes())?;
        Ok(true)
    }
}

/// Read a backup's text, with `password` when it is encrypted, and keep it for
/// `backup_restore`. An encrypted one without a password only says it needs one.
#[tauri::command]
pub async fn backup_open(opened: State<'_, Opened>, text: String, password: Option<String>) -> R<Summary> {
    if kova_engine::backup::needs_password(&text)? && password.as_deref().is_none_or(str::is_empty) {
        return Ok(Summary { needs_password: true, kova: None, created: None, profiles: Vec::new() });
    }
    let b = tokio::task::spawn_blocking(move || kova_engine::backup::read(&text, password.as_deref())).await.map_err(blocking_error)??;
    let summary = Summary { needs_password: false, kova: Some(b.kova.clone()), created: Some(b.created), profiles: b.profiles.iter().map(|p| p.profile.name.clone()).collect() };
    *opened.0.lock().unwrap() = Some(b);
    Ok(summary)
}

/// Restore the opened backup's profiles, and its settings too with `with_settings`.
#[tauri::command]
pub async fn backup_restore(engine: State<'_, Arc<Engine>>, opened: State<'_, Opened>, with_settings: bool) -> R<Restored> {
    let b = opened.0.lock().unwrap().take().ok_or_else(|| Error::Other("Open the backup first".into()))?;
    let (added, skipped) = engine.import_backup(b, with_settings).await?;
    Ok(Restored { added, skipped })
}

/// Android: the text of a file the user picks (`None` if they cancel). The desktop
/// window uses its own file dialog and `read_text_file`.
#[tauri::command]
pub async fn pick_text_file(app: AppHandle) -> R<Option<String>> {
    #[cfg(target_os = "android")]
    return crate::android::open_document(&app).await;
    #[cfg(not(target_os = "android"))]
    {
        let _ = app;
        Err(Error::Other("Only on Android".into()))
    }
}
