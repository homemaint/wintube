//! Remux a separate video-only and audio-only download into a single MP4.
//!
//! This is the pure-Rust replacement for `ffmpeg -i video.mp4 -i audio.mp4
//! -c copy out.mp4`, which is the only thing Wintube ever needed ffmpeg for.
//!
//! The pipeline is:
//!
//! 1. **Demux** the video track.  YouTube serves video renditions as
//!    fragmented MP4, so the samples come from the movie fragments via the
//!    custom [`fmp4`] parser; plain (non-fragmented) MP4 falls back to the
//!    `mp4` crate's sample tables.
//! 2. **Demux** the audio track.  It is usually a regular M4A, but when
//!    yt-dlp's `FixupM4a` postprocessor has not rewritten the original DASH
//!    stream, the file is still fragmented; the same [`fmp4`] parser then
//!    supplies the samples.  Every AAC access unit gets an ADTS header.
//! 3. **Convert** the H.264 video samples from AVCC (4-byte length prefixed)
//!    to Annex B, prefixing the first frame with the SPS/PPS from `avcC`.
//! 4. **Mux** both tracks with `muxide`, which writes a fast-start MP4.

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom, Write};
use std::path::Path;

use serde::{Deserialize, Serialize};

use muxide::api::{AacProfile, AudioCodec, MuxerBuilder, VideoCodec};

use crate::fmp4;

/// Summary of a completed remux.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RemuxInfo {
    pub video_codec: String,
    pub audio_codec: String,
    pub width: u32,
    pub height: u32,
    pub fps: f64,
    pub video_samples: usize,
    pub audio_samples: usize,
    /// `true` when the audio source was still a fragmented DASH M4A (yt-dlp's
    /// `FixupM4a` did not run) and had to be demuxed from its `moof` boxes.
    pub audio_fragmented: bool,
    pub output_bytes: u64,
}

/// Where the video payloads are read from.
enum VideoSource {
    /// Fragmented MP4: seek to the byte offset recorded for each sample.
    Fragments {
        file: File,
        samples: Vec<fmp4::FragmentSample>,
    },
    /// Plain MP4: let the `mp4` crate resolve the sample table.
    Flat {
        reader: mp4::Mp4Reader<BufReader<File>>,
        track_id: u32,
    },
}

impl VideoSource {
    fn read(&mut self, index: usize) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
        match self {
            VideoSource::Fragments { file, samples } => {
                let sample = samples[index];
                let mut buffer = vec![0u8; sample.size as usize];
                file.seek(SeekFrom::Start(sample.offset))?;
                file.read_exact(&mut buffer)?;
                Ok(buffer)
            }
            VideoSource::Flat { reader, track_id } => {
                let sample = reader
                    .read_sample(*track_id, index as u32 + 1)?
                    .ok_or_else(|| format!("video sample {} is missing", index + 1))?;
                Ok(sample.bytes.to_vec())
            }
        }
    }
}

/// One decoded AAC access unit plus its presentation time (track timescale).
struct AudioSample {
    pts: i64,
    bytes: Vec<u8>,
}

