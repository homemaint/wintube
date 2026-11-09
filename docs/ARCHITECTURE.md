# Architecture

## Overview

WinTube is a Tauri 2 desktop app: a Rust backend (`src-tauri` + workspace crates) and a vanilla-JS WebView2 frontend (`src/main.js`). Search uses InnerTube JSON (primary) with a yt-dlp fallback; playback and downloads go through yt-dlp for format resolution; downloads are merged (or remuxed) into a single MP4.

## Crates

| Crate | Role |
| --- | --- |
| `wintube-core` | Shared models: `Video`, `Format`, `Quality` (144p–8K ladder), `DownloadJob`, `JobStatus`, `CodecPref` |
| `wintube-yt` | `YtClient` — runs `yt-dlp -j` for `PlayInfo`; `innertube.rs` — InnerTube search + thumbnail inlining |
| `wintube-download` | `DownloadManager` — spawns yt-dlp, parses `--newline` progress, resolves destinations, orchestrates remux vs ffmpeg fallback |
| `wintube-mux` | Pure-Rust remux: `fmp4.rs` demuxer + `remux.rs` re-mux pipeline (see below) |
| `wintube-storage` | JSON persistence (history, settings) under `%APPDATA%\Wintube\` |
| `wintube-player` | Playback URL picking (progressive → DASH → HLS), mpv args |

## Download pipeline (Auto / AVC)

```
User picks quality (144p–8K) + codec (Auto/AVC)
  → quality.ytdlp_video_avc()   e.g. bestvideo[height<=1080][vcodec^=avc]
  → yt-dlp -f <video> -o video_tmpl  (progress 0–85%)
  → yt-dlp -f bestaudio[ext=m4a] -o audio_tmpl (progress 85–97%)
  → status=Merging (97%)
  → wintube-mux remux (spawn_blocking)
  → status=Done (100%), output=<title> [id] 1080p.merged.mp4
```

AV1 / VP9 requests fall back to yt-dlp's own `--merge-output-format mp4` (needs ffmpeg). These codecs cannot use the Rust remuxer yet.

## Remux pipeline (`wintube-mux`)

The goal is `ffmpeg -i video.mp4 -i audio.mp4 -c copy out.mp4` in pure Rust.

1. **Video demux.** YouTube video renditions are fragmented MP4 (DASH). `fmp4.rs` parses `tfhd`/`tfdt`/`trun` to rebuild decode-order samples (offset, size, DTS, CTS offset, keyframe flag). Non-fragmented MP4 falls back to the `mp4` crate's tables.
2. **Audio demux.** Usually a flat M4A (`mp4` crate). When yt-dlp's `FixupM4a` hasn't run, the file is still fragmented DASH — the same `fmp4.rs` parser is used. (This was the root cause of the 0.1.4 silent-audio fix: the `mp4` crate returned the file header as every "sample".)
3. **AVCC → Annex B.** H.264 length-prefixed NALs become start-code-delimited; SPS/PPS from `avcC` are prepended to the first frame.
4. **Mux.** `muxide` writes a fast-start MP4. Video is fed in **decode order** via `write_video_with_dts(pts, dts, data, is_keyframe)`; audio is ADTS-wrapped and fed in PTS order.
5. **Chunk-offset repair.** muxide interleaves `mdat` in presentation order but writes `stco` in decode order — with B-frames every entry after the first would point at the wrong sample. `patch_video_chunk_offsets()` rewrites the video `stco` in place.
6. **Timestamp hardening.** If the audio `stts` deltas are all zero (seen in `FixupM4a`-rewritten files), timestamps are rebuilt from the AAC frame size (1024 samples).

## Frontend

`src/main.js` is a single-file SPA:

- Routes: `home` / `search` / `watch` / `history` / `downloads` / `settings`
- Search results are cached per query; the grid paints skeletons immediately, then streams in cards
- `download://progress` events repaint the Downloads list from local state (no fetch race); a 1.5 s poll is the fallback
- Thumbnails are inlined as `data:` URIs by the Rust side
- Watch page uses a native `<video>` element with the picked URL; SponsorBlock skipping via `setInterval`; history saved on watch

## Commands (Tauri)

`get_play_info`, `search_videos`, `start_download`, `list_downloads`, `cancel_download`, `remux_download`, `get_settings`, `save_settings`, `get_history`, `save_history`, `get_sponsor_segments`, `ensure_sidecars`, `sidecar_diagnostics`

All yt-dlp invocations pass `--encoding utf-8` (so fullwidth `｜` U+FF5C in filenames survives), and `resolve_downloads()` scans the output dir by `[videoId]` as a safety net for any encoding/sanitisation mismatch.
