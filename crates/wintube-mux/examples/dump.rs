use std::fs::File;
use std::io::BufReader;

fn main() {
    let f = File::open("TestMux/v.mp4").unwrap();
    let size = f.metadata().unwrap().len();
    let mut mp4 = mp4::Mp4Reader::read_header(BufReader::new(f), size).unwrap();
    let tid = *mp4.tracks().keys().next().unwrap();
    let tr = mp4.tracks().get(&tid).unwrap();
    println!("ts={} samples={} tid={}", tr.timescale(), tr.sample_count(), tid);
    for i in 1..=8u32 {
        let s = mp4.read_sample(tid, i).unwrap().unwrap();
        println!("s{} start={} dur={} roff={} sync={} len={}", i, s.start_time, s.duration, s.rendering_offset, s.is_sync, s.bytes.len());
    }
}