/// Remux a video-only MP4 + audio-only MP4 (H.264 + AAC) into one MP4.
///
/// `width`, `height` and `fps` are not read back from the container because
/// they are already known from the yt-dlp format description.
pub fn remux_mp4_tracks(
    video_mp4: &str,
    audio_mp4: &str,
    output_mp4: &str,
    width: u32,
    height: u32,
    fps: f64,
) -> Result<RemuxInfo, Box<dyn std::error::Error>> {
    // ---------------------------------------------------------------- video
    let video_file = File::open(video_mp4)?;
    let video_len = video_file.metadata()?.len();
    let mut video_reader = mp4::Mp4Reader::read_header(BufReader::new(video_file), video_len)?;

    // Pull the track metadata out of the reader so the immutable borrow ends
    // before we need `&mut` for reading samples.  A zero `width`/`height`/`fps`
    // means "read it from the container".
    let (video_track_id, video_timescale, sps, pps, nal_length_size, flat_count, detected) = {
        let (id, track) = video_reader
            .tracks()
            .iter()
            .find(|(_, track)| matches!(track.track_type(), Ok(mp4::TrackType::Video)))
            .ok_or("video track not found")?;
        let nal_len = track
            .trak
            .mdia
            .minf
            .stbl
            .stsd
            .avc1
            .as_ref()
            .map(|avc1| (avc1.avcc.length_size_minus_one + 1) as usize)
            .unwrap_or(4);
        (
            *id,
            track.timescale(),
            track.sequence_parameter_set()?.to_vec(),
            track.picture_parameter_set()?.to_vec(),
            nal_len,
            track.sample_count(),
            (track.width() as u32, track.height() as u32, track.frame_rate()),
        )
    };
    let (width, height, fps) = (
        if width == 0 { detected.0 } else { width },
        if height == 0 { detected.1 } else { height },
        if fps == 0.0 { detected.2 } else { fps },
    );
    if width == 0 || height == 0 || fps == 0.0 {
        return Err("video dimensions could not be determined".into());
    }
    if video_timescale == 0 {
        return Err("video track has a zero timescale".into());
    }
    if sps.is_empty() || pps.is_empty() {
        return Err("video track has no H.264 parameter sets".into());
    }

    // Movie fragments carry the authoritative timing for YouTube downloads.
    let fragments = fmp4::read_fragment_samples(Path::new(video_mp4), video_track_id)?;

    let timing: Vec<(i64, i64, bool)>;
    let mut source: VideoSource;
    if !fragments.is_empty() {
        timing = fragments
            .iter()
            .map(|sample| {
                let dts = sample.dts as i64;
                (dts, dts + sample.cto, sample.is_keyframe)
            })
            .collect();
        source = VideoSource::Fragments {
            file: File::open(video_mp4)?,
            samples: fragments,
        };
    } else {
        // A plain, non-fragmented MP4: the `mp4` crate's sample tables are
        // correct here, so use them.
        let mut entries = Vec::with_capacity(flat_count as usize);
        for index in 1..=flat_count {
            let sample = video_reader
                .read_sample(video_track_id, index)?
                .ok_or_else(|| format!("video sample {index} is missing"))?;
            let dts = sample.start_time as i64;
            entries.push((dts, dts + sample.rendering_offset as i64, sample.is_sync));
        }
        timing = entries;
        source = VideoSource::Flat {
            reader: video_reader,
            track_id: video_track_id,
        };
    }
    if timing.is_empty() {
        return Err("video track contains no samples".into());
    }

    // ---------------------------------------------------------------- audio
    let audio_file = File::open(audio_mp4)?;
    let audio_len = audio_file.metadata()?.len();
    let mut audio_reader = mp4::Mp4Reader::read_header(BufReader::new(audio_file), audio_len)?;

    let (audio_track_id, audio_timescale, sample_rate, channels, aac_profile) = {
        let (id, track) = audio_reader
            .tracks()
            .iter()
            .find(|(_, track)| matches!(track.track_type(), Ok(mp4::TrackType::Audio)))
            .ok_or("audio track not found")?;
        let freq = track.sample_freq_index()?.freq();
        let channel_config = track.channel_config()? as u16;
        let profile = match track.audio_profile()? {
            mp4::AudioObjectType::AacMain => AacProfile::Main,
            mp4::AudioObjectType::AacLowComplexity => AacProfile::Lc,
            mp4::AudioObjectType::AacScalableSampleRate => AacProfile::Ssr,
            mp4::AudioObjectType::AacLongTermPrediction => AacProfile::Ltp,
            mp4::AudioObjectType::SpectralBandReplication => AacProfile::He,
            mp4::AudioObjectType::AACScalable => AacProfile::Hev2,
            other => return Err(format!("unsupported AAC object type: {other:?}").into()),
        };
        (*id, track.timescale(), freq, channel_config, profile)
    };
    if audio_timescale == 0 {
        return Err("audio track has a zero timescale".into());
    }
    let sample_rate_index = adts_sample_rate_index(sample_rate)
        .ok_or_else(|| format!("unsupported AAC sample rate: {sample_rate}"))?;

    // A YouTube DASH audio track is fragmented until yt-dlp's `FixupM4a`
    // rewrites it.  The `mp4` crate parses that file's `moov` but its
    // fragmented-sample path returns the bytes at offset 0 for every sample,
    // so the merged audio ends up full of `ftyp`/`moov` headers instead of
    // AAC frames.  Prefer the fragment table whenever the file has one.
    let fragment_samples = fmp4::read_fragment_samples(Path::new(audio_mp4), audio_track_id)?;
    let audio_count = audio_reader.tracks()[&audio_track_id].sample_count();
    let mut audio_samples: Vec<AudioSample> = Vec::with_capacity(if fragment_samples.is_empty() {
        audio_count as usize
    } else {
        fragment_samples.len()
    });
    if !fragment_samples.is_empty() {
        let mut audio_file = File::open(audio_mp4)?;
        for sample in &fragment_samples {
            let mut bytes = vec![0u8; sample.size as usize];
            audio_file.seek(SeekFrom::Start(sample.offset))?;
            audio_file.read_exact(&mut bytes)?;
            audio_samples.push(AudioSample {
                pts: sample.dts as i64 + sample.cto,
                bytes,
            });
        }
    } else {
        for index in 1..=audio_count {
            let sample = audio_reader
                .read_sample(audio_track_id, index)?
                .ok_or_else(|| format!("audio sample {index} is missing"))?;
            let pts = sample.start_time as i64 + sample.rendering_offset as i64;
            audio_samples.push(AudioSample {
                pts,
                bytes: sample.bytes.to_vec(),
            });
        }
    }
    if audio_samples.is_empty() {
        return Err("audio track contains no samples".into());
    }

    // Some M4A files (notably ones rewritten by yt-dlp's `FixupM4a`) can end
    // up with an `stts` table whose deltas are all zero. Every timestamp then
    // collapses to 0, which makes muxide write `stts = (N, 0)` and a broken
    // audio timeline — the track is present but no decoder can play it.
    // AAC frames are 1024 samples, so rebuild a sane timeline in that case.
    let increasing = audio_samples.windows(2).all(|pair| pair[0].pts < pair[1].pts);
    if !increasing {
        let frame_units = (1024.0 * audio_timescale as f64 / sample_rate as f64)
            .round()
            .max(1.0) as i64;
        for (index, sample) in audio_samples.iter_mut().enumerate() {
            sample.pts = index as i64 * frame_units;
        }
    }
    let audio_origin = audio_samples[0].pts;

    // ------------------------------------------------------------ timestamps
    // Normalise both timelines: the first decoded frame sits at DTS 0 and the
    // earliest displayed frame sits at PTS 0.  Because the shift is the same
    // for every sample, the composition offsets keep their relative spacing
    // and the display order is preserved.
    let pts_origin = timing.iter().map(|entry| entry.1).min().unwrap_or(0);
    let dts_origin = timing[0].0;
    let video_first_pts = (timing[0].1 - pts_origin) as f64 / video_timescale as f64;
    // `muxide` refuses audio that starts before the first video frame.
    let audio_delay = video_first_pts.max(0.0);

    // ------------------------------------------------------------------ mux
    let output = File::create(output_mp4)?;
    let mut muxer = MuxerBuilder::new(output)
        .video(VideoCodec::H264, width, height, fps)
        .audio(AudioCodec::Aac(aac_profile), sample_rate, channels)
        .build()?;

    let mut video_pts_seconds: Vec<f64> = Vec::with_capacity(timing.len());
    for index in 0..timing.len() {
        let (dts, pts, is_keyframe) = timing[index];
        let dts_seconds = (dts - dts_origin) as f64 / video_timescale as f64;
        let pts_seconds = (pts - pts_origin) as f64 / video_timescale as f64;
        video_pts_seconds.push(pts_seconds);

        let payload = source.read(index)?;
        let mut annexb = avcc_to_annexb(&payload, nal_length_size);

        if index == 0 {
            // `muxide` derives the `avcC` from the first frame, so the
            // parameter sets have to travel with it.
            let mut with_params = Vec::with_capacity(annexb.len() + sps.len() + pps.len() + 8);
            with_params.extend_from_slice(&[0, 0, 0, 1]);
            with_params.extend_from_slice(&sps);
            with_params.extend_from_slice(&[0, 0, 0, 1]);
            with_params.extend_from_slice(&pps);
            with_params.extend_from_slice(&annexb);
            annexb = with_params;
        }

        muxer.write_video_with_dts(pts_seconds, dts_seconds, &annexb, is_keyframe)?;
    }

    let adts_profile = adts_profile_field(aac_profile);
    for sample in &audio_samples {
        let pts_seconds = (sample.pts - audio_origin) as f64 / audio_timescale as f64 + audio_delay;
        let frame = adts_wrap(adts_profile, sample_rate_index, channels as u8, &sample.bytes)?;
        muxer.write_audio(pts_seconds, &frame)?;
    }

    let stats = muxer.finish_with_stats()?;

    // `muxide` interleaves the `mdat` payload in *presentation* order but
    // writes the sample tables in decode order, so for B-frame streams its
    // video `stco` entries point at the wrong samples.  Remap them back.
    patch_video_chunk_offsets(output_mp4, &video_pts_seconds)?;

    Ok(RemuxInfo {
        video_codec: "h264".to_string(),
        audio_codec: "aac".to_string(),
        width,
        height,
        fps,
        video_samples: timing.len(),
        audio_samples: audio_samples.len(),
        audio_fragmented: !fragment_samples.is_empty(),
        output_bytes: stats.bytes_written,
    })
}

