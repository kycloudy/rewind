use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Settings {
    /// Replay buffer master switch.
    pub buffer_enabled: bool,
    /// Seconds saved by the clip hotkey.
    pub clip_seconds: u32,
    pub fps: u32,
    /// "low" | "medium" | "high" | "ultra"
    pub quality: String,
    /// "native" | "1440" | "1080" | "720"
    pub resolution: String,
    /// "auto" or an FFmpeg encoder name such as "h264_nvenc".
    pub encoder: String,
    pub monitor: u32,
    pub system_audio: bool,
    pub mic: bool,
    /// None = Windows default input device.
    pub mic_device: Option<String>,
    pub mic_volume: f32,
    /// Mute the mic while it is quieter than `mic_gate_threshold`.
    pub mic_gate: bool,
    /// Noise gate threshold in dBFS.
    pub mic_gate_threshold: f32,
    pub clip_hotkey: String,
    pub record_hotkey: String,
    pub clips_dir: PathBuf,
    pub launch_at_login: bool,
    pub clip_sound: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            buffer_enabled: true,
            clip_seconds: 30,
            fps: 60,
            quality: "high".into(),
            resolution: "native".into(),
            encoder: "auto".into(),
            monitor: 0,
            system_audio: true,
            mic: false,
            mic_device: None,
            mic_volume: 1.0,
            mic_gate: false,
            mic_gate_threshold: -45.0,
            clip_hotkey: "F8".into(),
            record_hotkey: "F9".into(),
            clips_dir: default_clips_dir(),
            launch_at_login: false,
            clip_sound: true,
        }
    }
}

impl Settings {
    /// Video bitrate in kbit/s.
    pub fn video_kbps(&self) -> u32 {
        let base = match self.quality.as_str() {
            "low" => 8_000,
            "medium" => 15_000,
            "ultra" => 45_000,
            _ => 25_000,
        };
        let res_scale = match self.resolution.as_str() {
            "720" => 0.5,
            "1080" => 0.8,
            _ => 1.0,
        };
        let fps_scale = if self.fps <= 30 { 0.7 } else { 1.0 };
        (base as f32 * res_scale * fps_scale) as u32
    }

    pub fn target_height(&self) -> Option<u32> {
        self.resolution.parse().ok()
    }

    pub fn any_audio(&self) -> bool {
        self.system_audio || self.mic
    }

    /// True when a change between `self` and `other` needs the capture pipeline restarted.
    pub fn capture_changed(&self, other: &Settings) -> bool {
        self.buffer_enabled != other.buffer_enabled
            || self.clip_seconds != other.clip_seconds
            || self.fps != other.fps
            || self.quality != other.quality
            || self.resolution != other.resolution
            || self.encoder != other.encoder
            || self.monitor != other.monitor
            || self.system_audio != other.system_audio
            || self.mic != other.mic
            || self.mic_device != other.mic_device
            || (self.mic_volume - other.mic_volume).abs() > f32::EPSILON
            || self.mic_gate != other.mic_gate
            || (self.mic_gate_threshold - other.mic_gate_threshold).abs() > f32::EPSILON
    }

    pub fn load(path: &Path) -> Self {
        let mut s: Settings = std::fs::read_to_string(path)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        s.clip_seconds = s.clip_seconds.clamp(5, 300);
        s.fps = if s.fps >= 45 { 60 } else { 30 };
        s.mic_volume = s.mic_volume.clamp(0.0, 2.0);
        s.mic_gate_threshold = s.mic_gate_threshold.clamp(-80.0, -10.0);
        if s.clips_dir.as_os_str().is_empty() {
            s.clips_dir = default_clips_dir();
        }
        s
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self).unwrap_or_default())
    }
}

fn default_clips_dir() -> PathBuf {
    let base = std::env::var_os("USERPROFILE")
        .or_else(|| std::env::var_os("HOME"))
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    base.join("Videos").join("Rewind")
}
