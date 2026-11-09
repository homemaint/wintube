fn main() {
    // 0/0/0 exercises the container auto-detection path used by the
    // download manager.
    let r = wintube_mux::remux_mp4_tracks("TestMux/v.mp4", "TestMux/a.m4a", "TestMux/out.mp4", 0, 0, 0.0)
        .expect("remux failed");
    println!(
        "OK video={} ({}x{} @ {:.2}fps) audio={} bytes={}",
        r.video_samples, r.width, r.height, r.fps, r.audio_samples, r.output_bytes
    );
}
