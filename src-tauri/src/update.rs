//! In-app updates. The release workflow signs the installer and publishes
//! `latest.json` with each GitHub release. The updater reads it from the latest release.

use crate::AppState;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_updater::UpdaterExt;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    current: String,
    /// The newer version, if there is one.
    available: Option<String>,
}

async fn find(app: &AppHandle) -> Result<Option<tauri_plugin_updater::Update>, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    updater.check().await.map_err(|e| format!("Could not check for updates ({e})"))
}

#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<UpdateInfo, String> {
    let available = find(&app).await?.map(|u| u.version);
    Ok(UpdateInfo { current: app.package_info().version.to_string(), available })
}

/// Downloads and runs the new installer. On Windows the app exits and the installer
/// starts it again when it is done.
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    let update = find(&app).await?.ok_or("Rewind is up to date")?;
    let mut got = 0u64;
    let mut shown = -1i64;
    let bytes = update
        .download(
            |chunk, total| {
                got += chunk as u64;
                let Some(total) = total.filter(|t| *t > 0) else { return };
                let pct = (got * 100 / total) as i64;
                if pct != shown {
                    shown = pct;
                    let _ = app.emit("update-progress", got as f64 / total as f64);
                }
            },
            || {},
        )
        .await
        .map_err(|e| format!("Could not download the update ({e})"))?;

    // Stop capture first so FFmpeg does not lock files that the installer replaces.
    let rec = app.state::<AppState>().recorder.clone();
    let _ = tauri::async_runtime::spawn_blocking(move || rec.shutdown()).await;

    if let Err(e) = update.install(bytes) {
        let state = app.state::<AppState>();
        let rec = state.recorder.clone();
        let settings = state.settings.lock().clone();
        std::thread::spawn(move || rec.apply(settings, true));
        return Err(format!("Could not install the update ({e})"));
    }
    Ok(())
}
