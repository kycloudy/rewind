//! Replay buffer and session recording.
//!
//! One FFmpeg process captures the screen (Desktop Duplication via `ddagrab`),
//! encodes on the GPU, and writes a ring of 2-second MPEG-TS segments. Saving
//! a clip joins the newest segments without re-encoding. Session recording
//! copies each finished segment aside and joins them when the session stops.
//! Both features share the one encoder, so recording costs no extra GPU time.

use crate::audio::{AudioConfig, AudioPump};
use crate::ffmpeg::{self, Vendor};
use crate::library::{self, Clip, Library};
use crate::settings::Settings;
use crate::win;
use parking_lot::Mutex;
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

type Listener = Box<dyn Fn(Status) + Send + Sync>;

const SEGMENT_SECS: f64 = 2.0;
/// A run that dies sooner than this counts as a failed start.
const START_GRACE: Duration = Duration::from_secs(6);
/// In games-only mode, keep capturing this long after the game leaves the
/// foreground, so a quick alt-tab does not wipe the buffer.
const GAME_LINGER: Duration = Duration::from_secs(60);

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub buffering: bool,
    /// Games-only mode is on and no game is in front, so capture is paused.
    pub waiting_for_game: bool,
    pub recording: bool,
    /// Unix ms when the current recording started.
    pub recording_since: Option<i64>,
    pub encoder: Option<String>,
    pub clip_seconds: u32,
    pub saving: bool,
    pub error: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Pipeline {
    /// Frames stay on the GPU from capture to encoder, scaled there if needed.
    ZeroCopy,
    /// Frames are copied to system memory (needed for x264, and the fallback for GPU scaling).
    Download,
}