/// Converts an AVCC sample (length prefixed NAL units) to Annex B.
fn avcc_to_annexb(data: &[u8], nal_length_size: usize) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 32);
    let mut pos = 0usize;

    while pos + nal_length_size <= data.len() {
        let mut length = 0usize;
        for byte in &data[pos..pos + nal_length_size] {
            length = (length << 8) | *byte as usize;
        }
        pos += nal_length_size;
        if length == 0 || pos + length > data.len() {
            break;
        }
        out.extend_from_slice(&[0, 0, 0, 1]);
        out.extend_from_slice(&data[pos..pos + length]);
        pos += length;
    }

    out
}

/// The ADTS sampling-frequency index for a frequency in Hz.
fn adts_sample_rate_index(sample_rate: u32) -> Option<u8> {
    Some(match sample_rate {
        96000 => 0,
        88200 => 1,
        64000 => 2,
        48000 => 3,
        44100 => 4,
        32000 => 5,
        24000 => 6,
        22050 => 7,
        16000 => 8,
        12000 => 9,
        11025 => 10,
        8000 => 11,
        7350 => 12,
        _ => return None,
    })
}

/// The ADTS profile field (audio object type minus one).
fn adts_profile_field(profile: AacProfile) -> u8 {
    let object_type = match profile {
        AacProfile::Main => 1u8,
        AacProfile::Lc => 2,
        AacProfile::Ssr => 3,
        AacProfile::Ltp => 4,
        AacProfile::He => 5,
        AacProfile::Hev2 => 29,
    };
    // The ADTS field is only two bits wide; HE profiles signal their real
    // object type through the `esds` descriptor that `muxide` writes.
    (object_type.saturating_sub(1)) & 0x03
}

