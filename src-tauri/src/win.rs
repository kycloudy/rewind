//! Small Win32 helpers. Each function has a no-op fallback off Windows so the
//! crate still builds (and its shared logic can be checked) on other systems.

use serde::Serialize;
use std::path::Path;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Monitor {
    pub index: u32,
    pub name: String,
    pub width: i32,
    pub height: i32,
    pub primary: bool,
}

/// The app the player is looking at: (exe stem, window title).
pub fn foreground_app() -> Option<(String, String)> {
    imp::foreground_app()
}

/// True when the foreground window is borderless and covers its whole monitor,
/// as full-screen and borderless-windowed games do.
pub fn foreground_full_screen() -> bool {
    imp::foreground_full_screen()
}

/// Apps that often run full screen but are not games.
const NOT_GAMES: &[&str] = &[
    "applicationframehost", "arc", "brave", "chrome", "explorer", "firefox", "lockapp", "mpc-be64",
    "mpc-hc", "mpc-hc64", "mpv", "msedge", "opera", "potplayer", "potplayermini64", "powerpnt",
    "searchhost", "shellexperiencehost", "startmenuexperiencehost", "vivaldi", "vlc", "wmplayer",
];

/// Guesses whether the foreground app is a game, from its exe stem.
pub fn looks_like_game(exe: &str, full_screen: bool) -> bool {
    full_screen && !exe.is_empty() && !NOT_GAMES.contains(&exe.to_lowercase().as_str())
}

pub fn monitors() -> Vec<Monitor> {
    imp::monitors()
}

pub fn copy_file_to_clipboard(path: &Path) -> Result<(), String> {
    imp::copy_file_to_clipboard(path)
}

/// Plays a short chime without blocking. `rising` = true for "saved", false for "stopped".
pub fn chime(rising: bool) {
    imp::chime(rising)
}

pub fn reveal(path: &Path) {
    imp::reveal(path)
}

/// Builds a game name from the foreground window, e.g. "Minecraft* 1.21.1 - Multiplayer" -> "Minecraft".
pub fn game_name(exe: &str, title: &str) -> String {
    let head = title.split(" - ").next().unwrap_or("").split(" | ").next().unwrap_or("");
    let words: Vec<&str> = head
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| c == '*' || c == '™' || c == '®'))
        .filter(|w| !w.is_empty())
        .take_while(|w| !is_version(w))
        .collect();
    let name = words.join(" ");
    if !name.is_empty() && name.chars().count() <= 48 {
        return name;
    }
    // Fall back to a tidied exe name: "VALORANT-Win64-Shipping" -> "VALORANT".
    let stem = exe
        .trim_end_matches("-Win64-Shipping")
        .trim_end_matches("_x64")
        .trim_end_matches("64")
        .replace(['_', '-'], " ");
    let stem = stem.trim();
    if stem.is_empty() { "Desktop".into() } else { stem.to_string() }
}

/// "1.21.1", "v2.3" and similar.
fn is_version(w: &str) -> bool {
    let digits = w.strip_prefix(['v', 'V']).unwrap_or(w);
    digits.starts_with(|c: char| c.is_ascii_digit()) && (digits.contains('.') || digits.len() < w.len())
}

/// 16-bit mono WAV with a soft two-note chime.
#[cfg_attr(not(any(windows, wincheck)), allow(dead_code))]
pub fn chime_wav(rising: bool) -> Vec<u8> {
    let rate = 44_100u32;
    let notes: [(f32, f32); 2] = if rising { [(784.0, 0.07), (1175.0, 0.11)] } else { [(988.0, 0.07), (659.0, 0.11)] };
    let mut samples: Vec<i16> = Vec::new();
    for (freq, secs) in notes {
        let n = (rate as f32 * secs) as usize;
        for i in 0..n {
            let t = i as f32 / rate as f32;
            let attack = (t / 0.004).min(1.0);
            let release = (1.0 - i as f32 / n as f32).powf(1.6);
            let v = (t * freq * std::f32::consts::TAU).sin() * attack * release * 0.32;
            samples.push((v * i16::MAX as f32) as i16);
        }
    }
    let data_len = (samples.len() * 2) as u32;
    let mut w = Vec::with_capacity(44 + data_len as usize);
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data_len).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes()); // PCM
    w.extend_from_slice(&1u16.to_le_bytes()); // mono
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 2).to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        w.extend_from_slice(&s.to_le_bytes());
    }
    w
}

