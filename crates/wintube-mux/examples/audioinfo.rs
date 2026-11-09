//! Prints the track layout and first samples of an MP4/M4A file.
//!
//! `cargo run -p wintube-mux --example audioinfo -- <file>`

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};

fn main() {
    let path = std::env::args().nth(1).expect("usage: audioinfo <file>");
    let mut f = File::open(&path).expect("open");
    let size = f.metadata().unwrap().len();
    println!("== {path} ({size} bytes) ==");

    // Top-level boxes: is this fragmented (moof) or flat (moov + mdat)?
    let mut pos = 0u64;
    while pos + 8 <= size {
        f.seek(SeekFrom::Start(pos)).unwrap();
        let mut h = [0u8; 8];
        if f.read_exact(&mut h).is_err() {
            break;
        }
        let mut s = u32::from_be_bytes(h[0..4].try_into().unwrap()) as u64;
        let kind = String::from_utf8_lossy(&h[4..8]).to_string();
        if s == 1 {
            let mut ext = [0u8; 8];
            f.seek(SeekFrom::Start(pos + 8)).unwrap();
            f.read_exact(&mut ext).unwrap();
            s = u64::from_be_bytes(ext);
        } else if s == 0 {
            s = size - pos;
        }
        println!("box {kind} off={pos} size={s}");
        if s < 8 {
            break;
        }
        pos += s;
    }

    let reader = File::open(&path).unwrap();
    let mut mp4 = mp4::Mp4Reader::read_header(BufReader::new(reader), size).unwrap();
    println!("moofs={}", mp4.moofs.len());
    let ids: Vec<u32> = mp4.tracks().keys().copied().collect();
    for tid in &ids {
        let tr = &mp4.tracks()[tid];
        println!(
            "tid={tid} type={:?} ts={} samples={} dur={:?}",
            tr.track_type(),
            tr.timescale(),
            tr.sample_count(),
            tr.duration()
        );
        if let Ok(fi) = tr.sample_freq_index() {
            println!("  freq_idx={fi:?} freq={}", fi.freq());
        }
        if let Ok(cc) = tr.channel_config() {
            println!("  chan={cc:?}");
        }
        if let Ok(aot) = tr.audio_profile() {
            println!("  aot={aot:?}");
        }
        println!("  mdhd={:?}", tr.trak.mdia.mdhd);
        println!("  stts entries={:?}", tr.trak.mdia.minf.stbl.stts.entries.iter().take(4).collect::<Vec<_>>());
        println!("  stts count={}", tr.trak.mdia.minf.stbl.stts.entries.len());
        if let Some(edts) = &tr.trak.edts {
            println!("  edts={:?}", edts.elst);
        }
    }

    let aid = ids[0];
    for i in 1..=3u32 {
        match mp4.read_sample(aid, i) {
            Ok(Some(s)) => println!(
                "sample{i} start={} dur={} roff={} len={} head={:02x?}",
                s.start_time,
                s.duration,
                s.rendering_offset,
                s.bytes.len(),
                &s.bytes[..s.bytes.len().min(16)]
            ),
            Ok(None) => println!("sample{i} None"),
            Err(e) => println!("sample{i} ERR {e:?}"),
        }
    }
}
