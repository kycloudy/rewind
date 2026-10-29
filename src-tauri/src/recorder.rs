//! Replay buffer and session recording.
//!
//! One FFmpeg process captures the screen (Desktop Duplication via `ddagrab`),
//! encodes on the GPU, and writes MPEG-TS to stdout. `Ring` keeps the newest
//! 2-second chunks of that stream in memory, so the buffer writes nothing to
//! disk. Saving a clip remuxes the newest chunks to MP4 without re-encoding.
//! Session recording appends each finished chunk to a file and joins the files
//! when the session stops. Both features share the one encoder, so recording
//! costs no extra GPU time.

use crate::audio::{AudioConfig, AudioPump};
use crate::ffmpeg::{self, Vendor};
use crate::games::{self, Installed};
use crate::library::{self, Clip, Library};
use crate::ring::{self, Ring};
use crate::settings::Settings;
use crate::win;
use parking_lot::Mutex;
use serde::Serialize;
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter};
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
/// An app must look like a game for this long before capture starts.
const GAME_DWELL: Duration = Duration::from_secs(3);
/// How often to look for newly installed games.
const GAME_SCAN_EVERY: Duration = Duration::from_secs(60);
/// How often to list the running processes.
const PROCESS_SCAN_EVERY: Duration = Duration::from_secs(10);

/// The game that keeps capture on.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Game {
    pub name: String,
    /// Exe file name without ".exe".
    pub exe: String,
    /// True when only the full-screen test matched, so the app may not be a game.
    pub guessed: bool,
}

#[derive(Debug, Clone, Serialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub buffering: bool,
    /// Games-only mode is on and no game is in front, so capture is paused.
    pub waiting_for_game: bool,
    pub game: Option<Game>,
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
    ring: Arc<Ring>,
    /// Last line FFmpeg wrote to stderr.
    log: Arc<Mutex<String>>,
    strategies: Vec<(&'static str, Pipeline)>,
    strategy: usize,
}

struct Session {
    dir: PathBuf,
    run_id: u64,
    /// Seq of the last chunk written.
    last_seq: u64,
    /// The part file for the current run. A capture restart starts a new part.
    part: Option<BufWriter<File>>,
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
    /// The last game in the foreground. Clips are named after it.
    last_game: Option<String>,
    /// When a game was last in the foreground.
    game_seen: Option<Instant>,
    /// The app that looks like a game now, and since when.
    candidate: Option<(String, Instant)>,
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
    sessions_root: PathBuf,
    work_dir: PathBuf,
    thumbs_dir: PathBuf,
    library: Arc<Library>,
    inner: Mutex<Inner>,
    /// Games that launchers installed.
    installed: Mutex<Arc<Vec<Installed>>>,
    /// Exe paths of the running processes.
    processes: Mutex<Arc<Vec<String>>>,
    saving: AtomicBool,
    listener: Mutex<Option<Listener>>,
}

