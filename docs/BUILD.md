# Building from source

## Prerequisites

- **Rust** with the `x86_64-pc-windows-gnu` target (pinned via `rust-toolchain.toml`)
- **Node.js 18+** (24 LTS verified)
- **MinGW-w64** on PATH — needed for `ring` / C build scripts (e.g. WinLibs at `C:\mingw64\bin`)
- **WiX Toolset 3** (for MSI) and **NSIS** (for the setup EXE) — required by `tauri build` for bundles

## Important: spaceless path

The Windows resource compiler (`windres`) **fails on paths containing spaces**. The workspace path must not contain spaces when compiling.

- ✅ Build from `C:\Wintube`
- ❌ `D:\My Projects\Wintube` — spaces break `windres`

Recommended: keep the source of record in `D:\My Projects\Wintube`, and sync to a spaceless folder before building:

```powershell
robocopy "D:\My Projects\Wintube" C:\Wintube /E /XD target node_modules dist .git /NFL /NDL /NJH /NJS /NP
```

## Build

From the spaceless folder:

```powershell
$env:PATH = "C:\mingw64\bin;$env:PATH"
npm install
npx tauri build
```

Output:

```text
target\x86_64-pc-windows-gnu\release\bundle\msi\Wintube_0.1.4_x64_en-US.msi
target\x86_64-pc-windows-gnu\release\bundle\nsis\Wintube_0.1.4_x64-setup.exe
```

> `cargo test` / `cargo run -p wintube-mux` also need the spaced-path caveat — run them from the spaceless mirror.

## Run in dev

```powershell
$env:PATH = "C:\mingw64\bin;$env:PATH"
npm install
npx tauri dev
```

Runs the Vite dev server (`http://localhost:5173`) inside a Tauri window.

## Sidecars

`src-tauri/sidecars/yt-dlp.exe` is **not committed** to git (see `.gitignore`). The release workflow downloads it. For local dev, drop it next to the binary:

- Installed layout: `<exe>/yt-dlp.exe` or `<exe>/sidecars/yt-dlp.exe`
- Dev layout: `src-tauri/sidecars/yt-dlp.exe`

Fetch it with:

```powershell
Invoke-WebRequest https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe -OutFile src-tauri\sidecars\yt-dlp.exe
```

`WebView2Loader.dll` must sit next to `wintube.exe`. `tauri build` patches it in automatically.

## Project layout

```text
Cargo.toml            # workspace root
crates/
  wintube-core        # models: Video/Format/Quality/DownloadJob + yt-dlp ladder
  wintube-yt          # YtClient (yt-dlp -j) + InnerTube search
  wintube-download    # DownloadManager (yt-dlp --newline, remux/fallback)
  wintube-mux         # pure-Rust MP4 demux + remux (fmp4.rs + remux.rs)
  wintube-storage     # JSON history/settings in %APPDATA%/Wintube
  wintube-player       # URL picker / mpv args
src/                 # frontend (vanilla JS, src/main.js)
src-tauri/            # Tauri app: commands + tauri.conf.json
docs/                 # this documentation
```
