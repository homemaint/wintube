# Installation

## Requirements

- Windows 10 or Windows 11, 64-bit (x64)
- WebView2 Evergreen Runtime — pre-installed on most Windows 10/11 systems. If the app opens as a blank/white window, install it from Microsoft: https://developer.microsoft.com/microsoft-edge/webview2/

## MSI vs NSIS

| | MSI `Wintube_x64_en-US.msi` | NSIS `Wintube_x64-setup.exe` |
| --- | --- | --- |
| Size | ~25 MB | ~24 MB |
| Install location | `C:\Program Files\Wintube` (UAC) | Per-user (no UAC for exe files, still needs elevation for Program Files if you choose it) |
| Upgrade | Replaces previous version, upgrade prompt | Per-user overwrite |
| Recommended | Yes — standard enterprise install | Yes — fastest per-user install |

Pick MSI unless you need a per-user install without admin rights.

## Install

1. Download the latest release from https://github.com/homemaint/wintube/releases
2. Double-click the `.msi` (or `-setup.exe`) and accept the UAC prompt.
3. Launch **WinTube** from Start → WinTube, or from `C:\Program Files\Wintube\wintube.exe`.

What gets installed:

- `Wintube.exe` — the Tauri app
- `WebView2Loader.dll` — **must sit next to the EXE** (missing it yields a windowless process)
- `yt-dlp.exe` — the download sidecar (v2026.10.26+)

## Where files go

| Thing | Path |
| --- | --- |
| App | `C:\Program Files\Wintube\` |
| Settings + history (JSON) | `%APPDATA%\Wintube\` |
| Runtime/download log | `%APPDATA%\Wintube\wintube.log` |
| Downloads (default) | `%USERPROFILE%\Downloads\Wintube\` |
| Downloads (custom) | Settings → Download folder |

Change the download folder in **Settings → Download folder**.

## Uninstall

Settings → Apps → Wintube → Uninstall. Your downloads and settings in `%APPDATA%\Wintube` are kept (delete manually if you want a clean remove).

## Update

Download the newer MSI/NSIS from Releases and run it — it upgrades in place. Version numbers are SemVer (`0.1.4` → `0.1.5` → `0.2.0`).

## Fetching yt-dlp manually

The installer already bundles yt-dlp. To refresh it:

```powershell
Invoke-WebRequest https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe -OutFile "C:\Program Files\Wintube\yt-dlp.exe"
```

Or use the **Settings → Check yt-dlp setup** diagnostics to confirm the sidecar path and version.
