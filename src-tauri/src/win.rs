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

/// The window the player is looking at.
#[derive(Debug, Clone, Default)]
pub struct Foreground {
    /// Exe file name without ".exe", e.g. "cs2".
    pub exe: String,
    pub title: String,
    /// Full path of the exe.
    pub path: String,
    /// Index of the window's monitor, in the order `monitors` lists them.
    pub monitor: Option<u32>,
    /// True when the window is borderless and covers its whole monitor,
    /// as full-screen and borderless-windowed games do.
    pub full_screen: bool,
}

pub fn foreground() -> Option<Foreground> {
    imp::foreground()
}

/// Apps that are not games, even when they run full screen or from a game folder.
/// Lower-case exe names without ".exe".
const NOT_GAMES: &[&str] = &[
    // Windows
    "applicationframehost", "cmd", "explorer", "lockapp", "screenclippinghost", "searchhost", "searchui",
    "shellexperiencehost", "snippingtool", "startmenuexperiencehost", "systemsettings", "taskmgr",
    "textinputhost", "windowsinternal.composableshell.experiences.textinput.inputapp", "windowsterminal",
    "winstore.app", "wwahost",
    // Browsers
    "arc", "brave", "chrome", "firefox", "msedge", "opera", "opera_gx", "vivaldi",
    // Video, music, and photos
    "kodi", "microsoft.media.player", "mpc-be64", "mpc-hc", "mpc-hc64", "mpv", "photos", "plex", "potplayer",
    "potplayermini64", "spotify", "video.ui", "vlc", "wmplayer",
    // Chat, calls, and office
    "discord", "excel", "ms-teams", "olk", "onenote", "outlook", "powerpnt", "skype", "slack", "teams",
    "winword", "zoom",
    // Remote desktops and virtual machines
    "moonlight", "msrdc", "mstsc", "parsecd", "virtualboxvm", "vmconnect", "vmware", "vmware-vmx",
    // Game launchers and tools
    "battle.net", "eadesktop", "epicgameslauncher", "galaxyclient", "losslessscaling", "medal", "nvidia overlay",
    "obs64", "origin", "riotclientservices", "riotclientux", "soundpad", "steam", "steamwebhelper", "upc",
    "uplay", "wallpaper32", "wallpaper64",
    // Code editors
    "code", "cursor", "devenv",
];

/// Folders where game launchers install games, lower case with `\\` separators.
const GAME_DIRS: &[&str] = &[
    "\\steamapps\\common\\",
    "\\epic games\\",
    "\\gog galaxy\\games\\",
    "\\gog games\\",
    "\\riot games\\",
    "\\xboxgames\\",
    "\\ubisoft game launcher\\games\\",
    "\\ea games\\",
    "\\origin games\\",
    "\\battle.net\\games\\",
];

/// Games that often run in a normal window.
const KNOWN_GAMES: &[&str] = &[
    "cs2", "dota2", "eldenring", "fortniteclient-win64-shipping", "gta5", "league of legends", "minecraft",
    "overwatch", "r5apex", "rocketleague", "terraria", "valorant-win64-shipping", "wow",
];

/// Tools that install under game folders but are not games.
const GAME_FOLDER_TOOLS: &[&str] = &[
    "crashreporter", "crashhandler", "epicgameslauncher", "steam", "steamwebhelper", "unins000", "vc_redist.x64",
];