impl Recorder {
    pub fn new(data_dir: &Path, library: Arc<Library>, settings: Settings) -> Arc<Self> {
        // Older versions kept the ring buffer here on disk.
        let _ = std::fs::remove_dir_all(data_dir.join("buffer"));
        let rec = Arc::new(Self {
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
                candidate: None,
                restarts: 0,
            }),
            installed: Mutex::new(Arc::new(Vec::new())),
            processes: Mutex::new(Arc::new(Vec::new())),
            saving: AtomicBool::new(false),
            listener: Mutex::new(None),
        });
        for d in [&rec.sessions_root, &rec.work_dir, &rec.thumbs_dir] {
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
        let weak = Arc::downgrade(&rec);
        std::thread::Builder::new()
            .name("game-scan".into())
            .spawn(move || loop {
                let found = Arc::new(games::scan());
                match weak.upgrade() {
                    Some(r) => *r.installed.lock() = found,
                    None => break,
                }
                std::thread::sleep(GAME_SCAN_EVERY);
            })
            .ok();
        let weak = Arc::downgrade(&rec);
        std::thread::Builder::new()
            .name("process-scan".into())
            .spawn(move || loop {
                let found = Arc::new(win::processes());
                match weak.upgrade() {
                    Some(r) => *r.processes.lock() = found,
                    None => break,
                }
                std::thread::sleep(PROCESS_SCAN_EVERY);
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
        match spawn(&inner.settings, encoder, pipeline) {
            Ok((child, audio, ring, log)) => {
                log::info!("capture started: {encoder} {pipeline:?}");
                inner.status.buffering = true;
                inner.status.encoder = Some(label(encoder).into());
                inner.status.error = None;
                inner.run = Some(Run { id, child, audio, started: Instant::now(), ring, log, strategies, strategy });
            }
            Err(e) => inner.status.error = Some(e),
        }
    }

    /// Runs every 500 ms: watches FFmpeg, writes session chunks, tracks the game.
    fn tick(&self) {
        let installed = self.installed.lock().clone();
        let mut inner = self.inner.lock();
        let mut changed = false;

        let me = std::env::current_exe()
            .ok()
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().into_owned()))
            .unwrap_or_default();
        let (user, blocked) = (&inner.settings.game_apps, &inner.settings.not_games);
        let in_front = win::foreground().filter(|fg| !fg.exe.eq_ignore_ascii_case(&me)).and_then(|fg| {
            let on_screen = fg.monitor == Some(inner.settings.monitor);
            on_screen.then(|| games::detect(&fg, &installed, user, blocked)).flatten().map(|g| (fg, g))
        });
        // A game that runs behind other windows still counts.
        let found = in_front.or_else(|| games::running(&self.processes.lock(), &installed, user, blocked));
        match found {
            Some((fg, game)) => {
                // A new match must hold for a moment, so a short full-screen video or overlay does not start capture.
                let since = match &inner.candidate {
                    Some((exe, t)) if *exe == fg.exe => *t,
                    _ => Instant::now(),
                };
                inner.candidate = Some((fg.exe.clone(), since));
                let capturing = inner.game_seen.is_some_and(|t| t.elapsed() < GAME_LINGER);
                if capturing || since.elapsed() >= GAME_DWELL {
                    let now = Game { name: game.name.clone(), exe: fg.exe.clone(), guessed: game.guessed };
                    if inner.status.game.as_ref() != Some(&now) {
                        inner.status.game = Some(now);
                        changed = true;
                    }
                    inner.last_game = Some(game.name);
                    inner.game_seen = Some(Instant::now());
                }
            }
            None => inner.candidate = None,
        }

        if inner.status.game.is_some() && !inner.game_seen.is_some_and(|t| t.elapsed() < GAME_LINGER) {
            inner.status.game = None;
            changed = true;
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
                Ok(Some(_)) => Some((run.started.elapsed(), run.strategy, tail(&run.log))),
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

        let run_info = inner.run.as_ref().map(|r| (r.id, r.ring.clone()));
        if let (Some(session), Some((run_id, ring))) = (inner.session.as_mut(), run_info) {
            write_new_chunks(session, run_id, &ring);
        }

        let st = inner.status.clone();
        drop(inner);
        if changed {
            self.emit(st);
        }
    }

    /// Stops treating `exe` as the current game, so games-only mode pauses at once.
    pub fn forget_game(&self, exe: &str) {
        let mut inner = self.inner.lock();
        if !inner.status.game.as_ref().is_some_and(|g| g.exe.eq_ignore_ascii_case(exe)) || inner.session.is_some() {
            return;
        }
        inner.status.game = None;
        inner.game_seen = None;
        inner.candidate = None;
        inner.last_game = None;
        let st = inner.status.clone();
        drop(inner);
        self.emit(st);
    }

    fn current_game(&self) -> String {
        let inner = self.inner.lock();
        inner.last_game.clone().unwrap_or_else(|| "Desktop".into())
    }

    fn run_ring(&self) -> Option<(u64, Arc<Ring>)> {
        self.inner.lock().run.as_ref().map(|r| (r.id, r.ring.clone()))
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
        let (_, ring) = self.run_ring().ok_or("The replay buffer is off. Turn it on in Settings.")?;
        self.set_saving(true);

        // Include the moment of the key press: wait for the chunk in progress to close.
        ring.wait_past(ring.last_seq(), Duration::from_secs(5));
        let chunks = ring.last(seconds);
        if chunks.is_empty() {
            self.set_saving(false);
            return Err("Nothing recorded yet. Try again in a few seconds.".into());
        }
        let total = chunks.iter().map(|c| c.end - c.start).sum();
        let headers = ring.headers();
        let res = self.finish(
            |out| ffmpeg::remux_ts(out, move |w| ring::write_ts(w, &headers, &chunks)),
            total,
            &game,
            &clips_dir,
            "clip",
            chrono::Local::now(),
        );
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

    /// Writes the video with `make`, then adds it to the library.
    fn finish(
        &self,
        make: impl FnOnce(&Path) -> Result<(), String>,
        duration: f64,
        game: &str,
        clips_dir: &Path,
        kind: &str,
        when: chrono::DateTime<chrono::Local>,
    ) -> Result<Clip, String> {
        let folder = clips_dir.join(library::safe_file_stem(game));
        std::fs::create_dir_all(&folder).map_err(|e| format!("Could not create {}: {e}", folder.display()))?;
        let stem = format!(
            "{} {}{}",
            library::safe_file_stem(game),
            when.format("%Y-%m-%d %H-%M-%S"),
            if kind == "recording" { " (recording)" } else { "" }
        );
        let out = library::unique_path(&folder, &stem, "mp4");
        make(&out)?;

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
            trim: None,
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
        let (run_id, last_seq) = inner
            .run
            .as_ref()
            .map(|r| (r.id, r.ring.last_seq()))
            .ok_or("Turn on the replay buffer to record. Recording uses the same capture.")?;
        let sdir = self.sessions_root.join(library::new_id());
        std::fs::create_dir_all(&sdir).map_err(|e| e.to_string())?;
        let now = chrono::Local::now().timestamp_millis();
        inner.session = Some(Session {
            dir: sdir,
            run_id,
            last_seq,
            part: None,
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
        let (run, clips_dir) = {
            let i = self.inner.lock();
            if i.session.is_none() {
                return Err("Not recording".into());
            }
            (i.run.as_ref().map(|r| (r.id, r.ring.clone())), i.settings.clips_dir.clone())
        };
        // Wait for the chunk in progress so the last seconds are kept.
        if let Some((_, ring)) = &run {
            ring.wait_past(ring.last_seq(), Duration::from_secs(5));
        }
        let mut session = {
            let mut i = self.inner.lock();
            i.status.recording = false;
            i.status.recording_since = None;
            i.session.take().ok_or("Not recording")?
        };
        if let Some((id, ring)) = &run {
            write_new_chunks(&mut session, *id, ring);
        }
        close_part(&mut session);
        self.emit(self.status());

        let when = chrono::DateTime::from_timestamp_millis(session.started_ms)
            .map(|d| d.with_timezone(&chrono::Local))
            .unwrap_or_else(chrono::Local::now);
        let res = if session.parts.is_empty() {
            Err("No video to save".into())
        } else {
            let est = (chrono::Local::now().timestamp_millis() - session.started_ms) as f64 / 1000.0;
            let parts = std::mem::take(&mut session.parts);
            self.finish(|out| ffmpeg::concat(&parts, out, &self.work_dir), est, &session.game, &clips_dir, "recording", when)
        };
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
    let h264 = ffmpeg::resolve_encoder(&s.encoder, "h264");
    let wanted = ffmpeg::resolve_encoder(&s.encoder, &s.codec);
    let mut out = Vec::new();
    // If the HEVC or AV1 encoder fails to start, fall back to the same GPU's H.264.
    for enc in [wanted, h264] {
        if ffmpeg::vendor(enc) != Vendor::Cpu && !out.iter().any(|(e, _)| *e == enc) {
            out.push((enc, Pipeline::ZeroCopy));
            out.push((enc, Pipeline::Download));
        }
    }
    out.push(("libx264", Pipeline::Download));
    out
}

type Spawned = (Child, Option<AudioPump>, Arc<Ring>, Arc<Mutex<String>>);

fn spawn(s: &Settings, encoder: &'static str, pipeline: Pipeline) -> Result<Spawned, String> {
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
    cmd.args(["-f", "mpegts", "pipe:1"]);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    cmd.stdin(if audio { Stdio::piped() } else { Stdio::null() });

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("Could not start FFmpeg ({}): {e}", ffmpeg::path().display()))?;
    // Keep one clip of history, plus slack for the chunk that closes while saving.
    let ring = Ring::new(s.clip_seconds as f64 + 2.0 * SEGMENT_SECS);
    if let Some(out) = child.stdout.take() {
        ring.pump(out);
    }
    let log = Arc::new(Mutex::new(String::new()));
    if let Some(err) = child.stderr.take() {
        let log = log.clone();
        std::thread::spawn(move || {
            for line in BufReader::new(err).lines().map_while(Result::ok) {
                if !line.trim().is_empty() {
                    *log.lock() = line;
                }
            }
        });
    }
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
                    mic_denoise: s.mic_denoise,
                },
            )
        })
    } else {
        None
    };
    Ok((child, pump, ring, log))
}

