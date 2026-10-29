mod audio;
mod ffmpeg;
mod gate;
mod library;
mod recorder;
mod ring;
mod settings;
mod win;

use library::{Clip, Library};
use parking_lot::Mutex;
use recorder::{Recorder, Status};
use serde::Serialize;
use settings::Settings;
use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, Manager, RunEvent, State, WebviewWindowBuilder};
use tauri_plugin_autostart::ManagerExt as _;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, Shortcut, ShortcutState};

struct AppState {
    settings: Mutex<Settings>,
    settings_file: PathBuf,
    library: Arc<Library>,
    recorder: Arc<Recorder>,
    record_item: Mutex<Option<MenuItem<tauri::Wry>>>,
}

type S<'a> = State<'a, AppState>;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Devices {
    monitors: Vec<win::Monitor>,
    mics: Vec<String>,
    encoders: Vec<ffmpeg::EncoderInfo>,
}

// ---------------------------------------------------------------- commands

#[tauri::command]
fn get_status(state: S) -> Status {
    state.recorder.status()
}

#[tauri::command]
fn get_settings(state: S) -> Settings {
    state.settings.lock().clone()
}

#[tauri::command]
fn save_settings(app: AppHandle, state: S, settings: Settings) -> Result<Settings, String> {
    let prev = state.settings.lock().clone();
    // Hotkeys first: an invalid key should not save half the settings.
    register_hotkeys(&app, &settings)?;
    settings.save(&state.settings_file).map_err(|e| e.to_string())?;
    if prev.launch_at_login != settings.launch_at_login {
        let al = app.autolaunch();
        let _ = if settings.launch_at_login { al.enable() } else { al.disable() };
    }
    *state.settings.lock() = settings.clone();
    let rec = state.recorder.clone();
    let s = settings.clone();
    std::thread::spawn(move || rec.apply(s, false));
    Ok(settings)
}

#[tauri::command]
async fn get_devices() -> Devices {
    tauri::async_runtime::spawn_blocking(|| {
        let avail = ffmpeg::available_encoders();
        Devices {
            monitors: win::monitors(),
            mics: audio::list_inputs(),
            encoders: ffmpeg::ENCODERS.iter().filter(|e| avail.contains(&e.name)).cloned().collect(),
        }
    })
    .await
    .unwrap_or(Devices { monitors: vec![], mics: vec![], encoders: vec![] })
}

#[tauri::command]
fn list_clips(state: S) -> Vec<Clip> {
    state.library.list()
}

#[tauri::command]
async fn save_clip(app: AppHandle) -> Result<Clip, String> {
    do_save_clip(app).await
}

#[tauri::command]
async fn toggle_recording(app: AppHandle) -> Result<Option<Clip>, String> {
    do_toggle_recording(app).await
}

#[tauri::command]
fn rename_clip(state: S, id: String, title: String) -> Result<Clip, String> {
    let title = title.trim().to_string();
    if title.is_empty() {
        return Err("Enter a name".into());
    }
    state.library.update(&id, |c| c.title = title).ok_or("Clip not found".into())
}

#[tauri::command]
fn set_favorite(state: S, id: String, favorite: bool) -> Result<Clip, String> {
    state.library.update(&id, |c| c.favorite = favorite).ok_or("Clip not found".into())
}

#[tauri::command]
fn delete_clip(state: S, id: String) -> Result<(), String> {
    let clip = state.library.remove(&id).ok_or("Clip not found")?;
    let _ = std::fs::remove_file(&clip.path);
    if let Some(t) = clip.thumb {
        let _ = std::fs::remove_file(t);
    }
    let _ = std::fs::remove_file(state.recorder.thumbs_dir().join(format!("{id}-strip.jpg")));
    Ok(())
}

#[tauri::command]
fn reveal_clip(state: S, id: String) -> Result<(), String> {
    let clip = state.library.get(&id).ok_or("Clip not found")?;
    win::reveal(&clip.path);
    Ok(())
}

#[tauri::command]
fn open_clips_folder(state: S) {
    let dir = state.settings.lock().clips_dir.clone();
    let _ = std::fs::create_dir_all(&dir);
    #[cfg(windows)]
    let _ = std::process::Command::new("explorer").arg(&dir).spawn();
    #[cfg(not(windows))]
    let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
}

#[tauri::command]
fn copy_clip(state: S, id: String) -> Result<(), String> {
    let clip = state.library.get(&id).ok_or("Clip not found")?;
    win::copy_file_to_clipboard(&clip.path)
}

