# Rewind

Rewind is a small gameplay clipping app for Windows. It keeps the last 30 seconds of your screen in memory. Press **F8** and Rewind saves those seconds as a clip.

## Features

- **Instant replay.** F8 saves the last 15 s to 5 min. The clip includes the moment you pressed the key.
- **Games only.** By default, Rewind records only while a full-screen or borderless game is in front. It keeps recording for 60 s after you leave the game. Turn this off in Settings to record all the time.
- **Session recording.** F9 starts and stops a full recording. It uses the same capture, so it adds no extra GPU load.
- **GPU capture.** Rewind captures with Desktop Duplication and encodes on your GPU (NVIDIA NVENC, AMD AMF, or Intel Quick Sync). If no GPU encoder works, it uses the CPU (x264).
- **Game and microphone audio.** Rewind records system sound and, if you turn it on, your microphone. A noise gate can mute the microphone while you do not speak.
- **Game detection.** Rewind names and sorts each clip by the game in the foreground.
- **Library.** Clips are grouped by day and by game. You can search, add favorites, and rename clips.
- **Trimmer.** Drag the handles on the frame strip, or press **I** and **O** to set the start and end.
- **Discord export.** Export a clip at 10 MB or 50 MB. Rewind sets the bitrate and the resolution to fit.
- **Copy file.** Copy a clip to the clipboard, then paste it into Discord or a folder.
- **Tray app.** Closing the window keeps Rewind in the tray. Rewind can start with Windows.

Clips save to `Videos\Rewind\<Game>\` as MP4 (H.264 and AAC).

## Get the installer (no local setup)

1. Create a new GitHub repository and push this folder to the `main` branch.
2. Open the **Actions** tab. Wait for **Build Windows installer** to finish (about 10 minutes).
3. Open the run and download the **Rewind-setup** artifact.
4. Run `Rewind_0.1.0_x64-setup.exe`.

To publish a release, push a tag such as `v0.1.0`. The workflow attaches the installer to the release.

## Build on your PC

You need [Node.js 22](https://nodejs.org), [Rust](https://rustup.rs), and the Microsoft C++ Build Tools. Windows 10 and 11 already include WebView2.

```powershell
npm install
npm run fetch-ffmpeg     # downloads ffmpeg.exe into src-tauri/binaries
npx tauri dev            # run with hot reload
npx tauri build          # make the installer in src-tauri/target/release/bundle/nsis
```

To work on the UI only, run `npm run dev` and open http://localhost:1420. In a browser, the UI uses sample clips.

## How it works

```
ddagrab (GPU frames) ──► NVENC / AMF / QSV ──► 2-second .ts segments in a ring buffer
WASAPI loopback + mic ──► Rust mixer (48 kHz) ──► FFmpeg stdin ──┘
```

- One FFmpeg process writes 2-second segments into a ring buffer in `%LOCALAPPDATA%\app.rewind.clips\buffer`. The ring holds a little more than one clip.
- **Save clip** waits for the current segment to close. Then it joins the newest segments into an MP4 without re-encoding. Saving is almost instant and costs no quality.
- **Record** copies each finished segment aside and joins them when you stop.
- Scaling to 1440p, 1080p, or 720p happens on the GPU (`scale_d3d11`), so frames stay off the CPU at every resolution.
- If the first capture method fails, Rewind tries the next one: GPU zero-copy, then GPU with a frame copy, then CPU.
- Export and trim re-encode, so cuts are exact to the frame.

| File | Job |
| --- | --- |
| `src-tauri/src/recorder.rs` | Capture process, ring buffer, clip save, recording, restarts |
| `src-tauri/src/audio.rs` | System and mic capture, resampling, mixing |
| `src-tauri/src/gate.rs` | Microphone noise gate |
| `src-tauri/src/ffmpeg.rs` | Encoder detection, join, thumbnails, export |
| `src-tauri/src/win.rs` | Foreground game, displays, clipboard, sound |
| `src-tauri/src/lib.rs` | Commands, hotkeys, tray, settings |
| `src/` | React UI |

## Known limits

- Rewind captures one display, not one window. Use borderless or windowed full-screen mode for the most reliable capture. Some exclusive full-screen games block Desktop Duplication.
- On PCs with two GPUs (many laptops), the display list may not match the order Windows uses. If the capture is black, choose the other display in Settings.
- The installer bundles a GPL build of FFmpeg. That is fine for personal use. Read the FFmpeg license before you distribute Rewind.
