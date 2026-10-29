// Sample data so the UI can be developed and previewed in a normal browser (`npm run dev`).
import type { Clip, Settings, Status } from "./api";

const palettes: Record<string, [string, string, string]> = {
  VALORANT: ["#3b1d2a", "#ff4655", "#f2e6d8"],
  Minecraft: ["#1d3b2a", "#5fa04e", "#8fd0ff"],
  "Rocket League": ["#13234a", "#2f7bff", "#ff8a1f"],
  "Apex Legends": ["#2a1a17", "#da3a2c", "#e8d3b0"],
};

function thumb(game: string, seed: number): string {
  const [bg, a, b] = palettes[game] ?? ["#222", "#888", "#ccc"];
  const r = (n: number) => ((Math.sin(seed * 97 + n * 13) + 1) / 2) * 100;
  const svg = `<svg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 160 90' preserveAspectRatio='xMidYMid slice'>
    <defs><linearGradient id='g' x1='0' y1='0' x2='1' y2='1'><stop offset='0' stop-color='${bg}'/><stop offset='1' stop-color='#0d1016'/></linearGradient></defs>
    <rect width='160' height='90' fill='url(#g)'/>
    <circle cx='${r(1) * 1.6}' cy='${r(2) * 0.9}' r='${18 + r(3) / 4}' fill='${a}' opacity='.55'/>
    <rect x='${r(4) * 1.2}' y='${30 + r(5) / 3}' width='${30 + r(6) / 2}' height='12' rx='2' fill='${b}' opacity='.35'/>
    <path d='M0 ${70 + r(7) / 10} Q80 ${50 + r(8) / 5} 160 ${72 - r(9) / 10} V90 H0Z' fill='#000' opacity='.35'/>
  </svg>`;
  return "data:image/svg+xml;utf8," + encodeURIComponent(svg);
}

const now = Date.now();
const H = 3600_000;
const games = ["VALORANT", "Minecraft", "Rocket League", "Apex Legends"];
let clips: Clip[] = [
  ["VALORANT", "Ace on Ascent", 30.4, 0.6, true],
  ["VALORANT", "Operator flick", 30.1, 1.4, false],
  ["Minecraft", "Creeper ambush", 30.2, 3.2, false],
  ["Rocket League", "Ceiling shot", 29.9, 20, true],
  ["Rocket League", "Rocket League session", 1520, 22, false],
  ["Apex Legends", "Last squad standing", 60.3, 26, false],
  ["Minecraft", "Elytra landing", 30.0, 50, false],
  ["VALORANT", "Clutch 1v3", 30.3, 74, false],
  ["Apex Legends", "Kraber headshot", 30.0, 98, true],
].map(([game, title, duration, hoursAgo, favorite], i) => ({
  id: `m${i}`,
  path: "",
  thumb: thumb(game as string, i + 1),
  title: title as string,
  game: game as string,
  created: now - (hoursAgo as number) * H,
  duration: duration as number,
  size: Math.round((duration as number) * 3.1e6),
  favorite: favorite as boolean,
  kind: (duration as number) > 300 ? "recording" : "clip",
}));

let settings: Settings = {
  bufferEnabled: true,
  gamesOnly: true,
  gameApps: [],
  clipSeconds: 30,
  fps: 60,
  quality: "high",
  resolution: "native",
  encoder: "auto",
  monitor: 0,
  systemAudio: true,
  mic: false,
  micDevice: null,
  micVolume: 1,
  micGate: false,
  micGateThreshold: -45,
  clipHotkey: "F8",
  recordHotkey: "F9",
  clipsDir: "C:\\Users\\Husa\\Videos\\Rewind",
  launchAtLogin: true,
  clipSound: true,
};

let status: Status = {
  buffering: true,
  waitingForGame: false,
  recording: false,
  recordingSince: null,
  encoder: "NVIDIA NVENC",
  clipSeconds: 30,
  saving: false,
  error: null,
};

const listeners = new Map<string, Set<(p: unknown) => void>>();
const emit = (e: string, p: unknown) => listeners.get(e)?.forEach((f) => f(p));
const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

export const mock = {
  on(event: string, cb: (p: unknown) => void) {
    if (!listeners.has(event)) listeners.set(event, new Set());
    listeners.get(event)!.add(cb);
    return () => listeners.get(event)!.delete(cb);
  },
  async call(cmd: string, a: Record<string, any> = {}): Promise<unknown> {
    switch (cmd) {
      case "get_status":
        return status;
      case "get_settings":
        return settings;
      case "save_settings":
        settings = a.settings;
        status = { ...status, buffering: settings.bufferEnabled, clipSeconds: settings.clipSeconds };
        emit("status", status);
        return settings;
      case "get_devices":
        return {
          monitors: [
            { index: 0, name: "Main display", width: 2560, height: 1440, primary: true },
            { index: 1, name: "Display 2", width: 1920, height: 1080, primary: false },
          ],
          mics: ["Microphone (HyperX QuadCast)", "Headset Microphone (Arctis 7)"],
          encoders: [
            { name: "h264_nvenc", label: "NVIDIA NVENC" },
            { name: "libx264", label: "CPU (x264)" },
          ],
        };
      case "list_clips":
        return [...clips].sort((x, y) => y.created - x.created);
      case "save_clip": {
        emit("status", (status = { ...status, saving: true }));
        await wait(900);
        const game = games[Math.floor(Math.random() * games.length)];
        const c: Clip = {
          id: "m" + Date.now(),
          path: "",
          thumb: thumb(game, Date.now() % 97),
          title: `${game} clip`,
          game,
          created: Date.now(),
          duration: settings.clipSeconds,
          size: settings.clipSeconds * 3.1e6,
          favorite: false,
          kind: "clip",
        };
        clips.push(c);
        emit("status", (status = { ...status, saving: false }));
        emit("clip-saved", c);
        return c;
      }
      case "toggle_recording":
        status = status.recording
          ? { ...status, recording: false, recordingSince: null }
          : { ...status, recording: true, recordingSince: Date.now() };
        emit("status", status);
        return null;
      case "rename_clip":
      case "set_favorite": {
        const c = clips.find((x) => x.id === a.id)!;
        if (cmd === "rename_clip") c.title = a.title;
        else c.favorite = a.favorite;
        return { ...c };
      }
      case "delete_clip":
        clips = clips.filter((x) => x.id !== a.id);
        return;
      case "filmstrip": {
        const c = clips.find((x) => x.id === a.id)!;
        return c.thumb;
      }
      case "export_clip": {
        for (let p = 0; p <= 1; p += 0.1) {
          emit("export-progress", { id: a.req.id, progress: p });
          await wait(120);
        }
        const src = clips.find((x) => x.id === a.req.id)!;
        const c: Clip = {
          ...src,
          id: "m" + Date.now(),
          title: `${src.title} (${a.req.targetMb ? a.req.targetMb + " MB" : "trimmed"})`,
          created: Date.now(),
          duration: a.req.end - a.req.start,
          kind: "export",
          favorite: false,
        };
        clips.push(c);
        return c;
      }
      default:
        return null;
    }
  },
};
