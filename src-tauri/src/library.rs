use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Clip {
    pub id: String,
    pub path: PathBuf,
    pub thumb: Option<PathBuf>,
    pub title: String,
    pub game: String,
    /// Unix time in milliseconds.
    pub created: i64,
    /// Seconds.
    pub duration: f64,
    pub size: u64,
    pub favorite: bool,
    /// "clip" | "recording" | "export"
    pub kind: String,
    /// The part of the file to play and share. The file itself keeps every frame.
    #[serde(default)]
    pub trim: Option<Trim>,
}

/// Seconds from the start of the file.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Trim {
    pub start: f64,
    pub end: f64,
}

pub struct Library {
    file: PathBuf,
    clips: Mutex<Vec<Clip>>,
}

impl Library {
    pub fn open(file: PathBuf) -> Self {
        let clips: Vec<Clip> = std::fs::read_to_string(&file)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        Self { file, clips: Mutex::new(clips) }
    }

    fn persist(&self, clips: &[Clip]) {
        if let Some(dir) = self.file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let tmp = self.file.with_extension("json.tmp");
        if std::fs::write(&tmp, serde_json::to_string(clips).unwrap_or_default()).is_ok() {
            let _ = std::fs::rename(&tmp, &self.file);
        }
    }

    /// Returns all clips, newest first. Drops entries whose file was deleted outside the app.
    pub fn list(&self) -> Vec<Clip> {
        let mut clips = self.clips.lock();
        let before = clips.len();
        clips.retain(|c| c.path.exists());
        if clips.len() != before {
            self.persist(&clips);
        }
        let mut out = clips.clone();
        out.sort_by_key(|c| std::cmp::Reverse(c.created));
        out
    }

    pub fn get(&self, id: &str) -> Option<Clip> {
        self.clips.lock().iter().find(|c| c.id == id).cloned()
    }

    pub fn add(&self, clip: Clip) {
        let mut clips = self.clips.lock();
        clips.push(clip);
        self.persist(&clips);
    }

    pub fn update<F: FnOnce(&mut Clip)>(&self, id: &str, f: F) -> Option<Clip> {
        let mut clips = self.clips.lock();
        let clip = clips.iter_mut().find(|c| c.id == id)?;
        f(clip);
        let out = clip.clone();
        self.persist(&clips);
        Some(out)
    }

    pub fn remove(&self, id: &str) -> Option<Clip> {
        let mut clips = self.clips.lock();
        let idx = clips.iter().position(|c| c.id == id)?;
        let clip = clips.remove(idx);
        self.persist(&clips);
        Some(clip)
    }
}

pub fn new_id() -> String {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    format!(
        "{:x}{:04x}",
        chrono::Utc::now().timestamp_millis(),
        N.fetch_add(1, Ordering::Relaxed) & 0xffff
    )
}

/// Turns a game name into something safe for a Windows file name.
pub fn safe_file_stem(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| if "<>:\"/\\|?*".contains(c) || c.is_control() { ' ' } else { c })
        .collect();
    let trimmed = cleaned.split_whitespace().collect::<Vec<_>>().join(" ");
    let short: String = trimmed.chars().take(60).collect();
    if short.is_empty() { "Clip".into() } else { short }
}

/// Picks `dir/stem.ext`, adding " (2)", " (3)"... when the name is taken.
pub fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let first = dir.join(format!("{stem}.{ext}"));
    if !first.exists() {
        return first;
    }
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}).{ext}")))
        .find(|p| !p.exists())
        .unwrap()
}
