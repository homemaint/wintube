//! Minimal fragmented-MP4 (fMP4) demuxer.
//!
//! YouTube's DASH video renditions are served as *fragmented* MP4: the media
//! payload is split into a sequence of `moof` (movie fragment) / `mdat`
//! (media data) pairs, and every sample's size, duration and timing lives in
//! the fragment boxes (`tfhd` / `tfdt` / `trun`) instead of the classic sample
//! tables (`stts` / `ctts` / `stsz` / `stsc` / `stco`).
//!
//! The `mp4` crate parses the `moov` of such a file correctly, but its
//! fragmented *sample* path is not usable: it derives byte offsets from the
//! wrong base, assumes a flat `default_sample_duration` for every sample,
//! loses the `trun` composition offsets and guesses the sync flag from the
//! sample count.  This module implements just enough of ISO/IEC 14496-12 to
//! rebuild the sample table exactly the way ffmpeg does:
//!
//! * `tfhd` – track id plus the fragment defaults (duration / size / flags)
//!   and the base-data-offset policy.
//! * `tfdt` – the fragment's base media decode time.
//! * `trun` – the actual per-sample run: byte sizes, durations, sample flags
//!   and signed composition-time offsets.
//!
//! Sample payloads are located from `trun.data_offset`, which is relative to
//! the base data offset (`moof` start when `default-base-is-moof` is set).

use std::collections::HashMap;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use thiserror::Error;

/// Errors produced while parsing a fragmented MP4.
#[derive(Debug, Error)]
pub enum Fmp4Error {
    #[error("i/o error: {0}")]
    Io(#[from] std::io::Error),
    #[error("malformed fragmented mp4: {0}")]
    Malformed(String),
}

/// Convenience result alias.
pub type Result<T> = std::result::Result<T, Fmp4Error>;

/// A single media sample recovered from a movie fragment.
#[derive(Debug, Clone, Copy)]
pub struct FragmentSample {
    /// Absolute byte offset of the sample payload inside the file.
    pub offset: u64,
    /// Payload size in bytes.
    pub size: u32,
    /// Decode time, in track timescale units.
    pub dts: u64,
    /// Composition time offset (`pts - dts`), in track timescale units.
    pub cto: i64,
    /// Sample duration, in track timescale units.
    pub duration: u32,
    /// `true` when the sample is a sync sample (random access point).
    pub is_keyframe: bool,
}

// ---------------------------------------------------------------------------
// Byte reader
// ---------------------------------------------------------------------------

struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(data: &'a [u8]) -> Self {
        Reader { data, pos: 0 }
    }

    fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if self.remaining() < n {
            return Err(Fmp4Error::Malformed(format!(
                "truncated box: wanted {n} bytes, {} left",
                self.remaining()
            )));
        }
        let slice = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(slice)
    }

    fn u8(&mut self) -> Result<u8> {
        Ok(self.take(1)?[0])
    }

    fn u24(&mut self) -> Result<u32> {
        let b = self.take(3)?;
        Ok(((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32)
    }

    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }

    fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }

    /// Reads a `FullBox` header: 1 version byte + 24 flag bits.
    fn full_box(&mut self) -> Result<(u8, u32)> {
        let version = self.u8()?;
        let flags = self.u24()?;
        Ok((version, flags))
    }
}

// ---------------------------------------------------------------------------
// Box walking
// ---------------------------------------------------------------------------

struct Box<'a> {
    kind: [u8; 4],
    payload: &'a [u8],
    /// Absolute offset of the box (header included) within the file.
    start: u64,
}