struct Run {
    id: u64,
    child: Child,
    audio: Option<AudioPump>,
    started: Instant,
    dir: PathBuf,
    strategies: Vec<(&'static str, Pipeline)>,
    strategy: usize,
}

struct Session {
    dir: PathBuf,
    run_id: u64,
    from_time: f64,
    seen: HashSet<String>,
    parts: Vec<PathBuf>,
    started_ms: i64,
    game: String,
}

struct Inner {
    settings: Settings,
    run: Option<Run>,
    next_run_id: u64,
    session: Option<Session>,
    status: Status,
    last_game: Option<String>,
    /// When a full-screen game was last in the foreground.
    game_seen: Option<Instant>,
    restarts: u32,
}

impl Inner {
    fn capture_wanted(&self) -> bool {
        !self.settings.games_only
            || self.session.is_some()
            || self.game_seen.is_some_and(|t| t.elapsed() < GAME_LINGER)
    }
}

pub struct Recorder {
    buffer_root: PathBuf,
    sessions_root: PathBuf,
    work_dir: PathBuf,
    thumbs_dir: PathBuf,
    library: Arc<Library>,
    inner: Mutex<Inner>,
    saving: AtomicBool,
    listener: Mutex<Option<Listener>>,
}

#[derive(Debug, Clone)]
struct Segment {
    file: PathBuf,
    start: f64,
    end: f64,
}

impl Recorder {
    pub fn new(data_dir: &Path, library: Arc<Library>, settings: Settings) -> Arc<Self> {
        let buffer_root = data_dir.join("buffer");
        let _ = std::fs::remove_dir_all(&buffer_root);
        let rec = Arc::new(Self {
            buffer_root,
            sessions_root: data_dir.join("sessions"),
            work_dir: data_dir.join("work"),
            thumbs_dir: data_dir.join("thumbs"),
            library,
            inner: Mutex::new(Inner {
                status: Status { clip_seconds: settings.clip_seconds, ..Default::default() },
                settings,
                run: None,
                next_run_id: 1,
                session: None,
                last_game: None,
                game_seen: None,
                restarts: 0,
            }),
            saving: AtomicBool::new(false),
            listener: Mutex::new(None),
        });
        for d in [&rec.buffer_root, &rec.sessions_root, &rec.work_dir, &rec.thumbs_dir] {
            let _ = std::fs::create_dir_all(d);
        }
        let weak = Arc::downgrade(&rec);
        std::thread::Builder::new()
            .name("recorder-supervisor".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(500));
                match weak.upgrade() {
                    Some(r) => r.tick(),
                    None => break,
                }
            })
            .ok();
        rec
    }

    pub fn thumbs_dir(&self) -> &Path {
        &self.thumbs_dir
    }

    pub fn set_listener(&self, f: impl Fn(Status) + Send + Sync + 'static) {
        *self.listener.lock() = Some(Box::new(f));
    }

    fn emit(&self, status: Status) {
        if let Some(f) = self.listener.lock().as_ref() {
            f(status);
        }
    }

    pub fn status(&self) -> Status {
        self.inner.lock().status.clone()
    }

    // ------------------------------------------------------------ lifecycle

    /// Applies new settings. Restarts capture only when a capture setting changed.
    pub fn apply(&self, settings: Settings, force: bool) {
        let mut inner = self.inner.lock();
        let restart = force || inner.run.is_none() || inner.settings.capture_changed(&settings);
        inner.settings = settings;
        inner.status.clip_seconds = inner.settings.clip_seconds;
        if restart {
            inner.restarts = 0;
            self.restart_locked(&mut inner, 0);
        }
        let st = inner.status.clone();
        drop(inner);
        self.emit(st);
    }

    pub fn shutdown(&self) {
        let mut inner = self.inner.lock();
        if let Some(run) = inner.run.take() {
            stop_run(run);
        }
    }

    fn restart_locked(&self, inner: &mut Inner, strategy: usize) {
        let prev = inner.run.take();
        let strategies = prev.as_ref().filter(|_| strategy > 0).map(|r| r.strategies.clone());
        if let Some(run) = prev {
            stop_run(run);
        }
        inner.status.buffering = false;
        inner.status.encoder = None;
        inner.status.waiting_for_game = false;
        if !inner.settings.buffer_enabled {
            inner.status.error = None;
            return;
        }
        if !inner.capture_wanted() {
            inner.status.error = None;
            inner.status.waiting_for_game = true;
            return;
        }
        let strategies = strategies.unwrap_or_else(|| plan(&inner.settings));
        let Some(&(encoder, pipeline)) = strategies.get(strategy) else {
            inner.status.error = Some("Could not start screen capture. Update your graphics driver, then turn the replay buffer off and on.".into());
            return;
        };
        let id = inner.next_run_id;
        inner.next_run_id += 1;
        match spawn(&self.buffer_root, id, &inner.settings, encoder, pipeline) {
            Ok((child, audio, dir)) => {
                log::info!("capture started: {encoder} {pipeline:?}");
                inner.status.buffering = true;
                inner.status.encoder = Some(label(encoder).into());
                inner.status.error = None;
                inner.run = Some(Run { id, child, audio, started: Instant::now(), dir, strategies, strategy });
            }
            Err(e) => inner.status.error = Some(e),
        }
    }

    /// Runs every 500 ms: watches FFmpeg, copies session segments, tracks the game.
    fn tick(&self) {
        let mut inner = self.inner.lock();
        let mut changed = false;

        if let Some((exe, title)) = win::foreground_app() {
            let me = std::env::current_exe()
                .ok()
                .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_lowercase()))
                .unwrap_or_default();
            let exe_l = exe.to_lowercase();
            if !exe.is_empty() && exe_l != me && exe_l != "explorer" && exe_l != "searchhost" {
                inner.last_game = Some(win::game_name(&exe, &title));
                if win::looks_like_game(&exe, win::foreground_full_screen()) {
                    inner.game_seen = Some(Instant::now());
                }
            }
        }

        // Games-only mode: start when a game comes to the front, pause when none has been there for a while.
        if inner.settings.buffer_enabled && inner.capture_wanted() == inner.status.waiting_for_game {
            log::info!("{} capture", if inner.status.waiting_for_game { "resuming" } else { "pausing" });
            inner.restarts = 0;
            self.restart_locked(&mut inner, 0);
            changed = true;
        }

        let exited = match inner.run.as_mut() {
            Some(run) => match run.child.try_wait() {
                Ok(Some(_)) => Some((run.started.elapsed(), run.strategy, tail(&run.dir.join("ffmpeg.log")))),
                _ => None,
            },
            None => None,
        };
        if let Some((age, strategy, log_tail)) = exited {
            log::warn!("ffmpeg exited after {age:?}: {log_tail}");
            if age < START_GRACE {
                self.restart_locked(&mut inner, strategy + 1);
            } else if inner.restarts < 5 {
                inner.restarts += 1;
                self.restart_locked(&mut inner, strategy);
            } else {
                inner.run = None;
                inner.status.buffering = false;
                inner.status.error = Some(format!("Screen capture stopped: {log_tail}"));
            }
            changed = true;
        }

        let run_info = inner.run.as_ref().map(|r| (r.id, r.dir.clone()));
        if let (Some(session), Some((run_id, dir))) = (inner.session.as_mut(), run_info) {
            copy_new_parts(session, run_id, &dir);
        }

        let st = inner.status.clone();
        drop(inner);
        if changed {
            self.emit(st);
        }
    }

    fn current_game(&self) -> String {
        let inner = self.inner.lock();
        inner.last_game.clone().unwrap_or_else(|| "Desktop".into())
    }

    fn run_dir(&self) -> Option<(u64, PathBuf)> {
        self.inner.lock().run.as_ref().map(|r| (r.id, r.dir.clone()))
    }

    // ------------------------------------------------------------ clipping

    /// Saves the last `clip_seconds` of gameplay.
    pub fn save_clip(&self) -> Result<Clip, String> {
        if self.saving.swap(true, Ordering::SeqCst) {
            return Err("Already saving a clip".into());
        }
        let res = self.save_clip_inner();
        self.saving.store(false, Ordering::SeqCst);
        res
    }

    fn save_clip_inner(&self) -> Result<Clip, String> {
        let game = self.current_game();
        let (seconds, clips_dir) = {
            let i = self.inner.lock();
            (i.settings.clip_seconds as f64, i.settings.clips_dir.clone())
        };
        if self.inner.lock().status.waiting_for_game {
            return Err("Rewind records only while a game is open. You can change this in Settings.".into());
        }
        let (_, dir) = self.run_dir().ok_or("The replay buffer is off. Turn it on in Settings.")?;
        self.set_saving(true);

        // Include the moment of the key press: wait for the segment in progress to close.
        let before = read_list(&dir).last().map(|s| s.end).unwrap_or(0.0);
        wait_for(|| read_list(&dir).last().map(|s| s.end).unwrap_or(0.0) > before, Duration::from_secs(5));
        let segs = read_list(&dir);
        if segs.is_empty() {
            self.set_saving(false);
            return Err("Nothing recorded yet. Try again in a few seconds.".into());
        }
        let mut picked = Vec::new();
        let mut total = 0.0;
        for s in segs.iter().rev() {
            if total >= seconds - 0.5 {
                break;
            }
            total += s.end - s.start;
            picked.push(s.clone());
        }
        picked.reverse();

        // Copy first so the ring buffer cannot overwrite a segment mid-join.
        let tmp = self.work_dir.join(library::new_id());
        std::fs::create_dir_all(&tmp).map_err(|e| e.to_string())?;
        let mut parts = Vec::new();
        for (i, s) in picked.iter().enumerate() {
            let dst = tmp.join(format!("{i:04}.ts"));
            if std::fs::copy(&s.file, &dst).is_ok() {
                parts.push(dst);
            }
        }
        let res = self.finish(&parts, total, &game, &clips_dir, "clip", chrono::Local::now());
        let _ = std::fs::remove_dir_all(&tmp);
        self.set_saving(false);
        res
    }

    fn set_saving(&self, on: bool) {
        let st = {
            let mut i = self.inner.lock();
            i.status.saving = on;
            i.status.clone()
        };
        self.emit(st);
    }

    fn finish(
        &self,
        parts: &[PathBuf],
        duration: f64,
        game: &str,
        clips_dir: &Path,
        kind: &str,
        when: chrono::DateTime<chrono::Local>,
    ) -> Result<Clip, String> {
        if parts.is_empty() {
            return Err("No video to save".into());
        }
        let folder = clips_dir.join(library::safe_file_stem(game));
        std::fs::create_dir_all(&folder).map_err(|e| format!("Could not create {}: {e}", folder.display()))?;
        let stem = format!(
            "{} {}{}",
            library::safe_file_stem(game),
            when.format("%Y-%m-%d %H-%M-%S"),
            if kind == "recording" { " (recording)" } else { "" }
        );
        let out = library::unique_path(&folder, &stem, "mp4");
        ffmpeg::concat(parts, &out, &self.work_dir)?;

        let duration = ffmpeg::probe_duration(&out).unwrap_or(duration);
        let id = library::new_id();
        let thumb = self.thumbs_dir.join(format!("{id}.jpg"));
        let thumb_at = if kind == "clip" { (duration - 2.0).max(0.0) } else { duration.min(5.0) / 2.0 };
        let thumb = ffmpeg::thumbnail(&out, &thumb, thumb_at).ok().map(|_| thumb);

        let clip = Clip {
            id,
            size: std::fs::metadata(&out).map(|m| m.len()).unwrap_or(0),
            path: out,
            thumb,
            title: if kind == "recording" { format!("{game} session") } else { format!("{game} clip") },
            game: game.into(),
            created: when.timestamp_millis(),
            duration,
            favorite: false,
            kind: kind.into(),
        };
        self.library.add(clip.clone());
        Ok(clip)
    }

    // ------------------------------------------------------------ sessions

    pub fn toggle_recording(&self) -> Result<Option<Clip>, String> {
        let recording = self.inner.lock().session.is_some();
        if recording {
            self.stop_recording().map(Some)
        } else {
            self.start_recording().map(|_| None)
        }
    }

    pub fn start_recording(&self) -> Result<(), String> {
        let game = self.current_game();
        let mut inner = self.inner.lock();
        if inner.session.is_some() {
            return Ok(());
        }
        // Recording counts as a request to capture, even outside a game.
        if inner.status.waiting_for_game {
            inner.game_seen = Some(Instant::now());
            self.restart_locked(&mut inner, 0);
        }
        let (run_id, dir) = inner
            .run
            .as_ref()
            .map(|r| (r.id, r.dir.clone()))
            .ok_or("Turn on the replay buffer to record. Recording uses the same capture.")?;
        let from_time = read_list(&dir).last().map(|s| s.end).unwrap_or(0.0);
        let sdir = self.sessions_root.join(library::new_id());
        std::fs::create_dir_all(&sdir).map_err(|e| e.to_string())?;
        let now = chrono::Local::now().timestamp_millis();
        inner.session = Some(Session {
            dir: sdir,
            run_id,
            from_time,
            seen: HashSet::new(),
            parts: Vec::new(),
            started_ms: now,
            game,
        });
        inner.status.recording = true;
        inner.status.recording_since = Some(now);
        let st = inner.status.clone();
        drop(inner);
        self.emit(st);
        Ok(())
    }

    pub fn stop_recording(&self) -> Result<Clip, String> {
        let (run_dir, clips_dir) = {
            let i = self.inner.lock();
            if i.session.is_none() {
                return Err("Not recording".into());
            }
            (i.run.as_ref().map(|r| (r.id, r.dir.clone())), i.settings.clips_dir.clone())
        };
        // Wait for the segment in progress so the last seconds are kept.
        if let Some((_, dir)) = &run_dir {
            let before = read_list(dir).last().map(|s| s.end).unwrap_or(0.0);
            wait_for(|| read_list(dir).last().map(|s| s.end).unwrap_or(0.0) > before, Duration::from_secs(5));
        }
        let mut session = {
            let mut i = self.inner.lock();
            i.status.recording = false;
            i.status.recording_since = None;
            i.session.take().ok_or("Not recording")?
        };
        if let Some((id, dir)) = &run_dir {
            copy_new_parts(&mut session, *id, dir);
        }
        self.emit(self.status());

        let when = chrono::DateTime::from_timestamp_millis(session.started_ms)
            .map(|d| d.with_timezone(&chrono::Local))
            .unwrap_or_else(chrono::Local::now);
        let est = session.parts.len() as f64 * SEGMENT_SECS;
        let res = self.finish(&session.parts, est, &session.game, &clips_dir, "recording", when);
        let _ = std::fs::remove_dir_all(&session.dir);
        res
    }
}

