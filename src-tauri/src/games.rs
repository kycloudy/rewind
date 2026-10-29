//! Decides whether the app in front is a game.
//!
//! Game launchers know which games are installed and where. Rewind reads their
//! manifests and registry keys, so a game counts wherever it is installed and
//! its clips get the store name. Other apps count when they are on the user's
//! list or a known list, or when they run full screen.

use crate::win::{self, Foreground, Hive};
use std::fs;
use std::path::{Path, PathBuf};

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
    // Tools that install next to games
    "crashhandler", "crashreportclient", "crashreporter", "dxsetup", "unins000", "unitycrashhandler64",
    "vc_redist.x64", "vc_redist.x86",
    // Code editors
    "code", "cursor", "devenv",
];

/// Folders where game launchers install games, lower case with `\` separators.
/// They catch games that `scan` misses, such as Riot, EA and Battle.net games.
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

/// Steam apps that are tools, not games.
const STEAM_TOOLS: &[u32] = &[
    228980, // Steamworks Common Redistributables
    250820, // SteamVR
    365670, // Blender
    388080, // Borderless Gaming
    431730, // Aseprite
    431960, // Wallpaper Engine
    629520, // Soundpad
    993090, // Lossless Scaling
];

/// A game that a launcher installed.
#[derive(Debug, Clone, PartialEq)]
pub struct Installed {
    pub name: String,
    /// Install folder, lower case, with `\` separators and a trailing `\`.
    pub dir: String,
}

impl Installed {
    fn new(name: &str, dir: &str) -> Option<Self> {
        let (name, dir) = (name.trim(), dir.trim());
        if name.is_empty() || dir.is_empty() {
            return None;
        }
        let mut dir = dir.to_lowercase().replace('/', "\\");
        if !dir.ends_with('\\') {
            dir.push('\\');
        }
        Some(Self { name: name.into(), dir })
    }
}

/// The game in front.
#[derive(Debug, Clone, PartialEq)]
pub struct Detected {
    pub name: String,
    /// True when only the full-screen test matched, so the app may not be a game.
    pub guessed: bool,
}

/// Decides whether `fg` is a game. The user's lists decide first.
pub fn detect(fg: &Foreground, installed: &[Installed], user_games: &[String], not_games: &[String]) -> Option<Detected> {
    judge(fg, installed, user_games, not_games).0
}

/// Like `detect`, and says why. The reason is shown in Settings.
pub fn judge(
    fg: &Foreground,
    installed: &[Installed],
    user_games: &[String],
    not_games: &[String],
) -> (Option<Detected>, &'static str) {
    let exe = fg.exe.to_lowercase();
    let exe = exe.trim_end_matches(".exe");
    if exe.is_empty() {
        return (None, "Windows did not give the app's exe path");
    }
    if listed(not_games, exe) {
        return (None, "On your Not games list");
    }
    let path = fg.path.to_lowercase().replace('/', "\\");
    let store = installed.iter().filter(|g| path.starts_with(&g.dir)).max_by_key(|g| g.dir.len());
    let found = |guessed, why| {
        let name = store.map_or_else(|| game_name(&fg.exe, &fg.title), |g| g.name.clone());
        (Some(Detected { name, guessed }), why)
    };
    if listed(user_games, exe) {
        return found(false, "On your Other games list");
    }
    if NOT_GAMES.contains(&exe) {
        return (None, "Known app that is not a game");
    }
    // Minecraft Java Edition runs as javaw, like every other Java app.
    let minecraft = matches!(exe, "java" | "javaw") && fg.title.starts_with("Minecraft");
    if store.is_some() {
        return found(false, "Installed by a game launcher");
    }
    if minecraft || KNOWN_GAMES.contains(&exe) {
        return found(false, "Known game");
    }
    if GAME_DIRS.iter().any(|d| path.contains(d)) {
        return found(false, "Runs from a game folder");
    }
    if fg.full_screen {
        return found(true, "Full screen, but not a known game");
    }
    (None, "Not full screen and not a known game")
}

/// Finds a game among the running processes, for a game that is not in front.
/// Only an installed game or a listed app counts. The full-screen test cannot run here.
pub fn running(paths: &[String], installed: &[Installed], user_games: &[String], not_games: &[String]) -> Option<(Foreground, Detected)> {
    paths.iter().find_map(|path| {
        let exe = Path::new(path).file_stem()?.to_string_lossy().into_owned();
        let fg = Foreground { exe, path: path.clone(), ..Default::default() };
        let game = detect(&fg, installed, user_games, not_games)?;
        Some((fg, game))
    })
}

