use std::path::Path;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "TestMux/v.mp4".into());
    let tid: u32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(1);
    let samples = wintube_mux::fmp4::read_fragment_samples(Path::new(&path), tid).unwrap();
    println!("samples={}", samples.len());
    for s in samples.iter().take(10) {
        println!(
            "off={} size={} dts={} cto={} dur={} key={}",
            s.offset, s.size, s.dts, s.cto, s.duration, s.is_keyframe
        );
    }
}
