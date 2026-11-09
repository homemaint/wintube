# Contributing to WinTube

Thanks for stopping by! WinTube is a small, focused project: an ad-free, lightweight YouTube client for Windows with 144p–8K downloads, built in Rust + Tauri 2.

## Ways to contribute

1. **Report bugs** — open an issue with:
   - WinTube version (Settings → Diagnostics, or `wintube.exe` file properties)
   - Steps to reproduce, expected vs actual
   - The last ~20 lines of `%APPDATA%\Wintube\wintube.log` (contains the yt-dlp invocation + exit + stderr tail)
   - `yt-dlp` version (`yt-dlp.exe --version`)
2. **Request features** — open an issue describing the problem, not just the solution.
3. **Fix something** — PRs are welcome.

## Development setup

WinTube builds only from a **spaceless path** (e.g. `C:\Wintube`). See [docs/BUILD.md](docs/BUILD.md).

```powershell
$env:PATH = "C:\mingw64\bin;$env:PATH"
npm install
npx tauri dev     # live dev
npx tauri build   # release MSI/NSIS
```

## PR guidelines

- Keep PRs small and focused; one fix/feature per PR.
- Rust: `cargo fmt` + `cargo clippy` clean; document non-obvious logic with a short comment.
- Frontend: vanilla JS only (`src/main.js`) — no new build step unless it's clearly worth it.
- If you touch the download/remux path, verify with a real download: kittens-or-zoo video is fine, but a full-length video (to exercise fragmented M4A audio) is better.
- Update `docs/CHANGELOG.md` under **Next** with a one-line entry for user-visible changes.

## What we won't merge

- DRM circumvention of any kind.
- Anything that redistributes or hosts video content.
- Features that require a Google/YouTube API key bundled in the binary.
- Bundled yt-dlp/ffmpeg binaries — always fetch at build time (see `docs/BUILD.md#sidecars`).

## Code of Conduct

Be kind, be specific, and assume good intent. This is a personal-use tool — let's keep it friendly.

## License

By contributing, you agree your contributions are licensed under MIT (see [LICENSE](LICENSE)).