/// True when `exe` (lower case, no ".exe") is on a user list of app names.
pub fn listed(list: &[String], exe: &str) -> bool {
    list.iter().any(|g| g.trim().to_lowercase().trim_end_matches(".exe") == exe)
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

// ---------------------------------------------------------------- launchers

/// Finds the games that Steam, Epic, GOG, Ubisoft Connect and the Xbox app installed.
/// It reads a few small files and registry keys, so call it once a minute at most.
pub fn scan() -> Vec<Installed> {
    let mut all = steam();
    all.extend(epic());
    all.extend(gog());
    all.extend(ubisoft());
    all.extend(xbox());
    all.sort_by(|a, b| a.dir.cmp(&b.dir));
    all.dedup_by(|a, b| a.dir == b.dir);
    all
}

fn steam() -> Vec<Installed> {
    let root = win::reg_string(Hive::User, r"Software\Valve\Steam", "SteamPath")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files (x86)\Steam"));
    let mut libraries = vec![root.clone()];
    if let Ok(text) = fs::read_to_string(root.join("steamapps").join("libraryfolders.vdf")) {
        libraries.extend(vdf_pairs(&text).into_iter().filter(|(k, _)| k.eq_ignore_ascii_case("path")).map(|(_, v)| v.into()));
    }
    let mut games = Vec::new();
    for lib in libraries {
        let apps = lib.join("steamapps");
        for file in files_with_ext(&apps, "acf") {
            let Some((id, name, dir)) = fs::read_to_string(&file).ok().and_then(|t| parse_acf(&t)) else {
                continue;
            };
            if !STEAM_TOOLS.contains(&id) {
                games.extend(Installed::new(&name, &apps.join("common").join(dir).to_string_lossy()));
            }
        }
    }
    games
}

fn epic() -> Vec<Installed> {
    let data = std::env::var_os("ProgramData").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\ProgramData"));
    let dir = data.join(r"Epic\EpicGamesLauncher\Data\Manifests");
    files_with_ext(&dir, "item")
        .iter()
        .filter_map(|f| parse_epic(&fs::read_to_string(f).ok()?))
        .collect()
}

fn gog() -> Vec<Installed> {
    const KEY: &str = r"SOFTWARE\WOW6432Node\GOG.com\Games";
    win::reg_subkeys(Hive::Machine, KEY)
        .iter()
        .filter_map(|id| {
            let key = format!(r"{KEY}\{id}");
            let name = win::reg_string(Hive::Machine, &key, "gameName")?;
            Installed::new(&name, &win::reg_string(Hive::Machine, &key, "path")?)
        })
        .collect()
}

fn ubisoft() -> Vec<Installed> {
    const KEY: &str = r"SOFTWARE\WOW6432Node\Ubisoft\Launcher\Installs";
    win::reg_subkeys(Hive::Machine, KEY)
        .iter()
        .filter_map(|id| {
            let dir = win::reg_string(Hive::Machine, &format!(r"{KEY}\{id}"), "InstallDir")?;
            // Ubisoft keeps no name here. The folder is named after the game.
            let name = Path::new(dir.trim_end_matches(['/', '\\'])).file_name()?.to_string_lossy().into_owned();
            Installed::new(&name, &dir)
        })
        .collect()
}

/// Xbox app games. Each drive's `.GamingRoot` file names the folder that holds them.
fn xbox() -> Vec<Installed> {
    let mut games = Vec::new();
    for drive in win::fixed_drives() {
        let Ok(bytes) = fs::read(Path::new(&drive).join(".GamingRoot")) else {
            continue;
        };
        for folder in parse_gaming_root(&bytes) {
            let Ok(entries) = fs::read_dir(Path::new(&drive).join(folder)) else {
                continue;
            };
            for e in entries.flatten().filter(|e| e.path().is_dir()) {
                games.extend(Installed::new(&e.file_name().to_string_lossy(), &e.path().to_string_lossy()));
            }
        }
    }
    games
}

fn files_with_ext(dir: &Path, ext: &str) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case(ext)))
        .collect()
}