#[cfg(any(windows, wincheck))]
mod imp {
    use super::Monitor;
    use std::path::Path;
    use std::sync::OnceLock;
    use windows_sys::Win32::Foundation::{CloseHandle, GlobalFree, HWND, LPARAM, RECT};
    use windows_sys::Win32::Graphics::Gdi::{
        EnumDisplayMonitors, GetMonitorInfoW, MonitorFromWindow, HDC, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONULL,
    };
    use windows_sys::Win32::Media::Audio::{PlaySoundW, SND_ASYNC, SND_MEMORY, SND_NODEFAULT};
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GHND};
    use windows_sys::Win32::System::Threading::{
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::Shell::DROPFILES;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetForegroundWindow, GetWindowLongW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId,
        GWL_STYLE, WS_CAPTION,
    };

    const CF_HDROP: u32 = 15;
    const MONITORINFOF_PRIMARY: u32 = 1;

    pub fn foreground_app() -> Option<(String, String)> {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_null() {
                return None;
            }
            let mut title = [0u16; 256];
            let n = GetWindowTextW(hwnd, title.as_mut_ptr(), title.len() as i32);
            let title = String::from_utf16_lossy(&title[..n.max(0) as usize]);

            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            let proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            let mut exe = String::new();
            if !proc.is_null() {
                let mut buf = [0u16; 520];
                let mut len = buf.len() as u32;
                if QueryFullProcessImageNameW(proc, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) != 0 {
                    let full = String::from_utf16_lossy(&buf[..len as usize]);
                    exe = Path::new(&full)
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                }
                CloseHandle(proc);
            }
            Some((exe, title))
        }
    }

    pub fn foreground_full_screen() -> bool {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd.is_null() {
                return false;
            }
            // The desktop itself fills the screen too.
            let mut class = [0u16; 32];
            let n = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
            let class = String::from_utf16_lossy(&class[..n.max(0) as usize]);
            if class == "Progman" || class == "WorkerW" {
                return false;
            }
            // A maximized app window has a title bar; a full-screen game has none.
            if GetWindowLongW(hwnd, GWL_STYLE) as u32 & WS_CAPTION == WS_CAPTION {
                return false;
            }
            let hmon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONULL);
            if hmon.is_null() {
                return false;
            }
            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            let mut r: RECT = std::mem::zeroed();
            if GetMonitorInfoW(hmon, &mut info) == 0 || GetWindowRect(hwnd, &mut r) == 0 {
                return false;
            }
            let m = info.rcMonitor;
            r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom
        }
    }

    unsafe extern "system" fn monitor_cb(hmon: HMONITOR, _hdc: HDC, _rect: *mut RECT, data: LPARAM) -> i32 {
        let list = &mut *(data as *mut Vec<(bool, i32, i32)>);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(hmon, &mut info) != 0 {
            let r = info.rcMonitor;
            list.push((info.dwFlags & MONITORINFOF_PRIMARY != 0, r.right - r.left, r.bottom - r.top));
        }
        1
    }

    pub fn monitors() -> Vec<Monitor> {
        let mut raw: Vec<(bool, i32, i32)> = Vec::new();
        unsafe {
            EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), Some(monitor_cb), &mut raw as *mut _ as LPARAM);
        }
        // Desktop Duplication lists the primary output first on most systems.
        raw.sort_by_key(|m| !m.0);
        raw.into_iter()
            .enumerate()
            .map(|(i, (primary, w, h))| Monitor {
                index: i as u32,
                name: if primary { "Main display".into() } else { format!("Display {}", i + 1) },
                width: w,
                height: h,
                primary,
            })
            .collect()
    }

    pub fn copy_file_to_clipboard(path: &Path) -> Result<(), String> {
        let wide: Vec<u16> = path.to_string_lossy().encode_utf16().chain([0, 0]).collect();
        let header = std::mem::size_of::<DROPFILES>();
        let total = header + wide.len() * 2;
        unsafe {
            let mem = GlobalAlloc(GHND, total);
            if mem.is_null() {
                return Err("Out of memory".into());
            }
            let ptr = GlobalLock(mem) as *mut u8;
            if ptr.is_null() {
                GlobalFree(mem);
                return Err("Could not lock clipboard memory".into());
            }
            let df = ptr as *mut DROPFILES;
            (*df).pFiles = header as u32;
            (*df).fWide = 1;
            std::ptr::copy_nonoverlapping(wide.as_ptr() as *const u8, ptr.add(header), wide.len() * 2);
            GlobalUnlock(mem);

            if OpenClipboard(std::ptr::null_mut::<core::ffi::c_void>() as HWND) == 0 {
                GlobalFree(mem);
                return Err("The clipboard is busy. Try again.".into());
            }
            EmptyClipboard();
            let ok = !SetClipboardData(CF_HDROP, mem).is_null();
            CloseClipboard();
            if !ok {
                GlobalFree(mem);
                return Err("Could not copy the clip".into());
            }
        }
        Ok(())
    }

    pub fn chime(rising: bool) {
        static UP: OnceLock<Vec<u8>> = OnceLock::new();
        static DOWN: OnceLock<Vec<u8>> = OnceLock::new();
        let wav = if rising { UP.get_or_init(|| super::chime_wav(true)) } else { DOWN.get_or_init(|| super::chime_wav(false)) };
        unsafe {
            PlaySoundW(wav.as_ptr() as *const u16, std::ptr::null_mut(), SND_MEMORY | SND_ASYNC | SND_NODEFAULT);
        }
    }

    #[cfg(windows)]
    pub fn reveal(path: &Path) {
        use std::os::windows::process::CommandExt;
        let _ = std::process::Command::new("explorer")
            .raw_arg(format!("/select,\"{}\"", path.display()))
            .spawn();
    }

    #[cfg(not(windows))]
    pub fn reveal(_path: &Path) {}
}

