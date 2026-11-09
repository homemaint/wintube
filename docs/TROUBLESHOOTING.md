# Troubleshooting

## Blank / white window on launch

The app starts in a windowless process or a white box usually means **`WebView2Loader.dll` is missing next to `wintube.exe`**, or the WebView2 Runtime is absent.

1. Confirm `WebView2Loader.dll` sits in `C:\Program Files\Wintube\` next to `wintube.exe`.
2. Reinstall the WebView2 Evergreen Runtime: https://developer.microsoft.com/microsoft-edge/webview2/
3. Launch again. If it still fails, check `%APPDATA%\Wintube\wintube.log` for the last error.

## "HTTP 429: Too Many Requests" / "Sign in to confirm you're not a bot"

YouTube temporarily blocks extraction from your IP. This is upstream rate-limiting, not a WinTube bug.

1. Wait 10–30 minutes and retry.
2. Use **Settings → Check yt-dlp setup** — if the probe also fails with 429/bot, it's definitely the IP.
3. VPN or a different network often clears it immediately.
4. Update yt-dlp: re-fetch `yt-dlp.exe` (see [INSTALL.md](INSTALL.md#fetching-yt-dlp-manually)).

## Downloads stuck at one percentage (e.g. 5%) but the file is on disk

Fixed in **0.1.4**. The Downloads page used to re-fetch the job list on every progress tick, and a late stale response could overwrite the finished job. Upgrade to 0.1.4+. If you are on 0.1.4 and still see it, the job is in **Downloads → check the folder** — the file is written even if the page didn't paint.

## Merged MP4 has video but no audible audio (silent)

Fixed in **0.1.4**. Cause: the audio source was still a fragmented DASH M4A (`FixupM4a` didn't run), and the old remuxer read the file header (`ftyp`/`moov`) as every audio "sample". The remuxer now demuxes fragmented audio with its own `fmp4` parser.

If you still hit this on 0.1.4:

```powershell
ffprobe -v error -show_entries "stream=codec_type,codec_name" -of json "YOUR_FILE.mp4"
ffmpeg -v error -i "YOUR_FILE.mp4" -map 0:a:0 -t 10 -f null -
```

If the second command errors with `Invalid data`, re-download — the old file needs replacing. A fresh download on 0.1.4+ is correct.

## File named with `｜` instead of `|` (fullwidth pipe)

YouTube/youtube-dlp maps `|` (illegal on Windows) to the fullwidth `｜` (U+FF5C). That is expected. Filenames like `Title ｜ Female Version ｜ ... [id] 720p.merged.mp4` are correct. Early 0.1.1 builds dropped the `｜` on stdout and reported a nonexistent path — fixed with `--encoding utf-8` + directory scan.

## Download failed: "yt-dlp did not report a video/audio file"

1. Check `%APPDATA%\Wintube\wintube.log` — look for the last `run` + `exit` + `stderr_tail` lines.
2. The failure is almost always upstream (429, bot block, format unavailable). Wait and retry.
3. Try **Auto** quality/codec instead of a fixed 8K/AV1/VP9 — not all videos have every rendition.

## AV1 / VP9 download fails or needs ffmpeg

The pure-Rust remuxer only handles **H.264 + AAC**. For AV1/VP9, WinTube falls back to yt-dlp's ffmpeg merge, and **ffmpeg is not bundled**. Either:

- Pick **Auto / AVC** (no ffmpeg needed), or
- Install ffmpeg and put it on PATH (or next to `wintube.exe`).

This is on the roadmap to remove (AV1/VP9 remux in Rust).

## Progress shows wrong speed/ETA

Known limitation: progress parsing reads yt-dlp's `--newline` output. If a line lacks ` at …/s` or ` ETA …`, speed/ETA are blank (video still downloads). The percentage bar and Merging/Done states are still accurate.

## Build fails with `windres` / path errors

Your source path contains spaces. Build from a spaceless mirror (`C:\Wintube`). See [BUILD.md](BUILD.md#important-spaceless-path).

## "Resolution could not be found" / "job not found"

The Downloads page tried to refresh a job that had been removed. Fixed in 0.1.4 by keeping the locally-merged latest entry. Restart the app if you see it on older versions.