/// Splits a buffer into its child boxes.
///
/// `base` is the absolute file offset of `data[0]`, used only for bookkeeping.
fn boxes<'a>(data: &'a [u8], base: u64) -> Result<Vec<Box<'a>>> {
    let mut out = Vec::new();
    let mut pos = 0usize;

    while pos + 8 <= data.len() {
        let size32 = u32::from_be_bytes(data[pos..pos + 4].try_into().unwrap());
        let kind = [data[pos + 4], data[pos + 5], data[pos + 6], data[pos + 7]];

        let (size, header_len) = if size32 == 1 {
            if pos + 16 > data.len() {
                break;
            }
            (
                u64::from_be_bytes(data[pos + 8..pos + 16].try_into().unwrap()),
                16usize,
            )
        } else if size32 == 0 {
            // Box extends to the end of the enclosing buffer.
            ((data.len() - pos) as u64, 8usize)
        } else {
            (size32 as u64, 8usize)
        };

        if (size as usize) < header_len {
            break;
        }
        let end = pos.saturating_add(size as usize).min(data.len());
        out.push(Box {
            kind,
            payload: &data[pos + header_len..end],
            start: base + pos as u64,
        });
        if end <= pos {
            break;
        }
        pos = end;
    }

    Ok(out)
}

// ---------------------------------------------------------------------------
// Fragment boxes
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct Tfhd {
    track_id: u32,
    base_data_offset: Option<u64>,
    default_sample_duration: Option<u32>,
    default_sample_size: Option<u32>,
    default_sample_flags: Option<u32>,
    default_base_is_moof: bool,
}

fn parse_tfhd(payload: &[u8]) -> Result<Tfhd> {
    let mut r = Reader::new(payload);
    let (_version, flags) = r.full_box()?;

    let track_id = r.u32()?;
    let base_data_offset = if flags & 0x0000_0001 != 0 {
        Some(r.u64()?)
    } else {
        None
    };
    if flags & 0x0000_0002 != 0 {
        // sample_description_index – not needed, the `moov` already has it.
        let _ = r.u32()?;
    }
    let default_sample_duration = if flags & 0x0000_0008 != 0 {
        Some(r.u32()?)
    } else {
        None
    };
    let default_sample_size = if flags & 0x0000_0010 != 0 {
        Some(r.u32()?)
    } else {
        None
    };
    let default_sample_flags = if flags & 0x0000_0020 != 0 {
        Some(r.u32()?)
    } else {
        None
    };

    Ok(Tfhd {
        track_id,
        base_data_offset,
        default_sample_duration,
        default_sample_size,
        default_sample_flags,
        default_base_is_moof: flags & 0x0002_0000 != 0,
    })
}

fn parse_tfdt(payload: &[u8]) -> Result<u64> {
    let mut r = Reader::new(payload);
    let (version, _flags) = r.full_box()?;
    if version == 0 {
        Ok(r.u32()? as u64)
    } else {
        r.u64()
    }
}

#[derive(Debug, Default)]
struct Trun {
    sample_count: u32,
    data_offset: Option<i32>,
    durations: Vec<u32>,
    sizes: Vec<u32>,
    flags: Vec<u32>,
    ctos: Vec<i64>,
}

fn parse_trun(payload: &[u8]) -> Result<Trun> {
    let mut r = Reader::new(payload);
    let (version, flags) = r.full_box()?;

    let sample_count = r.u32()?;
    let data_offset = if flags & 0x0000_0001 != 0 {
        Some(r.i32()?)
    } else {
        None
    };
    let first_sample_flags = if flags & 0x0000_0004 != 0 {
        Some(r.u32()?)
    } else {
        None
    };

    let has_duration = flags & 0x0000_0100 != 0;
    let has_size = flags & 0x0000_0200 != 0;
    let has_flags = flags & 0x0000_0400 != 0;
    let has_cto = flags & 0x0000_0800 != 0;

    let count = sample_count as usize;
    let mut trun = Trun {
        sample_count,
        data_offset,
        durations: Vec::with_capacity(count),
        sizes: Vec::with_capacity(count),
        flags: Vec::with_capacity(count),
        ctos: Vec::with_capacity(count),
    };

    for index in 0..count {
        if has_duration {
            trun.durations.push(r.u32()?);
        }
        if has_size {
            trun.sizes.push(r.u32()?);
        }
        if has_flags {
            // When `first-sample-flags-present` is set the per-sample flags
            // field is *absent* for the first sample of the run.
            if index == 0 && first_sample_flags.is_some() {
                trun.flags.push(first_sample_flags.unwrap());
            } else {
                trun.flags.push(r.u32()?);
            }
        } else if index == 0 {
            if let Some(value) = first_sample_flags {
                trun.flags.push(value);
            }
        }
        if has_cto {
            // Version 1 stores signed offsets, version 0 unsigned ones.
            if version == 1 {
                trun.ctos.push(r.i32()? as i64);
            } else {
                trun.ctos.push(r.u32()? as i64);
            }
        }
    }

    Ok(trun)
}