/// Every `"key" "value"` pair in a Valve KeyValues (VDF or ACF) file, in order, at any depth.
fn vdf_pairs(text: &str) -> Vec<(String, String)> {
    // Each token is Some(quoted string) or None for a brace.
    let mut tokens: Vec<Option<String>> = Vec::new();
    let mut chars = text.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                let mut s = String::new();
                while let Some(c) = chars.next() {
                    match c {
                        '"' => break,
                        '\\' => s.extend(chars.next()),
                        _ => s.push(c),
                    }
                }
                tokens.push(Some(s));
            }
            '{' | '}' => tokens.push(None),
            _ => {}
        }
    }
    let mut pairs = Vec::new();
    let mut i = 0;
    while i + 1 < tokens.len() {
        if let (Some(k), Some(v)) = (&tokens[i], &tokens[i + 1]) {
            pairs.push((k.clone(), v.clone()));
            i += 2;
        } else {
            i += 1;
        }
    }
    pairs
}

/// (app ID, name, install folder name) from a Steam `appmanifest_*.acf` file.
fn parse_acf(text: &str) -> Option<(u32, String, String)> {
    let pairs = vdf_pairs(text);
    let get = |key: &str| pairs.iter().find(|(k, _)| k.eq_ignore_ascii_case(key)).map(|(_, v)| v.clone());
    Some((get("appid")?.parse().ok()?, get("name")?, get("installdir")?))
}

/// An Epic Games Launcher `.item` manifest.
fn parse_epic(text: &str) -> Option<Installed> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    // Unreal Engine and other apps have no "games" category.
    if let Some(cats) = v["AppCategories"].as_array() {
        if !cats.iter().any(|c| c.as_str() == Some("games")) {
            return None;
        }
    }
    Installed::new(v["DisplayName"].as_str()?, v["InstallLocation"].as_str()?)
}

