# Changelog

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
