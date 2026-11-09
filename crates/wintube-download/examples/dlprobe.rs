//! End-to-end probe for the download path: runs the same `DownloadManager`
//! the app uses and prints progress events until the job settles.
//!
//! `cargo run -p wintube-download --example dlprobe -- <video_id> <out_dir>`

use std::sync::Arc;
use std::time::Duration;

use wintube_core::{CodecPref, JobStatus, Quality};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let video_id = args
        .first()
        .cloned()
        .unwrap_or_else(|| "jNQXAC9IVRw".to_string());
    let dir = args.get(1).cloned().unwrap_or_else(|| {
        std::env::temp_dir().join("wintube-dlprobe").to_string_lossy().to_string()
    });

    let rt = tokio::runtime::Runtime::new().expect("tokio runtime");
    rt.block_on(async move {
        let mgr = wintube_download::DownloadManager::new();
        let jobs = Arc::clone(&mgr.jobs);
        let id = mgr
            .start(&video_id, "probe", Quality::Auto, CodecPref::Avc, &dir, |job| {
                println!(
                    "event {:?} {:.1}% speed={:?} eta={:?}",
                    job.status,
                    job.progress * 100.0,
                    job.speed_bps,
                    job.eta_secs
                );
            })
            .await
            .expect("start_download failed");

        loop {
            tokio::time::sleep(Duration::from_millis(500)).await;
            let done = {
                let m = jobs.lock().unwrap();
                let j = m.get(&id).expect("job vanished");
                matches!(
                    j.status,
                    JobStatus::Done | JobStatus::Failed | JobStatus::Cancelled
                )
            };
            if done {
                break;
            }
        }

        let m = jobs.lock().unwrap();
        let j = m.get(&id).expect("job vanished");
        println!(
            "FINAL status={:?} progress={:.2} output={:?} error={:?}",
            j.status, j.progress, j.output_path, j.error
        );
        if j.status != JobStatus::Done {
            std::process::exit(1);
        }
    });
}