#[cfg(not(any(windows, wincheck)))]
mod imp {
    use super::Monitor;
    use std::path::Path;

    pub fn foreground_app() -> Option<(String, String)> {
        None
    }
    pub fn foreground_full_screen() -> bool {
        false
    }
    pub fn monitors() -> Vec<Monitor> {
        vec![Monitor { index: 0, name: "Main display".into(), width: 1920, height: 1080, primary: true }]
    }
    pub fn copy_file_to_clipboard(_path: &Path) -> Result<(), String> {
        Err("Copying files is only available on Windows".into())
    }
    pub fn chime(_rising: bool) {}
    pub fn reveal(path: &Path) {
        if let Some(dir) = path.parent() {
            let _ = std::process::Command::new("xdg-open").arg(dir).spawn();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{game_name, looks_like_game};

    #[test]
    fn games() {
        assert!(looks_like_game("cs2", true));
        assert!(!looks_like_game("cs2", false));
        assert!(!looks_like_game("Chrome", true));
        assert!(!looks_like_game("", true));
    }

    #[test]
    fn names() {
        assert_eq!(game_name("javaw", "Minecraft* 1.21.1 - Multiplayer (3rd-party Server)"), "Minecraft");
        assert_eq!(game_name("VALORANT-Win64-Shipping", "VALORANT  "), "VALORANT");
        assert_eq!(game_name("cs2", "Counter-Strike 2"), "Counter-Strike 2");
        assert_eq!(game_name("x", "Terraria v1.4.4"), "Terraria");
        assert_eq!(game_name("RocketLeague", ""), "RocketLeague");
        assert_eq!(game_name("VALORANT-Win64-Shipping", ""), "VALORANT");
        assert_eq!(game_name("", ""), "Desktop");
    }
}