fn stop_run(mut run: Run) {
    let _ = run.child.kill();
    let _ = run.child.wait();
    if let Some(a) = run.audio.take() {
        a.stop();
    }
}

/// Appends chunks that closed since the last call to the session's part file.
fn write_new_chunks(session: &mut Session, run_id: u64, ring: &Ring) {
    if session.run_id != run_id {
        // Capture restarted: the new run's chunks count from 1 and need a new part.
        close_part(session);
        session.run_id = run_id;
        session.last_seq = 0;
    }
    let chunks = ring.after(session.last_seq);
    let Some(last) = chunks.last().map(|c| c.seq) else {
        return;
    };
    let new_part = session.part.is_none();
    if new_part {
        let path = session.dir.join(format!("part_{:04}.ts", session.parts.len()));
        match File::create(&path) {
            Ok(f) => {
                session.parts.push(path);
                session.part = Some(BufWriter::new(f));
            }
            Err(e) => {
                log::warn!("could not create {}: {e}", path.display());
                return;
            }
        }
    }
    // Headers once per part. The chunks also carry FFmpeg's own PAT and PMT.
    let headers = if new_part { ring.headers() } else { Vec::new() };
    if let Some(w) = session.part.as_mut() {
        if let Err(e) = ring::write_ts(w, &headers, &chunks) {
            log::warn!("could not write recording: {e}");
        }
    }
    session.last_seq = last;
}

fn close_part(session: &mut Session) {
    if let Some(mut w) = session.part.take() {
        use std::io::Write;
        let _ = w.flush();
    }
}

fn tail(log: &Mutex<String>) -> String {
    let line = log.lock().clone();
    if line.is_empty() {
        "FFmpeg exited".into()
    } else {
        line
    }
}
