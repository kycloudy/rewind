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
    /// Full path of the exe. Empty when Windows will not give it (for example anti-cheat games).
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

/// A registry root.
#[derive(Clone, Copy)]
pub enum Hive {
    User,
    Machine,
}

/// A string value from the registry.
pub fn reg_string(hive: Hive, key: &str, value: &str) -> Option<String> {
    imp::reg_string(hive, key, value)
}

/// The names of a registry key's subkeys.
pub fn reg_subkeys(hive: Hive, key: &str) -> Vec<String> {
    imp::reg_subkeys(hive, key)
}

/// Root folders of the local fixed drives, e.g. `C:\`.
pub fn fixed_drives() -> Vec<String> {
    imp::fixed_drives()
}

/// Full exe paths of the running processes the user may read. Opens every process, so call it every few seconds at most.
pub fn processes() -> Vec<String> {
    imp::processes()
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
    use windows_sys::Win32::Storage::FileSystem::{GetDriveTypeW, GetLogicalDrives};
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegEnumKeyExW, RegGetValueW, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ,
        RRF_RT_REG_SZ,
    };
    use windows_sys::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData};
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
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
    const DRIVE_FIXED: u32 = 3;
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
            // Anti-cheat can block OpenProcess. A process snapshot still gives the exe name.
            if exe.is_empty() {
                exe = exe_name_from_snapshot(pid).unwrap_or_default();
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

    pub fn processes() -> Vec<String> {
        let mut paths = Vec::new();
        unsafe {
            let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snap.is_null() || snap == windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE {
                return paths;
            }
            let mut entry: PROCESSENTRY32W = std::mem::zeroed();
            entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
            let mut ok = Process32FirstW(snap, &mut entry) != 0;
            while ok {
                let proc = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, entry.th32ProcessID);
                if !proc.is_null() {
                    let mut buf = [0u16; 520];
                    let mut len = buf.len() as u32;
                    if QueryFullProcessImageNameW(proc, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len) != 0 {
                        paths.push(String::from_utf16_lossy(&buf[..len as usize]));
                    }
                    CloseHandle(proc);
                }
                ok = Process32NextW(snap, &mut entry) != 0;
            }
            CloseHandle(snap);
        }
        paths
    }

    /// The exe name (no ".exe") of process `pid`, read from a process snapshot.
    /// This works when the process cannot be opened. The path is not available.
    unsafe fn exe_name_from_snapshot(pid: u32) -> Option<String> {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap.is_null() || snap as isize == -1 {
            return None;
        }
        let mut entry: PROCESSENTRY32W = std::mem::zeroed();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut name = None;
        let mut ok = Process32FirstW(snap, &mut entry) != 0;
        while ok {
            if entry.th32ProcessID == pid {
                let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
                let file = String::from_utf16_lossy(&entry.szExeFile[..len]);
                name = Path::new(&file).file_stem().map(|s| s.to_string_lossy().into_owned());
                break;
            }
            ok = Process32NextW(snap, &mut entry) != 0;
        }
        CloseHandle(snap);
        name
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

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain([0]).collect()
    }

    fn hkey(hive: super::Hive) -> HKEY {
        match hive {
            super::Hive::User => HKEY_CURRENT_USER,
            super::Hive::Machine => HKEY_LOCAL_MACHINE,
        }
    }

    pub fn reg_string(hive: super::Hive, key: &str, value: &str) -> Option<String> {
        let (key, value) = (wide(key), wide(value));
        let mut buf = [0u16; 1024];
        let mut len = (buf.len() * 2) as u32;
        let res = unsafe {
            RegGetValueW(
                hkey(hive),
                key.as_ptr(),
                value.as_ptr(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buf.as_mut_ptr().cast(),
                &mut len,
            )
        };
        if res != 0 {
            return None;
        }
        let s = String::from_utf16_lossy(&buf[..(len as usize / 2).min(buf.len())]);
        Some(s.trim_end_matches('\0').to_string())
    }

    pub fn reg_subkeys(hive: super::Hive, key: &str) -> Vec<String> {
        let key = wide(key);
        let mut names = Vec::new();
        unsafe {
            let mut hk: HKEY = std::ptr::null_mut();
            if RegOpenKeyExW(hkey(hive), key.as_ptr(), 0, KEY_READ, &mut hk) != 0 {
                return names;
            }
            for i in 0.. {
                let mut name = [0u16; 256];
                let mut len = name.len() as u32;
                let res = RegEnumKeyExW(
                    hk,
                    i,
                    name.as_mut_ptr(),
                    &mut len,
                    std::ptr::null(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                );
                if res != 0 {
                    break;
                }
                names.push(String::from_utf16_lossy(&name[..len as usize]));
            }
            RegCloseKey(hk);
        }
        names
    }

    pub fn fixed_drives() -> Vec<String> {
        let mask = unsafe { GetLogicalDrives() };
        (0..26u8)
            .filter(|i| mask & (1 << i) != 0)
            .map(|i| format!("{}:\\", (b'A' + i) as char))
            .filter(|root| unsafe { GetDriveTypeW(wide(root).as_ptr()) } == DRIVE_FIXED)
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
    pub fn reg_string(_hive: super::Hive, _key: &str, _value: &str) -> Option<String> {
        None
    }
    pub fn reg_subkeys(_hive: super::Hive, _key: &str) -> Vec<String> {
        Vec::new()
    }
    pub fn fixed_drives() -> Vec<String> {
        Vec::new()
    }
    pub fn processes() -> Vec<String> {
        Vec::new()
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