/// Guesses whether the foreground app is a game.
///
/// An app on the user's list always counts. Other apps count when they are a
/// known game, run from a game launcher's install folder, or run full screen,
/// unless they are a known non-game.
pub fn looks_like_game(fg: &Foreground, user_games: &[String]) -> bool {
    let exe = fg.exe.to_lowercase();
    let exe = exe.trim_end_matches(".exe");
    if exe.is_empty() {
        return false;
    }
    if user_games.iter().any(|g| g.trim().trim_end_matches(".exe").eq_ignore_ascii_case(exe)) {
        return true;
    }
    if NOT_GAMES.contains(&exe) {
        return false;
    }
    // Minecraft Java Edition runs as javaw, like every other Java app.
    let minecraft = matches!(exe, "java" | "javaw") && fg.title.starts_with("Minecraft");
    if fg.full_screen || minecraft || KNOWN_GAMES.contains(&exe) {
        return true;
    }
    let path = fg.path.to_lowercase().replace('/', "\\");
    !GAME_FOLDER_TOOLS.contains(&exe) && GAME_DIRS.iter().any(|d| path.contains(d))
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
        EnumDisplayMonitors, GetMonitorInfoW, MonitorFromWindow, HDC, HMONITOR, MONITORINFO, MONITOR_DEFAULTTONEAREST,
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

    pub fn foreground() -> Option<super::Foreground> {
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
            let mut path = String::new();
            if !proc.is_null() {
                let mut buf = [0u16; 520];
                let mut len = buf.len() as u32;
                if QueryFullProcessImageNameW(proc, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) != 0 {
                    path = String::from_utf16_lossy(&buf[..len as usize]);
                    exe = Path::new(&path)
                        .file_stem()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_default();
                }
                CloseHandle(proc);
            }

            let mut info: MONITORINFO = std::mem::zeroed();
            info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
            let hmon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            let mut r: RECT = std::mem::zeroed();
            let (monitor, full_screen) = if !hmon.is_null()
                && GetMonitorInfoW(hmon, &mut info) != 0
                && GetWindowRect(hwnd, &mut r) != 0
            {
                let m = info.rcMonitor;
                let monitor = monitor_list().iter().position(|(_, x)| *x == rect(&m)).map(|i| i as u32);
                let covers = r.left <= m.left && r.top <= m.top && r.right >= m.right && r.bottom >= m.bottom;
                (monitor, covers && !is_desktop(hwnd) && GetWindowLongW(hwnd, GWL_STYLE) as u32 & WS_CAPTION != WS_CAPTION)
            } else {
                (None, false)
            };
            Some(super::Foreground { exe, title, path, monitor, full_screen })
        }
    }

    /// The desktop fills the screen too.
    unsafe fn is_desktop(hwnd: HWND) -> bool {
        let mut class = [0u16; 32];
        let n = GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32);
        let class = String::from_utf16_lossy(&class[..n.max(0) as usize]);
        class == "Progman" || class == "WorkerW"
    }

    fn rect(r: &RECT) -> (i32, i32, i32, i32) {
        (r.left, r.top, r.right, r.bottom)
    }

    unsafe extern "system" fn monitor_cb(hmon: HMONITOR, _hdc: HDC, _rect: *mut RECT, data: LPARAM) -> i32 {
        let list = &mut *(data as *mut Vec<(bool, (i32, i32, i32, i32))>);
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(hmon, &mut info) != 0 {
            list.push((info.dwFlags & MONITORINFOF_PRIMARY != 0, rect(&info.rcMonitor)));
        }
        1
    }

    /// Monitors as (primary, rect), primary first.
    fn monitor_list() -> Vec<(bool, (i32, i32, i32, i32))> {
        let mut raw: Vec<(bool, (i32, i32, i32, i32))> = Vec::new();
        unsafe {
            EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), Some(monitor_cb), &mut raw as *mut _ as LPARAM);
        }
        // Desktop Duplication lists the primary output first on most systems.
        raw.sort_by_key(|m| !m.0);
        raw
    }

    pub fn monitors() -> Vec<Monitor> {
        monitor_list()
            .into_iter()
            .enumerate()
            .map(|(i, (primary, (l, t, r, b)))| Monitor {
                index: i as u32,
                name: if primary { "Main display".into() } else { format!("Display {}", i + 1) },
                width: r - l,
                height: b - t,
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

    pub fn foreground() -> Option<super::Foreground> {
        None
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
    use super::{game_name, looks_like_game, Foreground};

    fn app(exe: &str, path: &str, full_screen: bool) -> Foreground {
        Foreground { exe: exe.into(), path: path.into(), full_screen, ..Default::default() }
    }

    #[test]
    fn games() {
        assert!(looks_like_game(&app("cs2", "", true), &[]));
        assert!(!looks_like_game(&app("cs2x", "", false), &[]));
        assert!(!looks_like_game(&app("Chrome", "", true), &[]));
        assert!(!looks_like_game(&app("", "", true), &[]));
    }

    #[test]
    fn windowed_games() {
        assert!(looks_like_game(&app("cs2", "", false), &[]));
        let steam = r"C:\Program Files (x86)\Steam\steamapps\common\Hades\Hades.exe";
        assert!(looks_like_game(&app("Hades", steam, false), &[]));
        assert!(!looks_like_game(&app("steamwebhelper", steam, false), &[]));
        assert!(!looks_like_game(&app("chrome", steam, false), &[]));
        assert!(looks_like_game(&app("Stardew Valley", "", false), &["stardew valley.exe".into()]));
        assert!(looks_like_game(&app("mygame", "", false), &["MyGame".into()]));
    }

    #[test]
    fn full_screen_apps_that_are_not_games() {
        for exe in ["Microsoft.Media.Player", "mstsc", "Discord", "WINWORD", "ScreenClippingHost", "Spotify"] {
            assert!(!looks_like_game(&app(exe, "", true), &[]), "{exe}");
        }
        // The user's list wins.
        assert!(looks_like_game(&app("vlc", "", false), &["vlc".into()]));
    }

    #[test]
    fn minecraft_but_not_every_java_app() {
        let mut mc = app("javaw", "", false);
        mc.title = "Minecraft* 1.21.1 - Multiplayer (3rd-party Server)".into();
        assert!(looks_like_game(&mc, &[]));
        mc.title = "JDownloader 2".into();
        assert!(!looks_like_game(&mc, &[]));
    }

    #[test]
    fn desktop_apps_are_not_games() {
        let code = r"C:\Users\a\AppData\Local\Programs\Microsoft VS Code\Code.exe";
        assert!(!looks_like_game(&app("Code", code, false), &[]));
        assert!(!looks_like_game(&app("notepad", r"C:\Windows\notepad.exe", false), &[]));
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