/// Builds (once) a strip of frames for the trim timeline.
#[tauri::command]
async fn filmstrip(state: S<'_>, id: String) -> Result<PathBuf, String> {
    let clip = state.library.get(&id).ok_or("Clip not found")?;
    let out = state.recorder.thumbs_dir().join(format!("{id}-strip.jpg"));
    if out.exists() {
        return Ok(out);
    }
    tauri::async_runtime::spawn_blocking(move || {
        ffmpeg::filmstrip(&clip.path, &out, clip.duration, 12).map(|_| out)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportRequest {
    id: String,
    start: f64,
    end: f64,
    /// Target file size in MB. None keeps the capture quality.
    target_mb: Option<u32>,
}

#[derive(Clone, Serialize)]
struct ExportProgress {
    id: String,
    progress: f64,
}

#[tauri::command]
async fn export_clip(app: AppHandle, state: S<'_>, req: ExportRequest) -> Result<Clip, String> {
    let src = state.library.get(&req.id).ok_or("Clip not found")?;
    let settings = state.settings.lock().clone();
    let library = state.library.clone();
    let thumbs = state.recorder.thumbs_dir().to_path_buf();

    tauri::async_runtime::spawn_blocking(move || {
        let start = req.start.clamp(0.0, src.duration);
        let end = req.end.clamp(start + 0.5, src.duration.max(start + 0.5));
        let dur = end - start;
        let encoder = ffmpeg::resolve_encoder(&settings.encoder, "h264");

        let (video_kbps, audio_kbps, height) = match req.target_mb {
            Some(mb) => {
                // Leave 8% for container overhead and encoder overshoot.
                let total = (mb as f64 * 8192.0 * 0.92 / dur) as u32;
                let audio = if total < 1200 { 64 } else { 128 };
                let video = total.saturating_sub(audio).max(250);
                let height = match video {
                    0..=900 => Some(480),
                    901..=2500 => Some(720),
                    2501..=6000 => Some(1080),
                    _ => None,
                };
                (video, audio, height)
            }
            None => (settings.video_kbps(), 160, None),
        };

        let folder = src.path.parent().map(PathBuf::from).unwrap_or_else(|| settings.clips_dir.clone());
        let base = src.path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "Clip".into());
        let suffix = match req.target_mb {
            Some(mb) => format!(" ({mb} MB)"),
            None => " (trimmed)".into(),
        };
        let out = library::unique_path(&folder, &format!("{base}{suffix}"), "mp4");

        let id_for_events = req.id.clone();
        let job = ffmpeg::ExportJob {
            input: &src.path,
            output: &out,
            start,
            end,
            encoder,
            video_kbps,
            audio_kbps,
            height,
            fps: settings.fps,
        };
        ffmpeg::export(&job, |p| {
            let _ = app.emit("export-progress", ExportProgress { id: id_for_events.clone(), progress: p });
        })?;

        let id = library::new_id();
        let thumb = thumbs.join(format!("{id}.jpg"));
        let thumb = ffmpeg::thumbnail(&out, &thumb, (dur - 2.0).max(0.0)).ok().map(|_| thumb);
        let duration = ffmpeg::probe_duration(&out).unwrap_or(dur);
        let clip = Clip {
            id,
            size: std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0),
            path: out,
            thumb,
            title: format!("{}{}", src.title, suffix),
            game: src.game.clone(),
            created: chrono::Local::now().timestamp_millis(),
            duration,
            favorite: false,
            kind: "export".into(),
        };
        library.add(clip.clone());
        Ok(clip)
    })
    .await
    .map_err(|e| e.to_string())?
}

// ---------------------------------------------------------------- actions

async fn do_save_clip(app: AppHandle) -> Result<Clip, String> {
    let rec = app.state::<AppState>().recorder.clone();
    let sound = app.state::<AppState>().settings.lock().clip_sound;
    let res = tauri::async_runtime::spawn_blocking(move || rec.save_clip())
        .await
        .map_err(|e| e.to_string())?;
    match &res {
        Ok(clip) => {
            if sound {
                win::chime(true);
            }
            let _ = app.emit("clip-saved", clip);
        }
        Err(e) => {
            let _ = app.emit("notice", e);
        }
    }
    res
}

async fn do_toggle_recording(app: AppHandle) -> Result<Option<Clip>, String> {
    let rec = app.state::<AppState>().recorder.clone();
    let sound = app.state::<AppState>().settings.lock().clip_sound;
    let was_recording = rec.status().recording;
    if sound {
        win::chime(!was_recording);
    }
    let res = tauri::async_runtime::spawn_blocking(move || rec.toggle_recording())
        .await
        .map_err(|e| e.to_string())?;
    match &res {
        Ok(Some(clip)) => {
            let _ = app.emit("clip-saved", clip);
        }
        Ok(None) => {}
        Err(e) => {
            let _ = app.emit("notice", e);
        }
    }
    res
}

fn register_hotkeys(app: &AppHandle, s: &Settings) -> Result<(), String> {
    let clip = Shortcut::from_str(&s.clip_hotkey).map_err(|_| format!("\"{}\" is not a valid key", s.clip_hotkey))?;
    let record = Shortcut::from_str(&s.record_hotkey).map_err(|_| format!("\"{}\" is not a valid key", s.record_hotkey))?;
    if clip == record {
        return Err("Use different keys to save clips and to record".into());
    }
    let gs = app.global_shortcut();
    let _ = gs.unregister_all();
    for (shortcut, is_clip) in [(clip, true), (record, false)] {
        gs.on_shortcut(shortcut, move |app, _sc, ev| {
            if ev.state() != ShortcutState::Pressed {
                return;
            }
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                let _ = if is_clip {
                    do_save_clip(app).await.map(|_| ())
                } else {
                    do_toggle_recording(app).await.map(|_| ())
                };
            });
        })
        .map_err(|e| format!("Could not use {shortcut}: another app may already use it ({e})"))?;
    }
    Ok(())
}

