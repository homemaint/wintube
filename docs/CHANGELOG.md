# Changelog

## 0.1.7 - 2026-11-11

- **Selectable playback quality (the big one):** the watch page now has a 144p–1080p quality picker instead of being locked to 360p. YouTube serves video-only + audio-only adaptive streams above 360p, so playback now runs in a dual-element mode (muted video element + hidden audio element, kept in sync for play/pause/seek/volume/mute and drift correction) with an automatic fallback to the progressive 360p stream if the adaptive stream stalls or WebView2 refuses it (error code 4).
- **Stream resolution via InnerTube VISIONOS client:** videos where YouTube enables SABR-only streaming only expose a direct progressive 360p URL to yt-dlp. The player API's VISIONOS client still returns direct adaptive URLs for the full AVC ladder (144p–1080p) plus AAC audio when a `visitorData` session is supplied, so `get_playback_streams` now resolves through InnerTube first (~0.7–1s) with yt-dlp as the fallback. The signed URLs stream through a new `wintube-media` custom protocol relay (8 MiB range chunks, HEAD/416 handling, Content-Range) so the URLs never leave the app.
- **Retry button:** if HD can't be resolved, the picker shows "360p only (HD unavailable)" with a Retry HD button instead of a dead control.
- **Version label:** the top bar now shows the app version next to the logo (`WinTube-v0.1.7`).

## 0.1.6 - 2026-11-10

- **Search pagination:** "Load more" button on search results (accumulated pages, 20 cards per page, cap raised from 12).

## 0.1.5 - 2026-11-09

- **Back navigation:** topbar back arrow + Alt+Left shortcut; going back from a video returns to the originating search/history page (search results restored from cache, no re-fetch).
- Docs: screenshots added (home, watch, downloads, history, settings); LICENSE is pure MIT (third-party notices moved to docs/LICENSE-NOTICE.md).

## 0.1.4 — 2026-11-08

- **Fragmented-M4A audio fix (silent downloads):** when yt-dlp's `FixupM4a` hasn't run, the audio file is still a fragmented DASH M4A. The remuxer now demuxes fragmented audio with its own `fmp4` (`tfhd`/`tfdt`/`trun`) parser instead of the `mp4` crate (which returned the file header as every sample). Output is byte-identical to the flat-M4A path.
- **Downloads page no longer sticks:** progress events repaint the list from local state (no fetch race); snapshots merge keeping the furthest-along entry; 1.5 s polling fallback while a job is in flight.
- Remux now logs `audio_fragmented`, sample counts, and output bytes for post-hoc diagnosis.

## 0.1.3 — 2026-11-08

- **Audio-timeline hardening:** if the source audio `stts` deltas are all zero (seen in `FixupM4a`-rewritten files), timestamps are rebuilt from the AAC frame size (1024 samples) so muxide no longer writes `stts = (N, 0)` and a broken timeline. Validated against an artificially zero-delta file.

## 0.1.2 — 2026-11-08

- **Fullwidth-pipe filename fix:** yt-dlp stdout dropped `｜` (U+FF5C) on Windows; added `--encoding utf-8` and a `[videoId]` directory scan (`resolve_downloads`) so jobs complete even when reported destinations don't exist. Merged-output (`.merged.mp4`) recovery + path dedupe included.
- Your stalled downloads merge correctly; verified end-to-end (Pending → Downloading → Merging → Done).

## 0.1.1

- Download template fix `%(height)p` → `%(height)sp` (yt-dlp "unsupported format character 'p'").
- First shipped build with custom icon, YouTube-like UI, 144p–8K ladder.

## 0.1.0

- Initial release: Tauri 2 + WebView2, InnerTube search, yt-dlp sidecar, `wintube-mux` pure-Rust remux, History/Settings/SponsorBlock.