// ---------------------------------------------------------------- helpers

fn label(encoder: &str) -> &'static str {
    ffmpeg::ENCODERS.iter().find(|e| e.name == encoder).map(|e| e.label).unwrap_or("CPU (x264)")
}

/// Capture strategies to try, fastest first.
fn plan(s: &Settings) -> Vec<(&'static str, Pipeline)> {
    let enc = ffmpeg::resolve_encoder(&s.encoder);
    let mut out = Vec::new();
    if ffmpeg::vendor(enc) != Vendor::Cpu {
        out.push((enc, Pipeline::ZeroCopy));
        out.push((enc, Pipeline::Download));
    }
    out.push(("libx264", Pipeline::Download));
    out
}

fn spawn(
    root: &Path,
    id: u64,
    s: &Settings,
    encoder: &'static str,
    pipeline: Pipeline,
) -> Result<(Child, Option<AudioPump>, PathBuf), String> {
    let dir = root.join(format!("run-{id}"));
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    // Keep a little more than one clip of history, plus slack for saving.
    let wrap = (s.clip_seconds as f64 / SEGMENT_SECS).ceil() as u32 + 6;
    let audio = s.any_audio();

    let mut cmd = if audio { ffmpeg::command_with_stdin() } else { ffmpeg::command() };
    cmd.args(["-loglevel", "warning", "-nostats"]);
    cmd.args([
        "-f",
        "lavfi",
        "-i",
        &format!("ddagrab=output_idx={}:framerate={}:draw_mouse=1", s.monitor, s.fps),
    ]);
    if audio {
        cmd.args(["-thread_queue_size", "1024", "-f", "f32le", "-ar", "48000", "-ac", "2", "-i", "pipe:0"]);
    }

    let vendor = ffmpeg::vendor(encoder);
    let mut vf: Vec<String> = Vec::new();
    match pipeline {
        Pipeline::ZeroCopy => {
            if let Some(h) = s.target_height() {
                vf.push(format!("scale_d3d11=width=-2:height='min({h},ih)':format=nv12"));
            }
            if vendor == Vendor::Intel {
                vf.push("hwmap=derive_device=qsv".into());
                vf.push("format=qsv".into());
            }
        }
        Pipeline::Download => {
            vf.push("hwdownload".into());
            vf.push("format=bgra".into());
            if let Some(h) = s.target_height() {
                vf.push(format!("scale=-2:'min({h},ih)':flags=bilinear"));
            }
            vf.push(if vendor == Vendor::Cpu { "format=yuv420p".into() } else { "format=nv12".into() });
        }
    }
    if !vf.is_empty() {
        cmd.args(["-vf", &vf.join(",")]);
    }
    cmd.args(["-map", "0:v"]);
    if audio {
        cmd.args(["-map", "1:a", "-c:a", "aac", "-b:a", "160k", "-af", "aresample=async=1000"]);
    }
    let native_height = win::monitors().get(s.monitor as usize).map(|m| m.height.max(0) as u32);
    cmd.args(ffmpeg::encoder_args(encoder, s.video_kbps_at(native_height), s.fps, true));
    cmd.args(["-force_key_frames", "expr:gte(t,n_forced*2)"]);
    cmd.args(["-f", "segment", "-segment_time", "2", "-segment_format", "mpegts", "-reset_timestamps", "1"]);
    cmd.args(["-segment_wrap", &wrap.to_string()]);
    cmd.args(["-segment_list_type", "csv", "-segment_list_size", &(wrap - 2).to_string(), "-segment_list"]);
    cmd.arg(dir.join("list.csv"));
    cmd.arg(dir.join("seg_%03d.ts"));

    let log = std::fs::File::create(dir.join("ffmpeg.log")).map_err(|e| e.to_string())?;
    cmd.stdout(Stdio::null()).stderr(Stdio::from(log));
    cmd.stdin(if audio { Stdio::piped() } else { Stdio::null() });

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Could not start FFmpeg ({}): {e}", ffmpeg::path().display()))?;
    let pump = if audio {
        child.stdin.take().map(|stdin| {
            AudioPump::start(
                stdin,
                AudioConfig {
                    system: s.system_audio,
                    mic: s.mic,
                    mic_device: s.mic_device.clone(),
                    mic_volume: s.mic_volume,
                    mic_gate: s.mic_gate.then_some(s.mic_gate_threshold),
                },
            )
        })
    } else {
        None
    };
    Ok((child, pump, dir))
}

