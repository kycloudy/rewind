//! Thin helpers around the bundled ffmpeg.exe.

use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::OnceLock;

static FFMPEG: OnceLock<PathBuf> = OnceLock::new();

/// Finds ffmpeg: next to our exe (Tauri sidecar), then on PATH.
pub fn path() -> &'static Path {
    FFMPEG.get_or_init(|| {
        let exe_name = if cfg!(windows) { "ffmpeg.exe" } else { "ffmpeg" };
        if let Some(dir) = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf)) {
            let p = dir.join(exe_name);
            if p.exists() {
                return p;
            }
        }
        PathBuf::from(exe_name)
    })
}

/// A Command for ffmpeg that never opens a console window.
pub fn command() -> Command {
    let mut cmd = Command::new(path());
    cmd.arg("-hide_banner").arg("-nostdin");
    hide_window(&mut cmd);
    cmd
}

/// Like `command()` but without `-nostdin`, for processes we feed through stdin.
pub fn command_with_stdin() -> Command {
    let mut cmd = Command::new(path());
    cmd.arg("-hide_banner");
    hide_window(&mut cmd);
    cmd
}

#[cfg(windows)]
pub fn hide_window(cmd: &mut Command) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub fn hide_window(_cmd: &mut Command) {}

fn run(cmd: &mut Command) -> Result<(), String> {
    let out = cmd
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .map_err(|e| format!("Could not start FFmpeg: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("unknown error");
        Err(format!("FFmpeg failed: {last}"))
    }
}

// ---------------------------------------------------------------- encoders

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Vendor {
    Nvidia,
    Amd,
    Intel,
    Cpu,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct EncoderInfo {
    pub name: &'static str,
    pub label: &'static str,
}

pub const ENCODERS: [EncoderInfo; 4] = [
    EncoderInfo { name: "h264_nvenc", label: "NVIDIA NVENC" },
    EncoderInfo { name: "h264_amf", label: "AMD AMF" },
    EncoderInfo { name: "h264_qsv", label: "Intel Quick Sync" },
    EncoderInfo { name: "libx264", label: "CPU (x264)" },
];

pub fn vendor(encoder: &str) -> Vendor {
    match encoder {
        "h264_nvenc" => Vendor::Nvidia,
        "h264_amf" => Vendor::Amd,
        "h264_qsv" => Vendor::Intel,
        _ => Vendor::Cpu,
    }
}

static AVAILABLE: OnceLock<Vec<&'static str>> = OnceLock::new();

/// Encoders that can open on this machine, best first. Probed once, in parallel.
pub fn available_encoders() -> &'static [&'static str] {
    AVAILABLE.get_or_init(|| {
        let handles: Vec<_> = ENCODERS
            .iter()
            .map(|e| {
                let name = e.name;
                std::thread::spawn(move || {
                    let ok = run(command()
                        .args(["-loglevel", "error", "-f", "lavfi", "-i", "color=c=black:s=640x360:r=30:d=0.2"])
                        .args(["-pix_fmt", "yuv420p", "-c:v", name, "-f", "null", "-"]))
                    .is_ok();
                    (name, ok)
                })
            })
            .collect();
        handles
            .into_iter()
            .filter_map(|h| h.join().ok())
            .filter(|(_, ok)| *ok)
            .map(|(n, _)| n)
            .collect()
    })
}

pub fn resolve_encoder(choice: &str) -> &'static str {
    let avail = available_encoders();
    if let Some(e) = avail.iter().find(|e| **e == choice) {
        return e;
    }
    avail.first().copied().unwrap_or("libx264")
}

/// Encoder flags for a given bitrate (kbit/s). Shared by capture and export.
pub fn encoder_args(encoder: &str, kbps: u32, fps: u32, live: bool) -> Vec<String> {
    let mut a: Vec<String> = vec!["-c:v".into(), encoder.into()];
    let push = |a: &mut Vec<String>, s: &[&str]| a.extend(s.iter().map(|x| x.to_string()));
    match vendor(encoder) {
        Vendor::Nvidia => {
            push(&mut a, &["-preset", if live { "p3" } else { "p6" }, "-rc", "vbr", "-spatial-aq", "1"]);
            if live {
                push(&mut a, &["-tune", "ll", "-forced-idr", "1"]);
            }
        }
        Vendor::Amd => push(&mut a, &["-quality", if live { "speed" } else { "quality" }, "-rc", "vbr_peak"]),
        Vendor::Intel => push(&mut a, &["-preset", if live { "veryfast" } else { "medium" }]),
        Vendor::Cpu => {
            push(&mut a, &["-preset", if live { "veryfast" } else { "medium" }, "-pix_fmt", "yuv420p"]);
            if live {
                push(&mut a, &["-tune", "zerolatency"]);
            }
        }
    }
    let gop = (fps * 2).to_string();
    a.extend([
        "-b:v".into(), format!("{kbps}k"),
        "-maxrate".into(), format!("{}k", kbps * 3 / 2),
        "-bufsize".into(), format!("{}k", kbps * 2),
        "-g".into(), gop.clone(),
        "-keyint_min".into(), gop,
        "-bf".into(), "0".into(),
    ]);
    a
}

// ---------------------------------------------------------------- clip ops