/// Shows the main window. The window is destroyed on close so its WebView2 process
/// does not use memory while the player is in a game, so it is created again here.
fn show_main(app: &AppHandle) {
    let window = match app.get_webview_window("main") {
        Some(w) => w,
        None => {
            let Some(config) = app.config().app.windows.iter().find(|w| w.label == "main").cloned() else {
                return;
            };
            match WebviewWindowBuilder::from_config(app, &config).and_then(|b| b.build()) {
                Ok(w) => w,
                Err(e) => {
                    log::warn!("main window: {e}");
                    return;
                }
            }
        }
    };
    let _ = window.unminimize();
    let _ = window.show();
    let _ = window.set_focus();
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Rewind", true, None::<&str>)?;
    let clip = MenuItem::with_id(app, "clip", "Save clip", true, None::<&str>)?;
    let record = MenuItem::with_id(app, "record", "Start recording", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let sep = PredefinedMenuItem::separator(app)?;
    let menu = Menu::with_items(app, &[&open, &clip, &record, &sep, &quit])?;
    *app.state::<AppState>().record_item.lock() = Some(record);

    TrayIconBuilder::with_id("main")
        .icon(app.default_window_icon().cloned().expect("app icon"))
        .tooltip("Rewind")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, ev| match ev.id().as_ref() {
            "open" => show_main(app),
            "clip" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = do_save_clip(app).await;
                });
            }
            "record" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    let _ = do_toggle_recording(app).await;
                });
            }
            "quit" => {
                app.state::<AppState>().recorder.shutdown();
                app.exit(0);
            }
            _ => {}
        })
        .on_tray_icon_event(|tray, ev| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = ev {
                show_main(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_main(app)))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .setup(|app| {
            let handle = app.handle().clone();
            let data_dir = app.path().app_local_data_dir()?;
            std::fs::create_dir_all(&data_dir)?;
            let settings_file = data_dir.join("settings.json");
            let settings = Settings::load(&settings_file);
            let _ = std::fs::create_dir_all(&settings.clips_dir);

            let library = Arc::new(Library::open(data_dir.join("library.json")));
            let recorder = Recorder::new(&data_dir, library.clone(), settings.clone());
            {
                let handle = handle.clone();
                recorder.set_listener(move |st: Status| {
                    if let Some(tray) = handle.tray_by_id("main") {
                        let tip = if st.recording {
                            "Rewind: recording"
                        } else if st.buffering {
                            "Rewind: replay on"
                        } else {
                            "Rewind: replay off"
                        };
                        let _ = tray.set_tooltip(Some(tip));
                    }
                    if let Some(item) = handle.state::<AppState>().record_item.lock().as_ref() {
                        let _ = item.set_text(if st.recording { "Stop recording" } else { "Start recording" });
                    }
                    let _ = handle.emit("status", st);
                });
            }

            app.manage(AppState {
                settings: Mutex::new(settings.clone()),
                settings_file,
                library,
                recorder: recorder.clone(),
                record_item: Mutex::new(None),
            });

            build_tray(&handle)?;
            if let Err(e) = register_hotkeys(&handle, &settings) {
                log::warn!("hotkeys: {e}");
            }

            // Probing encoders takes a moment; do it off the UI thread, then start capture.
            std::thread::spawn(move || {
                let _ = ffmpeg::available_encoders();
                recorder.apply(settings, true);
            });

            let minimized = std::env::args().any(|a| a == "--minimized");
            if !minimized {
                show_main(&handle);
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_status,
            get_settings,
            save_settings,
            get_devices,
            list_clips,
            save_clip,
            toggle_recording,
            rename_clip,
            set_favorite,
            delete_clip,
            reveal_clip,
            open_clips_folder,
            copy_clip,
            filmstrip,
            export_clip,
        ])
        .build(tauri::generate_context!())
        .expect("error while building Rewind")
        .run(|_app, event| {
            // Closing the window keeps Rewind running in the tray, like other clipping apps.
            // Only an exit with a code (Quit in the tray menu) stops the app.
            if let RunEvent::ExitRequested { code: None, api, .. } = event {
                api.prevent_exit();
            }
        });
}