/// Wraps a raw AAC access unit in a 7 byte ADTS header.
fn adts_wrap(
    profile: u8,
    sample_rate_index: u8,
    channel_config: u8,
    payload: &[u8],
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let frame_length = payload.len() + 7;
    if frame_length > 0x1FFF {
        return Err(format!("AAC frame of {frame_length} bytes is too large").into());
    }

    let mut frame = Vec::with_capacity(frame_length);
    frame.push(0xFF);
    // syncword (1111) + MPEG-4 (0) + layer (00) + protection absent (1)
    frame.push(0xF1);
    frame.push(
        (profile << 6)
            | ((sample_rate_index & 0x0F) << 2)
            | ((channel_config >> 2) & 0x01),
    );
    frame.push(
        ((channel_config & 0x03) << 6) | (((frame_length >> 11) & 0x03) as u8),
    );
    frame.push(((frame_length >> 3) & 0xFF) as u8);
    // buffer fullness = 0x7FF (variable bit rate)
    frame.push((((frame_length & 0x07) << 5) as u8) | 0x1C);
    frame.push(0xFC);
    frame.extend_from_slice(payload);

    Ok(frame)
}

// ---------------------------------------------------------------------------
// Chunk-offset repair
// ---------------------------------------------------------------------------
//
// `muxide` stores interleaved samples in `mdat` sorted by presentation time,
// and collects one `stco` entry per sample in that order.  The remaining
// sample tables (`stsz`, `stts`, `ctts`, `stss`) are all in decode order, so
// with B-frames every entry after the first points a sample at another
// sample's bytes.  The audio track has strictly increasing PTS, hence the
// identity permutation, and is unaffected.
//
// This rewrites the video track's `stco` entries in place: the entry for
// decode-order sample `i` becomes the entry that currently sits at the
// presentation rank of `i`.  Entry width and file size do not change.

const MEDIA_TIMESCALE: u64 = 90_000;

/// Reads the box header at `offset`: (kind, size, header length).
fn read_box_header(
    file: &mut File,
    offset: u64,
) -> Result<Option<([u8; 4], u64, u64)>, Box<dyn std::error::Error>> {
    let file_len = file.metadata()?.len();
    if offset + 8 > file_len {
        return Ok(None);
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut header = [0u8; 16];
    file.read_exact(&mut header[..8])?;
    let size32 = u32::from_be_bytes(header[0..4].try_into().unwrap());
    let kind = [header[4], header[5], header[6], header[7]];
    if size32 == 1 {
        file.read_exact(&mut header[8..16])?;
        Ok(Some((kind, u64::from_be_bytes(header[8..16].try_into().unwrap()), 16)))
    } else if size32 == 0 {
        Ok(Some((kind, file_len - offset, 8)))
    } else {
        Ok(Some((kind, size32 as u64, 8)))
    }
}

/// Finds the payload range of the first child box of `kind` inside
/// `data[start..end]`, or `None`.
fn find_child(data: &[u8], start: usize, end: usize, kind: &[u8; 4]) -> Option<(usize, usize)> {
    let mut pos = start;
    while pos + 8 <= end {
        let size32 = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap());
        let box_kind = [data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]];
        let (size, header_len) = if size32 == 1 {
            if pos + 16 > end {
                break;
            }
            (
                u64::from_be_bytes(data[pos + 8..pos + 16].try_into().unwrap()) as usize,
                16usize,
            )
        } else if size32 == 0 {
            (end - pos, 8usize)
        } else {
            (size32 as usize, 8usize)
        };
        if size < header_len {
            break;
        }
        let payload_end = (pos + size).min(end);
        if &box_kind == kind {
            return Some((pos + header_len, payload_end));
        }
        pos += size;
    }
    None
}