/// Joins MPEG-TS segments into one faststart MP4 without re-encoding.
pub fn concat(parts: &[PathBuf], out: &Path, work_dir: &Path) -> Result<(), String> {
    let list = work_dir.join(format!("concat-{}.txt", crate::library::new_id()));
    let body: String = parts
        .iter()
        .map(|p| format!("file '{}'\n", p.to_string_lossy().replace('\\', "/").replace('\'', "'\\''")))
        .collect();
    std::fs::write(&list, body).map_err(|e| e.to_string())?;
    let res = run(command()
        .args(["-loglevel", "error", "-y", "-f", "concat", "-safe", "0", "-i"])
        .arg(&list)
        .args(["-map", "0", "-c", "copy", "-movflags", "+faststart"])
        .arg(out));
    let _ = std::fs::remove_file(&list);
    res
}

/// Remuxes an MPEG-TS stream, written to FFmpeg's stdin by `feed`, into a faststart MP4.
pub fn remux_ts(
    out: &Path,
    feed: impl FnOnce(&mut std::process::ChildStdin) -> std::io::Result<()> + Send + 'static,
) -> Result<(), String> {
    let mut child = command_with_stdin()
        .args(["-loglevel", "error", "-y", "-f", "mpegts", "-i", "pipe:0"])
        .args(["-map", "0", "-c", "copy", "-movflags", "+faststart"])
        .arg(out)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("Could not start FFmpeg: {e}"))?;
    let mut stdin = child.stdin.take().ok_or("FFmpeg has no stdin")?;
    // Write on another thread so a full stderr pipe cannot block both sides.
    let writer = std::thread::spawn(move || feed(&mut stdin));
    let res = child.wait_with_output().map_err(|e| e.to_string())?;
    let fed = writer.join().unwrap_or_else(|_| Err(std::io::Error::other("writer panicked")));
    if res.status.success() {
        fed.map_err(|e| format!("Could not write the clip: {e}"))
    } else {
        let err = String::from_utf8_lossy(&res.stderr);
        let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("unknown error");
        Err(format!("FFmpeg failed: {last}"))
    }
}

pub fn thumbnail(video: &Path, out: &Path, at: f64) -> Result<(), String> {
    run(command()
        .args(["-loglevel", "error", "-y", "-ss", &format!("{at:.2}"), "-i"])
        .arg(video)
        .args(["-frames:v", "1", "-vf", "scale=480:-2", "-q:v", "4"])
        .arg(out))
}

/// One image with `tiles` evenly spaced frames side by side, for the trim timeline.
pub fn filmstrip(video: &Path, out: &Path, duration: f64, tiles: u32) -> Result<(), String> {
    // One spare frame so the last tile always fills.
    let rate = (tiles + 1) as f64 / duration.max(0.5);
    run(command()
        .args(["-loglevel", "error", "-y", "-i"])
        .arg(video)
        .args([
            "-vf",
            &format!("fps={rate:.5},scale=192:-2,tile={tiles}x1"),
            "-frames:v",
            "1",
            "-q:v",
            "5",
        ])
        .arg(out))
}

pub fn probe_duration(video: &Path) -> Option<f64> {
    // `ffmpeg -i` prints "Duration: 00:00:31.53" to stderr.
    let out = command().arg("-i").arg(video).stderr(Stdio::piped()).stdout(Stdio::null()).output().ok()?;
    let text = String::from_utf8_lossy(&out.stderr);
    let d = text.split("Duration: ").nth(1)?.split(',').next()?.trim();
    let mut secs = 0.0;
    for part in d.split(':') {
        secs = secs * 60.0 + part.parse::<f64>().ok()?;
    }
    Some(secs)
}

pub struct ExportJob<'a> {
    pub input: &'a Path,
    pub output: &'a Path,
    pub start: f64,
    pub end: f64,
    pub encoder: &'a str,
    pub video_kbps: u32,
    pub audio_kbps: u32,
    pub height: Option<u32>,
    pub fps: u32,
}

/// Re-encodes a section of a clip. `progress` gets values in 0..=1.
pub fn export(job: &ExportJob, mut progress: impl FnMut(f64)) -> Result<(), String> {
    let dur = (job.end - job.start).max(0.1);
    let mut cmd = command();
    cmd.args(["-loglevel", "error", "-y", "-ss", &format!("{:.3}", job.start), "-i"])
        .arg(job.input)
        .args(["-t", &format!("{dur:.3}")]);
    let mut vf = Vec::new();
    if let Some(h) = job.height {
        vf.push(format!("scale=-2:'min({h},ih)'"));
    }
    vf.push("format=yuv420p".into());
    cmd.args(["-vf", &vf.join(",")]);
    cmd.args(encoder_args(job.encoder, job.video_kbps, job.fps, false));
    cmd.args(["-c:a", "aac", "-b:a", &format!("{}k", job.audio_kbps)]);
    cmd.args(["-movflags", "+faststart", "-progress", "pipe:1", "-nostats"]);
    cmd.arg(job.output);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());

    let mut child = cmd.spawn().map_err(|e| format!("Could not start FFmpeg: {e}"))?;
    let stderr = child.stderr.take();
    let err_thread = std::thread::spawn(move || {
        let mut s = String::new();
        if let Some(mut e) = stderr {
            use std::io::Read;
            let _ = e.read_to_string(&mut s);
        }
        s
    });
    if let Some(out) = child.stdout.take() {
        for line in BufReader::new(out).lines().map_while(Result::ok) {
            if let Some(v) = line.strip_prefix("out_time_us=") {
                if let Ok(us) = v.trim().parse::<f64>() {
                    progress((us / 1e6 / dur).clamp(0.0, 1.0));
                }
            }
        }
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let err = err_thread.join().unwrap_or_default();
    if status.success() {
        progress(1.0);
        Ok(())
    } else {
        let last = err.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or("unknown error");
        Err(format!("Export failed: {last}"))
    }
}