#[derive(Debug, Clone, Copy, Default)]
struct Trex {
    default_sample_duration: u32,
    default_sample_size: u32,
    default_sample_flags: u32,
}

/// Collects the `moov/mvex/trex` defaults, which act as a fallback when a
/// `tfhd` does not carry them.
fn collect_trex(payload: &[u8], base: u64, out: &mut HashMap<u32, Trex>) -> Result<()> {
    for b in boxes(payload, base)? {
        if &b.kind != b"mvex" {
            continue;
        }
        for mvex_child in boxes(b.payload, b.start)? {
            if &mvex_child.kind != b"trex" {
                continue;
            }
            let mut r = Reader::new(mvex_child.payload);
            let (_version, _flags) = r.full_box()?;
            let track_id = r.u32()?;
            let _default_sample_description_index = r.u32()?;
            let default_sample_duration = r.u32()?;
            let default_sample_size = r.u32()?;
            let default_sample_flags = r.u32()?;
            out.insert(
                track_id,
                Trex {
                    default_sample_duration,
                    default_sample_size,
                    default_sample_flags,
                },
            );
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Fragment walking
// ---------------------------------------------------------------------------

fn parse_moof(
    payload: &[u8],
    moof_start: u64,
    track_id: u32,
    trex: Option<Trex>,
    implicit_offset: u64,
    out: &mut Vec<FragmentSample>,
) -> Result<()> {
    for b in boxes(payload, moof_start)? {
        if &b.kind != b"traf" {
            continue;
        }
        parse_traf(b.payload, moof_start, track_id, trex, implicit_offset, out)?;
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn parse_traf(
    payload: &[u8],
    moof_start: u64,
    track_id: u32,
    trex: Option<Trex>,
    implicit_offset: u64,
    out: &mut Vec<FragmentSample>,
) -> Result<()> {
    let mut tfhd: Option<Tfhd> = None;
    let mut base_dts: u64 = 0;
    let mut truns: Vec<Trun> = Vec::new();

    for b in boxes(payload, moof_start)? {
        match &b.kind {
            b"tfhd" => tfhd = Some(parse_tfhd(b.payload)?),
            b"tfdt" => base_dts = parse_tfdt(b.payload)?,
            b"trun" => truns.push(parse_trun(b.payload)?),
            _ => {}
        }
    }

    let tfhd = match tfhd {
        Some(tfhd) => tfhd,
        None => return Ok(()),
    };
    if tfhd.track_id != track_id {
        return Ok(());
    }

    let default_duration = tfhd
        .default_sample_duration
        .or_else(|| trex.map(|t| t.default_sample_duration))
        .unwrap_or(0);
    let default_size = tfhd
        .default_sample_size
        .or_else(|| trex.map(|t| t.default_sample_size))
        .unwrap_or(0);
    let default_flags = tfhd
        .default_sample_flags
        .or_else(|| trex.map(|t| t.default_sample_flags))
        .unwrap_or(0);

    // The base data offset anchors `trun.data_offset`:
    //   * explicit `base_data_offset` wins,
    //   * otherwise `default-base-is-moof` anchors at the `moof` box,
    //   * otherwise it is the end of the previous fragment's data.
    let base = match tfhd.base_data_offset {
        Some(value) => value,
        None if tfhd.default_base_is_moof => moof_start,
        None => implicit_offset,
    };

    let mut dts = base_dts;
    let mut cursor: Option<u64> = None;

    for trun in truns {
        let run_start = match trun.data_offset {
            Some(offset) => (base as i64 + offset as i64) as u64,
            // Without an explicit offset the run continues where the previous
            // one ended.
            None => cursor.unwrap_or(base),
        };
        let mut offset = run_start;

        for index in 0..trun.sample_count as usize {
            let duration = trun.durations.get(index).copied().unwrap_or(default_duration);
            let size = trun.sizes.get(index).copied().unwrap_or(default_size);
            let flags = trun.flags.get(index).copied().unwrap_or(default_flags);
            let cto = trun.ctos.get(index).copied().unwrap_or(0);

            out.push(FragmentSample {
                offset,
                size,
                dts,
                cto,
                duration,
                // `sample_is_non_sync_sample` is bit 16 of the 32 bit sample
                // flags field; a clear bit means the sample is a sync sample.
                is_keyframe: (flags >> 16 & 1) == 0,
            });

            offset += size as u64;
            dts += duration as u64;
        }
        cursor = Some(offset);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Reads every sample of `track_id` from the movie fragments of `path`.
///
/// The returned samples are in **decode order** with their raw decode times,
/// composition offsets and keyframe flags, all expressed in the track's
/// timescale.  An empty vector means the file contains no fragments for that
/// track (i.e. it is a plain, non-fragmented MP4).
pub fn read_fragment_samples(path: &Path, track_id: u32) -> Result<Vec<FragmentSample>> {
    let mut file = File::open(path)?;
    let file_len = file.metadata()?.len();

    let mut trex: HashMap<u32, Trex> = HashMap::new();
    let mut samples: Vec<FragmentSample> = Vec::new();
    let mut implicit_offset: u64 = 0;
    let mut position: u64 = 0;

    while position + 8 <= file_len {
        file.seek(SeekFrom::Start(position))?;

        let mut header = [0u8; 16];
        let read = read_up_to(&mut file, &mut header)?;
        if read < 8 {
            break;
        }

        let size32 = u32::from_be_bytes(header[0..4].try_into().unwrap());
        let kind = [header[4], header[5], header[6], header[7]];
        let (size, header_len) = if size32 == 1 {
            if read < 16 {
                break;
            }
            (
                u64::from_be_bytes(header[8..16].try_into().unwrap()),
                16u64,
            )
        } else if size32 == 0 {
            (file_len - position, 8u64)
        } else {
            (size32 as u64, 8u64)
        };
        if size < header_len {
            break;
        }

        let payload_len = (size - header_len) as usize;
        // The header probe may have read more bytes than the box header
        // occupies, so rewind to the start of the payload.
        file.seek(SeekFrom::Start(position + header_len))?;
        match &kind {
            b"moov" => {
                let mut buffer = vec![0u8; payload_len];
                file.read_exact(&mut buffer)?;
                collect_trex(&buffer, position + header_len, &mut trex)?;
            }
            b"moof" => {
                let mut buffer = vec![0u8; payload_len];
                file.read_exact(&mut buffer)?;
                parse_moof(
                    &buffer,
                    position,
                    track_id,
                    trex.get(&track_id).copied(),
                    implicit_offset,
                    &mut samples,
                )?;
            }
            b"mdat" => {
                // The next fragment's implicit base is the end of this data.
                implicit_offset = position + size;
            }
            _ => {}
        }

        position = position.saturating_add(size);
    }

    Ok(samples)
}

fn read_up_to(file: &mut File, buffer: &mut [u8]) -> Result<usize> {
    let mut total = 0usize;
    while total < buffer.len() {
        match file.read(&mut buffer[total..]) {
            Ok(0) => break,
            Ok(n) => total += n,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err.into()),
        }
    }
    Ok(total)
}
