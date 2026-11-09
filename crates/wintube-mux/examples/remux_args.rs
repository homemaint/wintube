fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        eprintln!("usage: remux_args <video.mp4> <audio.m4a> <out.mp4>");
        std::process::exit(2);
    }
    let r = wintube_mux::remux_mp4_tracks(&args[0], &args[1], &args[2], 0, 0, 0.0)
        .expect("remux failed");
    println!(
        "OK video={} ({}x{} @ {:.2}fps) audio={} bytes={}",
        r.video_samples, r.width, r.height, r.fps, r.audio_samples, r.output_bytes
    );
}
