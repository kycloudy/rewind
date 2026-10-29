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
};

export type Status = {
  buffering: boolean;
  waitingForGame: boolean;
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
  clipSeconds: number;
  fps: number;
  quality: "low" | "medium" | "high" | "ultra";
  resolution: "native" | "1440" | "1080" | "720";
  encoder: string;
  monitor: number;
  systemAudio: boolean;
  mic: boolean;
  micDevice: string | null;
  micVolume: number;
  micGate: boolean;
  micGateThreshold: number;
  clipHotkey: string;
  recordHotkey: string;
  clipsDir: string;
  launchAtLogin: boolean;
  clipSound: boolean;
};

export type Devices = {
  monitors: { index: number; name: string; width: number; height: number; primary: boolean }[];
  mics: string[];
  encoders: { name: string; label: string }[];
};

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
  filmstrip: (id: string) => call<string>("filmstrip", { id }),
  exportClip: (id: string, start: number, end: number, targetMb: number | null) =>
    call<Clip>("export_clip", { req: { id, start, end, targetMb } }),
};

export async function pickFolder(): Promise<string | null> {
  if (!isDesktop) return null;
  const { open } = await import("@tauri-apps/plugin-dialog");
  const r = await open({ directory: true });
  return typeof r === "string" ? r : null;
}