fn patch_video_chunk_offsets(
    output_mp4: &str,
    video_pts_seconds: &[f64],
) -> Result<(), Box<dyn std::error::Error>> {
    if video_pts_seconds.is_empty() {
        return Ok(());
    }

    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(output_mp4)?;

    // Locate the `moov` box and load it; it is small compared with `mdat`.
    let mut moov: Option<(u64, u64)> = None;
    let mut cursor = 0u64;
    while let Some((kind, size, _header)) = read_box_header(&mut file, cursor)? {
        if size < 8 {
            break;
        }
        if &kind == b"moov" {
            moov = Some((cursor, size));
            break;
        }
        cursor += size;
    }
    let (moov_offset, moov_size) = moov.ok_or("output has no moov box")?;

    let mut data = vec![0u8; moov_size as usize];
    file.seek(SeekFrom::Start(moov_offset))?;
    file.read_exact(&mut data)?;

    // moov -> trak (first: the video track) -> mdia -> minf -> stbl -> stco
    let trak = find_child(&data, 8, data.len(), b"trak").ok_or("moov has no trak")?;
    let mdia = find_child(&data, trak.0, trak.1, b"mdia").ok_or("trak has no mdia")?;
    let minf = find_child(&data, mdia.0, mdia.1, b"minf").ok_or("mdia has no minf")?;
    let stbl = find_child(&data, minf.0, minf.1, b"stbl").ok_or("minf has no stbl")?;
    let (stco_start, stco_end, entry_size) =
        match find_child(&data, stbl.0, stbl.1, b"stco") {
            Some(range) => (range.0, range.1, 4usize),
            None => match find_child(&data, stbl.0, stbl.1, b"co64") {
                Some(range) => (range.0, range.1, 8usize),
                None => return Err("video track has no chunk offset table".into()),
            },
        };
    if stco_end - stco_start < 8 {
        return Err("chunk offset table is truncated".into());
    }
    let entry_count =
        u32::from_be_bytes(data[stco_start + 4..stco_start + 8].try_into().unwrap()) as usize;
    if entry_size == 4 {
        let _ = &data[stco_start..stco_end];
    }
    if entry_count != video_pts_seconds.len() {
        // `muxide` writes one chunk per sample when audio is present; if that
        // ever changes there is nothing safe to permute.
        return Err(format!(
            "chunk offset count {entry_count} does not match video sample count {}",
            video_pts_seconds.len()
        )
        .into());
    }

    let read_entry = |index: usize| -> u64 {
        let at = stco_start + 8 + index * entry_size;
        if entry_size == 4 {
            u32::from_be_bytes(data[at..at + 4].try_into().unwrap()) as u64
        } else {
            u64::from_be_bytes(data[at..at + 8].try_into().unwrap())
        }
    };
    let written: Vec<u64> = (0..entry_count).map(read_entry).collect();

    // Presentation rank of each decode-order sample, mirroring `muxide`'s
    // interleave sort: (scaled pts, video-before-audio, sample index).
    let mut order: Vec<usize> = (0..video_pts_seconds.len()).collect();
    order.sort_by_key(|&index| {
        ((video_pts_seconds[index] * MEDIA_TIMESCALE as f64).round() as u64, index)
    });
    let mut rank = vec![0usize; video_pts_seconds.len()];
    for (position, &index) in order.iter().enumerate() {
        rank[index] = position;
    }

    for index in 0..entry_count {
        let corrected = written[rank[index]];
        if corrected == written[index] {
            continue;
        }
        let at = moov_offset + (stco_start + 8 + index * entry_size) as u64;
        file.seek(SeekFrom::Start(at))?;
        if entry_size == 4 {
            file.write_all(&(corrected as u32).to_be_bytes())?;
        } else {
            file.write_all(&corrected.to_be_bytes())?;
        }
    }

    Ok(())
}
