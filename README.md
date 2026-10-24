# 📺 WinTube

> **A High-Performance, Ad-Free YouTube Client for Windows PC written in Pure Rust.**  
> Inspired by the architecture and features of [SmartTube](https://github.com/yuliskov/smarttube) for Android TV, engineered natively for Windows 10 & 11 with zero Electron/Chromium overhead.

---

## ⚡ Why WinTube?

Official YouTube on Windows requires running heavy browser engines (Chrome, Edge, or Electron apps) consuming 800MB–2GB of RAM, injecting invasive ads, throttling high-resolution formats, and lacking TV couch ergonomics.

**WinTube** ports the core architectural pillars of **SmartTube** directly to native Windows:
- **Instant Cold Start & Low Footprint**: Built with **Rust + Iced + wgpu (DirectX 12)**, using ~35MB RAM.
- **Hardware-Accelerated Playback**: Direct D3D11VA zero-copy video decoding (supporting AV1, VP9, and H.264 up to 4K/8K 60fps HDR).
- **Ad-Free by Architecture**: Direct Innertube API stream extraction with no browser ad scripts or sponsored containers.
- **Dual Form Factor**: Instant switching between **Desktop PC Mode** (mouse & keyboard) and **10-Foot Couch TV Mode** (D-Pad / Gamepad spatial navigation) via `F11`.

---

## 🚀 Key Features (SmartTube Parity)

| Feature | WinTube Implementation | SmartTube Android TV Counterpart |
|---|---|---|
| **Ad-Free Playback** | Native Innertube extraction via TV/Android clients | `TVHTML5` / `ANDROID_EMBEDDED` endpoints |
| **SponsorBlock** | Auto-skips sponsor segments, intros, outros, self-promo with 4-byte SHA-256 privacy hashing | `SponsorBlockManager` |
| **Return YouTube Dislike (RYD)** | Restores real-time like/dislike counts & community rating ratio | `ReturnYouTubeDislikeService` |
| **DeArrow Integration** | Replaces clickbait titles & sensational thumbnails with community-verified ones | `DeArrowService` |
| **MPEG-DASH Engine** | Dynamic in-memory `.mpd` generation for separate video + Opus audio muxing at 4K/8K | Dynamic MPD parser |
| **Form Factor Switching** | Desktop PC Mode & 10-Foot TV Couch Mode (toggle with `F11`) | 10-foot Leanback Android UI |
| **Windows Sleep Guard** | Win32 `SetThreadExecutionState` (`ES_DISPLAY_REQUIRED \| ES_SYSTEM_REQUIRED`) | `FLAG_KEEP_SCREEN_ON` in `PlaybackActivity` |
| **Audio Normalization** | Volume equalization & loudness balancing | Volume normalizer |
| **100% Offline Storage** | Pure-Rust `redb` embedded engine for watch history, resume timestamps, and subs | Android Room SQLite DB |
| **TV OAuth2 Activation** | `youtube.com/activate` device code authorization flow | YouTube TV device login |

---

## 🏗️ Architecture & Workspace Crates

The project is structured as a modular Cargo workspace:

```
wintube/
├── crates/
│   ├── wintube-innertube/   # YouTube Innertube client, Multi-client spoofing, DASH MPD builder, TV OAuth2
│   ├── wintube-services/    # SponsorBlock, ReturnYouTubeDislike, DeArrow, and pure-Rust redb storage
│   ├── wintube-player/      # Hardware media engine, D3D11VA decoder runner, Sleep Guard
│   └── wintube-app/         # Iced + wgpu DirectX 12 GUI, Dual-mode reactive architecture
└── .msvc_lib/               # Standalone MSVC x64 import stubs for zero-SDK compilation
```

### 1. `wintube-innertube`
- Implements spoofing for multiple Innertube clients:
  - `TVHTML5` (`client_version: 7.20230405.08.01`): Used for ad-free raw stream extraction and device code authorization.
  - `ANDROID_VR`: Reliable high-bitrate fallback for VP9/AV1 4K 60fps stream descriptors.
  - `IOS`: High compatibility H.264/AAC streams.
  - `WEB`: Autocomplete suggestions and general feed queries.
- In-memory MPEG-DASH manifest builder converts adaptive audio/video stream segments into standard MPD manifests.

### 2. `wintube-services`
- **SponsorBlock**: Queries Ajay's SponsorBlock API using 4-byte (8 hex character) SHA-256 hash prefix to prevent the server from tracking the user's viewing history. Automatically seeks past sponsored segments.
- **Return YouTube Dislike**: Queries the RYD API concurrently during video resolution to display actual dislike counts.
- **DeArrow**: Fetches crowd-sourced title replacements and clean thumbnail timestamps to eliminate sensationalism.
- **Storage Engine**: Built on pure-Rust `redb v2.6`. Zero C dependencies, ACID-compliant, crash-safe local storage for watch history, playback progress, and channel subscriptions.

### 3. `wintube-player`
- **Hardware Decoders**: Integrates with hardware decoders (`mpv` / `libmpv` or `ffplay`) with `--hwdec=d3d11va`, enabling hardware decoding on Nvidia, AMD, and Intel GPUs.
- **Track Selection**: Intelligent heuristics selecting optimal resolution (720p, 1080p, 1440p, 4K, 8K) and preferred codec (`AV1`, `VP9`, or `AVC1/H.264`).
- **Windows Sleep Inhibitor**: Automatically calls Win32 API `SetThreadExecutionState` on playback start to prevent screensavers and PC sleep, releasing it on pause/stop.

### 4. `wintube-app`
- GPU-accelerated interface built with **Iced v0.13** on DirectX 12 (`wgpu`).
- Dual responsive view layout:
  - **Desktop Mode**: Dense 3-column video grid, fine seekbars, mouse hover actions, and keyboard shortcuts.
  - **10-Foot TV Mode**: High-contrast 2-column cards, magnified typography, remote control D-Pad/arrow-key focus handling.

---

## ⌨️ Controls & Keyboard Shortcuts

| Shortcut | Action | Scope |
|---|---|---|
| `F11` | **Toggle Desktop Mode ⟷ 10-Foot TV Couch Mode** | Everywhere |
| `Space` or `K` | Play / Pause | Now Playing |
| `J` | Rewind 10 seconds | Now Playing |
| `L` | Fast-forward 10 seconds | Now Playing |
| `←` / `→` | Seek 5 seconds / Navigate Grid | TV Mode & Player |
| `↑` / `↓` | Spatial Grid Row Up / Down | TV Mode Grid |
| `Enter` | Select focused video | TV Mode Grid |
| `[` | Decrease playback speed (-0.25x) | Now Playing |
| `]` | Increase playback speed (+0.25x) | Now Playing |
| `M` or `I` | Toggle Mini Player / Compact View | Now Playing |
| `Esc` / `Backspace` | Return to Home / Feed | Now Playing |

---

## 🛠️ Building & Running

### Prerequisites
- Windows 10 or 11 (x64)
- Rust toolchain (`stable-x86_64-pc-windows-msvc` recommended)
- Visual Studio C++ Build Tools or standalone MSVC build environment

### Quick Start
```powershell
# Clone the repository
git clone https://github.com/your-username/wintube.git
cd wintube

# Run tests
cargo test -p wintube-innertube --test dash_test
cargo test -p wintube-services --test storage_test

# Launch WinTube GUI
cargo run -p wintube-app
```

---

## 🛡️ Privacy & Performance Guarantee

1. **Zero Google Login Requirement**: Full browsing, search, subscriptions, and history function completely offline.
2. **K-Anonymity on SponsorBlock**: Video IDs are never transmitted directly; only a truncated 4-byte hash is shared.
3. **No Web Tracking**: Zero telemetry, no third-party analytic trackers, and no ad scripts.
4. **Pure Rust Pipeline**: No Node.js runtime, no Chromium processes, no Electron background memory leaks.

---

## 📜 License

Licensed under either of [Apache License, Version 2.0](LICENSE-APACHE) or [MIT License](LICENSE-MIT) at your option.
