import { invoke, convertFileSrc } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { mock } from "./mock";

export type Clip = {
  id: string;
  path: string;
  thumb: string | null;
  title: string;
  game: string;
  created: number;
  duration: number;
  size: number;
  favorite: boolean;
  kind: "clip" | "recording" | "export";
  /** Seconds. The part to play and share. The file keeps every frame. */
  trim: Trim | null;
};

export type Trim = { start: number; end: number };

export type Game = { name: string; exe: string; guessed: boolean };

export type Status = {
  buffering: boolean;
  waitingForGame: boolean;
  game: Game | null;
  recording: boolean;
  recordingSince: number | null;
  encoder: string | null;
  clipSeconds: number;
  saving: boolean;
  error: string | null;
};

export type Settings = {
  bufferEnabled: boolean;
  gamesOnly: boolean;
  gameApps: string[];
  notGames: string[];
  clipSeconds: number;
  fps: number;
  quality: "low" | "medium" | "high" | "ultra";
  resolution: "native" | "1440" | "1080" | "720";
  encoder: string;
  codec: "h264" | "hevc" | "av1";
  monitor: number;
  systemAudio: boolean;
  mic: boolean;
  micDevice: string | null;
  micVolume: number;
  micGate: boolean;
  micGateThreshold: number;
  micDenoise: boolean;
  clipHotkey: string;
  recordHotkey: string;
  clipsDir: string;
  launchAtLogin: boolean;
  clipSound: boolean;
};

export type Devices = {
  monitors: { index: number; name: string; width: number; height: number; primary: boolean }[];
  mics: string[];
  encoders: { name: string; label: string; codec: "h264" | "hevc" | "av1"; vendor: string }[];
};

export type RunningApp = { exe: string; title: string };

export type UpdateInfo = { current: string; available: string | null };

/** True inside the desktop app. In a plain browser the UI runs on sample data. */
export const isDesktop = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

function call<T>(cmd: string, args?: Record<string, unknown>): Promise<T> {
  return isDesktop ? invoke<T>(cmd, args) : (mock.call(cmd, args) as Promise<T>);
}

export function on<T>(event: string, cb: (payload: T) => void): () => void {
  if (!isDesktop) return mock.on(event, cb as (p: unknown) => void);
  let off: (() => void) | undefined;
  let dead = false;
  listen<T>(event, (e) => cb(e.payload)).then((u) => (dead ? u() : (off = u)));
  return () => {
    dead = true;
    off?.();
  };
}

export const fileUrl = (path: string) => (isDesktop ? convertFileSrc(path) : path);

export const api = {
  status: () => call<Status>("get_status"),
  settings: () => call<Settings>("get_settings"),
  saveSettings: (settings: Settings) => call<Settings>("save_settings", { settings }),
  notAGame: (exe: string) => call<Settings>("not_a_game", { exe }),
  runningApps: () => call<RunningApp[]>("running_apps"),
  devices: () => call<Devices>("get_devices"),
  clips: () => call<Clip[]>("list_clips"),
  saveClip: () => call<Clip>("save_clip"),
  toggleRecording: () => call<Clip | null>("toggle_recording"),
  rename: (id: string, title: string) => call<Clip>("rename_clip", { id, title }),
  favorite: (id: string, favorite: boolean) => call<Clip>("set_favorite", { id, favorite }),
  remove: (id: string) => call<void>("delete_clip", { id }),
  reveal: (id: string) => call<void>("reveal_clip", { id }),
  openFolder: () => call<void>("open_clips_folder"),
  copy: (id: string) => call<void>("copy_clip", { id }),
  setTrim: (id: string, trim: Trim | null) => call<Clip>("set_trim", { id, trim }),
  filmstrip: (id: string) => call<string>("filmstrip", { id }),
  exportClip: (id: string, start: number, end: number, targetMb: number | null) =>
    call<Clip>("export_clip", { req: { id, start, end, targetMb } }),
  checkUpdate: () => call<UpdateInfo>("check_update"),
  installUpdate: () => call<void>("install_update"),
};

export async function pickFolder(): Promise<string | null> {
  if (!isDesktop) return null;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const r = await open({ directory: true });
  return typeof r === "string" ? r : null;
}

export async function appVersion(): Promise<string> {
  if (!isDesktop) return "dev";
  const { getVersion } = await import("@tauri-apps/api/app");
  return getVersion();
}

export async function openUrl(url: string): Promise<void> {
  if (!isDesktop) {
    window.open(url, "_blank", "noopener");
    return;
  }
  const { openUrl } = await import("@tauri-apps/plugin-opener");
  await openUrl(url);
}
