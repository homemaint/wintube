# License Notices

WinTube is licensed under the **MIT License** (see root `LICENSE`).

## Bundled sidecar

- **yt-dlp** (`yt-dlp.exe`) — [github.com/yt-dlp/yt-dlp](https://github.com/yt-dlp/yt-dlp), distributed under the **Unlicense**. WinTube does not modify yt-dlp; it invokes it as a sidecar subprocess for format resolution and downloading.

## Rust crates (selected)

| Crate | Used for | License |
| --- | --- | --- |
| tauri | Desktop shell | MIT/Apache-2.0 (see tauri.app) |
| muxide | MP4 muxing | MIT (see crates.io) |
| mp4 (alfg) | MP4 metadata / sample tables | MIT (see crates.io) |
| serde / serde_json | (De)serialization | MIT/Apache-2.0 |
| tokio | Async runtime | MIT |
| reqwest | HTTP (InnerTube) | MIT/Apache-2.0 |
| chrono, uuid, dirs, futures | Utilities | MIT/Apache-2.0 |

All dependencies are permissive-licensed (MIT/Apache-2.0 unless noted on crates.io). No GPL/AGPL dependencies are in the binary.

## Trademarks

"YouTube" and the YouTube logo are trademarks of Google LLC. WinTube uses the name only descriptively ("YouTube without ads") and is not affiliated with or endorsed by Google LLC or YouTube. All video content remains the property of its respective creators.
