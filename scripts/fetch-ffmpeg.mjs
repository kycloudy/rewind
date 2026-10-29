// Downloads a Windows FFmpeg build and places it where Tauri expects the sidecar.
// Usage: npm run fetch-ffmpeg
import { execFileSync } from "node:child_process";
import { copyFileSync, existsSync, mkdirSync, readdirSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

const BASE = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/";
// Pinned release branch first, rolling master build as the fallback.
const URLS = process.env.FFMPEG_URL
  ? [process.env.FFMPEG_URL]
  : [BASE + "ffmpeg-n8.1-latest-win64-gpl-8.1.zip", BASE + "ffmpeg-master-latest-win64-gpl.zip"];
const target = join("src-tauri", "binaries", "ffmpeg-x86_64-pc-windows-msvc.exe");

if (existsSync(target) && !process.argv.includes("--force")) {
  console.log(`FFmpeg is already in place: ${target}`);
  process.exit(0);
}

const work = join(tmpdir(), `rewind-ffmpeg-${Date.now()}`);
mkdirSync(work, { recursive: true });
const zip = join(work, "ffmpeg.zip");

let saved = false;
for (const url of URLS) {
  console.log(`Downloading ${url}`);
  const res = await fetch(url);
  if (!res.ok) {
    console.log(`HTTP ${res.status}`);
    continue;
  }
  writeFileSync(zip, Buffer.from(await res.arrayBuffer()));
  saved = true;
  break;
}
if (!saved) throw new Error("Could not download FFmpeg. Set FFMPEG_URL to a win64 GPL zip and run again.");

if (process.platform === "win32") {
  execFileSync(
    "powershell",
    ["-NoProfile", "-Command", `Expand-Archive -LiteralPath '${zip}' -DestinationPath '${work}' -Force`],
    { stdio: "inherit" },
  );
} else {
  execFileSync("unzip", ["-q", zip, "-d", work], { stdio: "inherit" });
}

function find(dir) {
  for (const name of readdirSync(dir)) {
    const p = join(dir, name);
    if (statSync(p).isDirectory()) {
      const hit = find(p);
      if (hit) return hit;
    } else if (name.toLowerCase() === "ffmpeg.exe") return p;
  }
  return null;
}

const exe = find(work);
if (!exe) throw new Error("ffmpeg.exe was not in the archive");
mkdirSync(join("src-tauri", "binaries"), { recursive: true });
copyFileSync(exe, target);
rmSync(work, { recursive: true, force: true });
console.log(`Saved ${target}`);