/// Folders, relative to the drive root, listed in a `.GamingRoot` file:
/// "RGBX", a 32-bit count, then null-terminated UTF-16 paths.
fn parse_gaming_root(bytes: &[u8]) -> Vec<String> {
    if bytes.len() < 8 || &bytes[..4] != b"RGBX" {
        return Vec::new();
    }
    let units: Vec<u16> = bytes[8..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    units
        .split(|&u| u == 0)
        .filter(|s| !s.is_empty())
        .map(String::from_utf16_lossy)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app(exe: &str, path: &str, full_screen: bool) -> Foreground {
        Foreground { exe: exe.into(), path: path.into(), full_screen, ..Default::default() }
    }

    fn is_game(fg: &Foreground, user: &[String]) -> bool {
        detect(fg, &[], user, &[]).is_some()
    }

    #[test]
    fn games() {
        assert!(is_game(&app("cs2", "", true), &[]));
        assert!(!is_game(&app("cs2x", "", false), &[]));
        assert!(!is_game(&app("Chrome", "", true), &[]));
        assert!(!is_game(&app("", "", true), &[]));
    }

    #[test]
    fn unreadable_path() {
        // Anti-cheat games: only the exe name is known, so the full-screen test decides.
        assert!(is_game(&app("SomeShooter-Win64-Shipping", "", true), &[]));
        assert!(!is_game(&app("SomeShooter-Win64-Shipping", "", false), &[]));
    }

    #[test]
    fn windowed_games() {
        assert!(is_game(&app("cs2", "", false), &[]));
        let steam = r"C:\Program Files (x86)\Steam\steamapps\common\Hades\Hades.exe";
        assert!(is_game(&app("Hades", steam, false), &[]));
        assert!(!is_game(&app("steamwebhelper", steam, false), &[]));
        assert!(!is_game(&app("chrome", steam, false), &[]));
        assert!(is_game(&app("Stardew Valley", "", false), &["stardew valley.exe".into()]));
        assert!(is_game(&app("mygame", "", false), &["MyGame.EXE".into()]));
    }

    #[test]
    fn full_screen_apps_that_are_not_games() {
        for exe in ["Microsoft.Media.Player", "mstsc", "Discord", "WINWORD", "ScreenClippingHost", "Spotify"] {
            assert!(!is_game(&app(exe, "", true), &[]), "{exe}");
        }
        // The user's list wins.
        assert!(is_game(&app("vlc", "", false), &["vlc".into()]));
        assert!(detect(&app("cs2", "", true), &[], &[], &["CS2.exe".into()]).is_none());
    }

    #[test]
    fn only_full_screen_is_a_guess() {
        assert_eq!(detect(&app("unknown", "", true), &[], &[], &[]).map(|d| d.guessed), Some(true));
        assert_eq!(detect(&app("cs2", "", true), &[], &[], &[]).map(|d| d.guessed), Some(false));
    }

    #[test]
    fn minecraft_but_not_every_java_app() {
        let mut mc = app("javaw", "", false);
        mc.title = "Minecraft* 1.21.1 - Multiplayer (3rd-party Server)".into();
        assert!(is_game(&mc, &[]));
        mc.title = "JDownloader 2".into();
        assert!(!is_game(&mc, &[]));
    }

    #[test]
    fn desktop_apps_are_not_games() {
        let code = r"C:\Users\a\AppData\Local\Programs\Microsoft VS Code\Code.exe";
        assert!(!is_game(&app("Code", code, false), &[]));
        assert!(!is_game(&app("notepad", r"C:\Windows\notepad.exe", false), &[]));
    }

    #[test]
    fn installed_games_count_anywhere_and_keep_their_store_name() {
        let installed = [
            Installed::new("Fortnite", r"D:\Games\Fortnite").unwrap(),
            Installed::new("Hades II", "E:/SteamLibrary/steamapps/common/Hades II/").unwrap(),
        ];
        let fort = app("FortniteClient-Win64-Shipping", r"D:\Games\Fortnite\FortniteGame\Binaries\Win64\x.exe", false);
        assert_eq!(detect(&fort, &installed, &[], &[]).map(|d| d.name), Some("Fortnite".into()));
        let hades = app("Hades2", r"E:\SteamLibrary\steamapps\common\Hades II\Ship\Hades2.exe", false);
        assert_eq!(detect(&hades, &installed, &[], &[]), Some(Detected { name: "Hades II".into(), guessed: false }));
        // A folder that only starts with the same letters is a different folder.
        let other = app("x", r"D:\Games\Fortnite Tools\x.exe", false);
        assert!(detect(&other, &installed, &[], &[]).is_none());
    }

    #[test]
    fn running_games_need_no_window() {
        let installed = [Installed::new("Fortnite", r"D:\Games\Fortnite").unwrap()];
        let paths = [
            r"C:\Windows\explorer.exe".to_string(),
            r"D:\Games\Fortnite\FortniteGame\Binaries\Win64\x.exe".to_string(),
        ];
        let (fg, game) = running(&paths, &installed, &[], &[]).unwrap();
        assert_eq!((fg.exe.as_str(), game.name.as_str()), ("x", "Fortnite"));
        assert!(running(&paths, &[], &[], &[]).is_none());
        assert!(running(&paths, &installed, &[], &["x".into()]).is_none());
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

    #[test]
    fn steam_files() {
        let folders = r#"
"libraryfolders"
{
	"0"
	{
		"path"		"C:\\Program Files (x86)\\Steam"
		"apps" { "228980" "0" }
	}
	"1"
	{
		"path"		"E:\\SteamLibrary"
	}
}"#;
        let paths: Vec<String> = vdf_pairs(folders).into_iter().filter(|(k, _)| k == "path").map(|(_, v)| v).collect();
        assert_eq!(paths, [r"C:\Program Files (x86)\Steam", r"E:\SteamLibrary"]);

        let acf = r#"
"AppState"
{
	"appid"		"1145350"
	"universe"		"1"
	"name"		"Hades II"
	"installdir"		"Hades II"
	"UserConfig" { "language" "english" }
}"#;
        assert_eq!(parse_acf(acf), Some((1145350, "Hades II".into(), "Hades II".into())));
        assert_eq!(parse_acf("\"AppState\" { \"appid\" \"1\" }"), None);
    }

    #[test]
    fn epic_manifest() {
        let game = r#"{"DisplayName":"Fortnite","InstallLocation":"C:\\Program Files\\Epic Games\\Fortnite","AppCategories":["public","games","applications"]}"#;
        assert_eq!(parse_epic(game), Installed::new("Fortnite", r"C:\Program Files\Epic Games\Fortnite"));
        let engine = r#"{"DisplayName":"Unreal Engine","InstallLocation":"C:\\UE_5.4","AppCategories":["engines"]}"#;
        assert_eq!(parse_epic(engine), None);
    }

    #[test]
    fn gaming_root() {
        let mut bytes = b"RGBX".to_vec();
        bytes.extend(1u32.to_le_bytes());
        for u in "XboxGames".encode_utf16().chain([0]) {
            bytes.extend(u.to_le_bytes());
        }
        assert_eq!(parse_gaming_root(&bytes), ["XboxGames"]);
        assert!(parse_gaming_root(b"nope").is_empty());
    }
}
