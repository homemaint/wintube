//! Probe: can the `mp4` crate parse the `moov` of a fragmented MP4 (v.mp4)?
use std::fs::File;
use std::io::BufReader;

fn main() {
    let path = std::env::args().nth(1).unwrap_or_else(|| "TestMux/v.mp4".into());
    let f = File::open(&path).unwrap();
    let size = f.metadata().unwrap().len();
    match mp4::Mp4Reader::read_header(BufReader::new(f), size) {
        Ok(mut mp4) => {
            println!("OK header; moofs={}", mp4.moofs.len());
            for (tid, tr) in mp4.tracks() {
                println!(
                    "tid={} type={:?} ts={} samples={} w={} h={} dur={:?}",
                    tid,
                    tr.track_type().ok(),
                    tr.timescale(),
                    tr.sample_count(),
                    tr.width(),
                    tr.height(),
                    tr.duration()
                );
                if tr.trak.mdia.minf.stbl.stsd.avc1.is_some() {
                    println!("  sps={:?}", tr.sequence_parameter_set().ok());
                    println!("  pps={:?}", tr.picture_parameter_set().ok());
                }
                if tr.trak.mdia.minf.stbl.stsd.mp4a.is_some() {
                    println!("  freq_idx={:?}", tr.sample_freq_index().ok());
                    println!("  chan={:?}", tr.channel_config().ok());
                    println!("  aot={:?}", tr.audio_profile().ok());
                }
                let elst = tr.trak.edts.as_ref().and_then(|e| e.elst.as_ref());
                println!(
                    "  elst={:?}",
                    elst.as_ref().map(|e| (e.version, e.entries.len()))
                );
                if let Some(elst) = elst {
                    for en in &elst.entries {
                        println!("    entry dur={} media={} rate={}", en.segment_duration, en.media_time, en.media_rate);
                    }
                }
            }
            // Try reading sample 1 of the video track.
            let vid = *mp4.tracks().keys().next().unwrap();
            match mp4.read_sample(vid, 1) {
                Ok(Some(s)) => println!("sample1 len={} start={} dur={} roff={} sync={}", s.bytes.len(), s.start_time, s.duration, s.rendering_offset, s.is_sync),
                Ok(None) => println!("sample1 None"),
                Err(e) => println!("sample1 ERR {e:?}"),
            }
        }
        Err(e) => println!("ERR {e:?}"),
    }
}