fn stop_run(mut run: Run) {
    let _ = run.child.kill();
    let _ = run.child.wait();
    if let Some(a) = run.audio.take() {
        a.stop();
    }
    let dir = run.dir.clone();
    // Delete the old ring buffer a moment later, once file handles are closed.
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(2));
        let _ = std::fs::remove_dir_all(dir);
    });
}

fn read_list(dir: &Path) -> Vec<Segment> {
    let Ok(text) = std::fs::read_to_string(dir.join("list.csv")) else {
        return Vec::new();
    };
    let mut segs: Vec<Segment> = text
        .lines()
        .filter_map(|l| {
            let mut it = l.split(',');
            let file = it.next()?.trim().trim_matches('"');
            let start = it.next()?.trim().parse().ok()?;
            let end = it.next()?.trim().parse().ok()?;
            Some(Segment { file: dir.join(file), start, end })
        })
        .collect();
    segs.sort_by(|a, b| a.start.total_cmp(&b.start));
    segs
}

fn copy_new_parts(session: &mut Session, run_id: u64, dir: &Path) {
    if session.run_id != run_id {
        // Capture restarted: the new run's timeline starts at zero.
        session.run_id = run_id;
        session.from_time = 0.0;
    }
    for seg in read_list(dir) {
        if seg.start + 0.01 < session.from_time {
            continue;
        }
        let key = format!("{run_id}:{:.3}", seg.start);
        if session.seen.contains(&key) {
            continue;
        }
        let dst = session.dir.join(format!("part_{:06}.ts", session.parts.len()));
        if std::fs::copy(&seg.file, &dst).is_ok() {
            session.seen.insert(key);
            session.parts.push(dst);
        }
    }
}

fn wait_for(mut cond: impl FnMut() -> bool, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if cond() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(80));
    }
    false
}

fn tail(path: &Path) -> String {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|t| t.lines().rev().find(|l| !l.trim().is_empty()).map(str::to_string))
        .unwrap_or_else(|| "FFmpeg exited".into())
}
