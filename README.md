# Rewind

Rewind is a small gameplay clipping app for Windows. It keeps the last 30 seconds of your screen in memory. Press **F8** and Rewind saves those seconds as a clip.

## Features

- **Instant replay.** F8 saves the last 15 s to 5 min. The clip includes the moment you pressed the key.
- **Games only.** By default, Rewind records only while a full-screen or borderless game is in front. It keeps recording for 60 s after you leave the game. Turn this off in Settings to record all the time.
- **Session recording.** F9 starts and stops a full recording. It uses the same capture, so it adds no extra GPU load.
- **GPU capture.** Rewind captures with Desktop Duplication and encodes on your GPU (NVIDIA NVENC, AMD AMF, or Intel Quick Sync). If no GPU encoder works, it uses the CPU (x264). In Settings, you can pick HEVC or AV1 instead of H.264 if your GPU supports it. Clips are about 30 to 40% smaller. HEVC and AV1 clips may not play in every app. If the GPU cannot start the codec, Rewind uses H.264.
- **Game and microphone audio.** Rewind records system sound and, if you turn it on, your microphone. A noise gate can mute the microphone while you do not speak.
- **Game detection.** Rewind names and sorts each clip by the game in the foreground.
- **Library.** Clips are grouped by day and by game. You can search, add favorites, and rename clips.
- **Trimmer.** Drag the handles on the frame strip, or press **I** and **O** to set the start and end.
- **Discord export.** Export a clip at 10 MB or 50 MB. Rewind sets the bitrate and the resolution to fit.
- **Copy file.** Copy a clip to the clipboard, then paste it into Discord or a folder.
- **Tray app.** Closing the window keeps Rewind in the tray. Rewind can start with Windows.
- **Updates.** Rewind checks GitHub Releases when you open it. Click **Update** to install the new version. Rewind restarts when the update is done.

Clips save to `Videos\Rewind\<Game>\` as MP4 (H.264, HEVC, or AV1, and AAC).

## Get the installer (no local setup)

1. Create a new GitHub repository and push this folder to the `main` branch.
2. Open the **Actions** tab. Wait for **Build Windows installer** to finish (about 10 minutes).
3. Open the run and download the **Rewind-setup** artifact.
4. Run `Rewind_0.0.4_x64-setup.exe`.

To publish a release, push a tag such as `v0.0.5`. The workflow sets the app version from the tag and attaches the installer to the release.

### Set up in-app updates (one time)

Rewind installs an update only if the installer has a valid signature. The release workflow signs the installer and attaches `latest.json`, which tells the app about the new version.

1. On your PC, make a key pair:

   ```powershell
   npx @tauri-apps/cli signer generate -w "$HOME\.tauri\rewind.key"
   ```

   Enter a password when it asks. Keep the key file and the password safe. If you lose them, installed copies cannot update.
2. In the repository, open **Settings > Secrets and variables > Actions** and add these repository secrets:

   | Secret | Value |
   | --- | --- |
   | `TAURI_SIGNING_PRIVATE_KEY` | The contents of `rewind.key` |
   | `TAURI_SIGNING_PRIVATE_KEY_PASSWORD` | The password |
   | `TAURI_UPDATER_PUBKEY` | The contents of `rewind.key.pub` |

3. Push a new tag. The release gets the installer and `latest.json`.

The tag version must be higher than the installed version, or the app does not offer the update.

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
ddagrab (GPU frames) ──► NVENC / AMF / QSV ──► MPEG-TS on stdout ──► 2-second chunks in RAM
WASAPI loopback + mic ──► Rust mixer (48 kHz) ──► FFmpeg stdin ──┘
```

- One FFmpeg process writes an MPEG-TS stream to Rewind. Rewind cuts it at each keyframe (every 2 s) and keeps a little more than one clip in RAM. The buffer writes nothing to disk.
- RAM use is about the bitrate times the clip length. At "high" on a 1440p display, that is about 100 MB for 30 s and about 1 GB for 5 min.
- **Save clip** waits for the current chunk to close. Then it remuxes the newest chunks into an MP4 without re-encoding. Saving is almost instant and costs no quality.
- **Record** writes each finished chunk to a file in `%LOCALAPPDATA%\app.rewind.clips\sessions` and makes an MP4 when you stop.
- Scaling to 1440p, 1080p, or 720p happens on the GPU (`scale_d3d11`), so frames stay off the CPU at every resolution.
- If the first capture method fails, Rewind tries the next one: GPU zero-copy, then GPU with a frame copy, then CPU.
- Export and trim re-encode, so cuts are exact to the frame.

| File | Job |
| --- | --- |
| `src-tauri/src/recorder.rs` | Capture process, clip save, recording, restarts |
| `src-tauri/src/ring.rs` | In-memory replay buffer: cuts the TS stream at keyframes |
| `src-tauri/src/audio.rs` | System and mic capture, resampling, mixing |
| `src-tauri/src/gate.rs` | Microphone noise gate |
| `src-tauri/src/ffmpeg.rs` | Encoder detection, join, thumbnails, export |
| `src-tauri/src/win.rs` | Foreground game, displays, clipboard, sound |
| `src-tauri/src/update.rs` | In-app updates from GitHub Releases |
| `src-tauri/src/lib.rs` | Commands, hotkeys, tray, settings |
| `src/` | React UI |

## Known limits

- Rewind captures one display, not one window. Use borderless or windowed full-screen mode for the most reliable capture. Some exclusive full-screen games block Desktop Duplication.
- On PCs with two GPUs (many laptops), the display list may not match the order Windows uses. If the capture is black, choose the other display in Settings.
- The installer bundles a GPL build of FFmpeg. That is fine for personal use. Read the FFmpeg license before you distribute Rewind.
