<div align="center">

# WinTube — YouTube without ads, on Windows

**Ad-free, lightweight YouTube client for Windows with 144p–8K downloads. Open-source Rust + Tauri 2 desktop app.**

[![License: MIT](https://img.shields.io/badge/License-MIT-yellow.svg)](LICENSE)
[![Windows](https://img.shields.io/badge/Platform-Windows-0078D4)](https://github.com/homemaint/wintube/releases)
[![Tauri 2](https://img.shields.io/badge/Tauri-2-FFC131)](https://tauri.app)
[![Rust](https://img.shields.io/badge/Rust-Cargo-orange)](https://www.rust-lang.org)
[![Release](https://img.shields.io/github/v/homemaint/wintube)](https://github.com/homemaint/wintube/releases)

**[Download latest release](https://github.com/homemaint/wintube/releases/latest)** · [Read docs](docs/) · [Report an issue](https://github.com/homemaint/wintube/issues)

</div>

> **No ads. No sign-in. No account needed.** Watch YouTube (or your favorite videos) on a fast, lightweight desktop app, and download for offline playback at up to 8K.

## What is WinTube?

WinTube is an **ad-free YouTube player and YouTube downloader for Windows**. It is a lightweight **YouTube without ads** desktop client with selectable-quality downloads (144p to 8K), built in **Rust + Tauri 2 + WebView2**. Videos play in a familiar, **YouTube-like UI** — sidebar, search, watch grid — with zero ads.

If you are looking for:
- **download youtube 1080p, 4k, or 8k MP4** on Windows,
- an **ad-free YouTube client** for your PC,
- a **lightweight YouTube player** (no Electron bloat, no tracking),
- **watch YouTube offline** after downloading,
- or an **open-source YouTube alternative** modeled after SmartTube,

WinTube is for you.

## Features

- **YouTube without ads** — no pre-roll, no mid-roll, no promo overlays
- **144p–8K downloads** with a full quality ladder: 4320p (8K), 2160p (4K), 1440p (QHD), 1080p (FHD), 720p (HD), 480p, 360p, 240p, 144p
- **Codec presets:** AVC (H.264), AV1, VP9, Auto
- **Pure-Rust remux — no ffmpeg required for most downloads:** `wintube-mux` demuxes fragmented MP4 and re-muxes H.264 + AAC to a single fast-start MP4. This is the `ffmpeg -c copy` equivalent, built in Rust.
- **Ad-block-style playback:** SponsorBlock auto-skip, subtitle selection, history + resume
- **No login required** — search and watch without a Google account
- **Lightweight on disk and RAM:** ~25 MB installer (MSI), ~5 MB EXE, WebView2 runtime
- **Configurable download folder**, default quality/codec, dark YouTube-like theme
- **Offline playback:** downloaded files are standard MP4 (H.264 + AAC)

## Screenshots

| Home / Search | Downloads | Watch |
| --- | --- | --- |
| *Home grid with trending videos* | *Progress + ETA in the Downloads page* | *Player with quality/codec picker* |

> Screenshots are on the Releases page and in the GitHub social preview. Drop yours into `docs/screenshots/` and link them here.

## Quick start

### 1. Install (Windows 10/11, x64)

Download the latest MSI or NSIS installer:

- **MSI (recommended):** `Wintube_<version>_x64_en-US.msi`
- **NSIS setup:** `Wintube_<version>_x64-setup.exe`

Both are on the [Releases page](https://github.com/homemaint/wintube/releases).

The installer places `Wintube.exe` + `yt-dlp.exe` sidecar next to it. WinTube requires the **WebView2 Evergreen Runtime** (already present on most Windows 10/11 machines).

### 2. Use it

1. Launch **WinTube** from the Start menu.
2. **Search** (press `/`) or browse Home.
3. Click a card to **watch**.
4. On the watch page, pick **quality (144p–8K)** + **codec (Auto/AVC/AV1/VP9)**, press **Download**.
5. Track progress in **Downloads**, or find the finished MP4 in your Downloads folder (`Downloads\Wintube\` by default).

### 3. Example: download a video at 1080p

1. Open your video → click **Download**.
2. Select **1080p · FHD** + **Codec: Auto**.
3. Wait for `Pending → Downloading → Merging → Done`.
4. Open the `.mp4` — H.264 video + AAC stereo audio, decodable in any player (VLC, mpv, browser).

## Tech stack

- **Rust** — workspace of small crates (`wintube-core`, `wintube-yt`, `wintube-download`, `wintube-mux`, `wintube-storage`, `wintube-player`)
- **Tauri 2 + WebView2** — desktop shell, React-free vanilla JS frontend (`src/main.js`)
- **yt-dlp sidecar** — format resolution, download, signature handling (same role SmartTube's decipher layer plays)
- **InnerTube-shaped endpoints** — search primary, yt-dlp fallback
- **`muxide` + `mp4` crates** — pure-Rust MP4 write/read for the remuxer

See [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md) for the full breakdown.

## Docs

- [Installation](docs/INSTALL.md) — MSI vs NSIS, file paths, WebView2
- [Building from source](docs/BUILD.md) — prerequisites, spaceless path note, `npx tauri build`
- [Architecture](docs/ARCHITECTURE.md) — crates, download pipeline, remux pipeline
- [Troubleshooting](docs/TROUBLESHOOTING.md) — 429/bot block, fragmented M4A, fullwidth-`｜` filenames, WebView2Loader
- [Changelog](docs/CHANGELOG.md) — release notes
- [License notices](docs/LICENSE-NOTICE.md) — third-party attribution

## Roadmap

- [x] Watch + search + downloads + history/settings + SponsorBlock
- [x] Pure-Rust H.264/AAC remux (no ffmpeg for Auto/AVC), fragmented-M4A audio support
- [ ] Full-quality streaming (DASH / MSE / libmpv) for AV1/VP9/8K playback
- [ ] Comments + live chat (full-clone scope)
- [ ] AV1/VP9 remux in Rust (currently falls back to ffmpeg)
- [ ] One-click yt-dlp updater, Deno/QuickJS for signature solving
- [ ] Subscriptions, channel/playlist pages, PiP

## Keywords (for search)

youtube without ads · ad-free youtube client · youtube downloader windows · download youtube 1080p 4k 8k mp4 · lightweight youtube player · watch youtube offline · best youtube downloader for windows · youtube adblock desktop · open-source youtube client · privacy youtube alternative · no sign-in youtube player

## Contributing

PRs and issues welcome — see [CONTRIBUTING.md](CONTRIBUTING.md) and the [issue templates](.github/ISSUE_TEMPLATE/).

## License

MIT — see [LICENSE](LICENSE). WinTube is an independent project, not affiliated with or endorsed by Google LLC or YouTube.

## Disclaimer

For personal use. YouTube and all video content remain the property of their respective creators and YouTube/Google LLC. Downloading may conflict with YouTube's Terms of Service — you are responsible for how you use the software. WinTube does not provide DRM bypass and does not host or redistribute content.
