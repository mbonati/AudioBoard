//! Audio files a module holds (Splitter's sample): decoding WAV and AIFF,
//! keeping the sound with the project, and handing it to the audio thread.
//!
//! A [`Sample`] is decoded once on the UI thread, at most [`MAX_SECONDS`]
//! long and at most stereo, and rounded to 16 bits there, so what plays is
//! exactly what the project keeps: the PCM, base64, inside the module's
//! entry ([`Saved`]). A project therefore carries its sounds with it, like
//! the board carries its stickers' images.
//!
//! [`SampleSlot`] passes the sample from the UI to the processor without
//! blocking the audio thread: the UI swaps it in under a lock and bumps a
//! version; the processor, seeing a new version, takes it with `try_lock`
//! and hands the one it let go back to the slot, so it is freed on the UI
//! thread rather than in the audio callback.

use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The longest sound kept (longer files are cut there).
pub const MAX_SECONDS: f32 = 30.0;
/// Columns of the waveform overview kept with a sample.
pub const OVERVIEW: usize = 2048;

/// A decoded sound: one or two channels of 16-bit-exact samples.
pub struct Sample {
    /// The file's name, for the panel.
    pub name: String,
    /// The file's sample rate.
    pub rate: f32,
    pub left: Vec<f32>,
    /// Empty for a mono sound.
    pub right: Vec<f32>,
    /// Per overview column, the lowest and highest sample.
    pub overview: Vec<[f32; 2]>,
    /// Whether the file was cut at [`MAX_SECONDS`].
    pub trimmed: bool,
    saved: OnceLock<Saved>,
}

impl Sample {
    /// A sound from planar samples (−1…1), rounded to 16 bits. `right`
    /// empty: mono.
    pub fn new(name: &str, rate: f32, left: Vec<f32>, right: Vec<f32>) -> Self {
        let max = (MAX_SECONDS * rate) as usize;
        let trimmed = left.len() > max;
        let round = |v: Vec<f32>| -> Vec<f32> { v.into_iter().take(max).map(|x| f32::from(to_i16(x)) / 32768.0).collect() };
        let left = round(left);
        let right = if right.is_empty() { Vec::new() } else { round(right) };
        let overview = overview(&left, &right);
        Sample { name: name.to_owned(), rate: rate.max(1.0), left, right, overview, trimmed, saved: OnceLock::new() }
    }

    pub fn frames(&self) -> usize {
        self.left.len()
    }

    pub fn channels(&self) -> usize {
        if self.right.is_empty() { 1 } else { 2 }
    }

    pub fn seconds(&self) -> f32 {
        self.frames() as f32 / self.rate
    }

    /// The right channel (the left one for a mono sound).
    pub fn right(&self) -> &[f32] {
        if self.right.is_empty() { &self.left } else { &self.right }
    }

    /// Reads a file, WAV or AIFF.
    pub fn load(path: &Path) -> Result<Sample, String> {
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        decode(&bytes, &name)
    }

    /// How the project keeps it (encoded once, then shared).
    pub fn saved(&self) -> Saved {
        self.saved
            .get_or_init(|| {
                let channels = self.channels();
                let mut pcm = Vec::with_capacity(self.frames() * channels * 2);
                for (&l, &r) in self.left.iter().zip(self.right()) {
                    pcm.extend_from_slice(&to_i16(l).to_le_bytes());
                    if channels == 2 {
                        pcm.extend_from_slice(&to_i16(r).to_le_bytes());
                    }
                }
                Saved { name: self.name.clone(), rate: self.rate.round() as u32, channels: channels as u8, pcm: Blob(base64_encode(&pcm).into()) }
            })
            .clone()
    }

    /// The sound a project kept, if it reads.
    pub fn from_saved(s: &Saved) -> Option<Sample> {
        let bytes = base64_decode(&s.pcm.0)?;
        let channels = (s.channels as usize).clamp(1, 2);
        let frames = bytes.len() / (2 * channels);
        let at = |i: usize| f32::from(i16::from_le_bytes([bytes[2 * i], bytes[2 * i + 1]])) / 32768.0;
        let left: Vec<f32> = (0..frames).map(|f| at(f * channels)).collect();
        let right: Vec<f32> = if channels == 2 { (0..frames).map(|f| at(f * 2 + 1)).collect() } else { Vec::new() };
        let sample = Sample::new(&s.name, s.rate.max(1) as f32, left, right);
        // Encoding it again would give the same text: keep the one read.
        let _ = sample.saved.set(s.clone());
        Some(sample)
    }
}

fn to_i16(x: f32) -> i16 {
    (x.clamp(-1.0, 1.0) * 32768.0).round().clamp(-32768.0, 32767.0) as i16
}

fn overview(left: &[f32], right: &[f32]) -> Vec<[f32; 2]> {
    let n = left.len();
    if n == 0 {
        return Vec::new();
    }
    let right = if right.is_empty() { left } else { right };
    (0..OVERVIEW)
        .map(|c| {
            let (a, b) = (c * n / OVERVIEW, ((c + 1) * n / OVERVIEW).max(c * n / OVERVIEW + 1).min(n));
            let mut lo = f32::MAX;
            let mut hi = f32::MIN;
            for i in a..b {
                let y = (left[i] + right[i]) * 0.5;
                lo = lo.min(y);
                hi = hi.max(y);
            }
            [lo, hi]
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Keeping it in the project
// ---------------------------------------------------------------------------

/// A sample as the project file holds it: 16-bit little-endian PCM,
/// interleaved, base64.
#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
pub struct Saved {
    pub name: String,
    pub rate: u32,
    pub channels: u8,
    pub pcm: Blob,
}

/// Text shared rather than copied: projects are compared several times a
/// second to tell unsaved changes, and a sample's text can run to
/// megabytes. Equal texts from the same encoding compare by pointer.
#[derive(Clone, Debug, PartialEq)]
pub struct Blob(pub Arc<str>);

impl Serialize for Blob {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for Blob {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(Blob(String::deserialize(d)?.into()))
    }
}

const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn base64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for k in 0..4 {
            out.push(if k <= chunk.len() { BASE64[(n >> (18 - 6 * k) & 63) as usize] as char } else { '=' });
        }
    }
    out
}

/// Decodes with a table (a sample's text is long).
fn base64_decode(text: &str) -> Option<Vec<u8>> {
    let mut table = [255u8; 256];
    for (i, &c) in BASE64.iter().enumerate() {
        table[c as usize] = i as u8;
    }
    let mut out = Vec::with_capacity(text.len() / 4 * 3);
    let (mut acc, mut bits) = (0u32, 0);
    for c in text.bytes().filter(|c| !c.is_ascii_whitespace() && *c != b'=') {
        let v = table[c as usize];
        if v == 255 {
            return None;
        }
        acc = acc << 6 | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
            acc &= (1 << bits) - 1;
        }
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Decoding
// ---------------------------------------------------------------------------

/// A WAV or AIFF file's sound: PCM of 8 to 32 bits or floats, the first two
/// channels of more.
pub fn decode(bytes: &[u8], name: &str) -> Result<Sample, String> {
    match bytes.get(0..4) {
        Some(b"RIFF") | Some(b"RF64") => decode_wav(bytes, name),
        Some(b"FORM") => decode_aiff(bytes, name),
        _ => Err("not a WAV or AIFF file".to_owned()),
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Encoding {
    /// Signed integers (unsigned for 8-bit WAV), little- or big-endian.
    Int { bits: u16, little: bool, unsigned8: bool },
    Float { bits: u16, little: bool },
}

/// Planar channels (at most two) from interleaved frames.
fn deinterleave(data: &[u8], channels: usize, enc: Encoding) -> Result<(Vec<f32>, Vec<f32>), String> {
    let width = match enc {
        Encoding::Int { bits, .. } => (bits as usize).div_ceil(8),
        Encoding::Float { bits, .. } => bits as usize / 8,
    };
    if !(1..=8).contains(&width) || channels == 0 {
        return Err("unsupported sample format".to_owned());
    }
    let frame = width * channels;
    let frames = data.len() / frame;
    let read = |at: usize| -> f32 {
        let b = &data[at..at + width];
        match enc {
            Encoding::Int { little, unsigned8, .. } => {
                if width == 1 {
                    return if unsigned8 { (b[0] as f32 - 128.0) / 128.0 } else { b[0] as i8 as f32 / 128.0 };
                }
                let mut v: i64 = 0;
                for k in 0..width {
                    let byte = if little { b[width - 1 - k] } else { b[k] };
                    v = (v << 8) | byte as i64;
                }
                // Sign-extend from the container width, then scale by the
                // container (padding bits are low and zero).
                let shift = 64 - 8 * width as u32;
                let v = (v << shift) >> shift;
                v as f32 / (1u64 << (8 * width - 1)) as f32
            }
            Encoding::Float { bits, little } => {
                if bits == 32 {
                    let a = [b[0], b[1], b[2], b[3]];
                    if little { f32::from_le_bytes(a) } else { f32::from_be_bytes(a) }
                } else {
                    let a = [b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]];
                    (if little { f64::from_le_bytes(a) } else { f64::from_be_bytes(a) }) as f32
                }
            }
        }
    };
    let clean = |x: f32| if x.is_finite() { x.clamp(-1.0, 1.0) } else { 0.0 };
    let left: Vec<f32> = (0..frames).map(|f| clean(read(f * frame))).collect();
    let right: Vec<f32> = if channels >= 2 { (0..frames).map(|f| clean(read(f * frame + width))).collect() } else { Vec::new() };
    Ok((left, right))
}

fn u16_le(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_le(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u16_be(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_be_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_be(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_be_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn decode_wav(b: &[u8], name: &str) -> Result<Sample, String> {
    if b.get(8..12) != Some(b"WAVE") {
        return Err("not a WAV file".to_owned());
    }
    let mut at = 12;
    let mut format = None;
    let mut data = None;
    while at + 8 <= b.len() {
        let id = &b[at..at + 4];
        let len = u32_le(b, at + 4).unwrap_or(0) as usize;
        let body = at + 8;
        // RF64 and streamed files may say 0xFFFFFFFF: the data runs on.
        let end = if len == u32::MAX as usize { b.len() } else { (body + len).min(b.len()) };
        match id {
            b"fmt " => {
                let tag = u16_le(b, body).ok_or("short fmt chunk")?;
                let channels = u16_le(b, body + 2).ok_or("short fmt chunk")?;
                let rate = u32_le(b, body + 4).ok_or("short fmt chunk")?;
                let bits = u16_le(b, body + 14).ok_or("short fmt chunk")?;
                // WAVE_FORMAT_EXTENSIBLE: the real tag opens the sub-format.
                let tag = if tag == 0xFFFE { u16_le(b, body + 24).unwrap_or(1) } else { tag };
                format = Some((tag, channels, rate, bits));
            }
            b"data" => data = Some(&b[body..end]),
            _ => {}
        }
        at = body + len + (len & 1);
        if len == u32::MAX as usize {
            break;
        }
    }
    let (tag, channels, rate, bits) = format.ok_or("no fmt chunk")?;
    let data = data.ok_or("no audio data")?;
    let enc = match (tag, bits) {
        (1, 8) => Encoding::Int { bits, little: true, unsigned8: true },
        (1, 9..=32) => Encoding::Int { bits, little: true, unsigned8: false },
        (3, 32 | 64) => Encoding::Float { bits, little: true },
        _ => return Err(format!("unsupported WAV format (tag {tag}, {bits} bits)")),
    };
    let (left, right) = deinterleave(data, channels as usize, enc)?;
    if left.is_empty() {
        return Err("the file holds no sound".to_owned());
    }
    Ok(Sample::new(name, rate as f32, left, right))
}

/// An IEEE 754 80-bit extended number (AIFF's sample rate).
fn extended(b: &[u8]) -> f64 {
    let exp = (((b[0] as i32) & 0x7F) << 8 | b[1] as i32) - 16383;
    let mant = u64::from_be_bytes([b[2], b[3], b[4], b[5], b[6], b[7], b[8], b[9]]);
    let v = mant as f64 * 2f64.powi(exp - 63);
    if b[0] & 0x80 != 0 { -v } else { v }
}

fn decode_aiff(b: &[u8], name: &str) -> Result<Sample, String> {
    let aifc = match b.get(8..12) {
        Some(b"AIFF") => false,
        Some(b"AIFC") => true,
        _ => return Err("not an AIFF file".to_owned()),
    };
    let mut at = 12;
    let mut comm = None;
    let mut data = None;
    while at + 8 <= b.len() {
        let id = &b[at..at + 4];
        let len = u32_be(b, at + 4).unwrap_or(0) as usize;
        let body = at + 8;
        let end = (body + len).min(b.len());
        match id {
            b"COMM" => {
                let channels = u16_be(b, body).ok_or("short COMM chunk")?;
                let bits = u16_be(b, body + 6).ok_or("short COMM chunk")?;
                let rate = extended(b.get(body + 8..body + 18).ok_or("short COMM chunk")?);
                let kind: [u8; 4] = if aifc { b.get(body + 18..body + 22).and_then(|k| k.try_into().ok()).unwrap_or(*b"NONE") } else { *b"NONE" };
                comm = Some((channels, bits, rate, kind));
            }
            b"SSND" => {
                let offset = u32_be(b, body).unwrap_or(0) as usize;
                let from = (body + 8 + offset).min(end);
                data = Some(&b[from..end]);
            }
            _ => {}
        }
        at = body + len + (len & 1);
    }
    let (channels, bits, rate, kind) = comm.ok_or("no COMM chunk")?;
    let data = data.ok_or("no audio data")?;
    let enc = match &kind {
        b"NONE" | b"twos" => Encoding::Int { bits, little: false, unsigned8: false },
        b"sowt" => Encoding::Int { bits, little: true, unsigned8: false },
        b"fl32" | b"FL32" => Encoding::Float { bits: 32, little: false },
        b"fl64" | b"FL64" => Encoding::Float { bits: 64, little: false },
        other => return Err(format!("unsupported AIFF-C compression ({})", String::from_utf8_lossy(other))),
    };
    let (left, right) = deinterleave(data, channels as usize, enc)?;
    if left.is_empty() {
        return Err("the file holds no sound".to_owned());
    }
    Ok(Sample::new(name, rate as f32, left, right))
}

// ---------------------------------------------------------------------------
// Handing it to the audio thread
// ---------------------------------------------------------------------------

/// The sample a module holds, shared by its panel and its processor (see
/// the module's documentation). Empty for modules that hold none.
#[derive(Default)]
pub struct SampleSlot {
    current: Mutex<Option<Arc<Sample>>>,
    version: AtomicU32,
    /// The sample the processor let go, to be freed on the UI thread.
    retired: Mutex<Option<Arc<Sample>>>,
}

impl SampleSlot {
    /// The sample now (UI thread).
    pub fn get(&self) -> Option<Arc<Sample>> {
        self.current.lock().ok()?.clone()
    }

    /// Puts in a new sample, or none (UI thread).
    pub fn set(&self, sample: Option<Arc<Sample>>) {
        if let Ok(mut c) = self.current.lock() {
            *c = sample;
        }
        self.version.fetch_add(1, Ordering::Release);
        self.collect();
    }

    /// Frees what the processor let go (UI thread; the panel calls it as it
    /// draws).
    pub fn collect(&self) {
        let old = self.retired.lock().ok().and_then(|mut r| r.take());
        drop(old);
    }

    pub fn version(&self) -> u32 {
        self.version.load(Ordering::Acquire)
    }

    /// Audio thread: takes the newest sample into `holding` if the version
    /// moved past `seen`. Never blocks: if the UI holds a lock, or has not
    /// yet freed the last sample let go, it tries again next time. True
    /// when `holding` changed.
    pub fn fetch(&self, seen: &mut u32, holding: &mut Option<Arc<Sample>>) -> bool {
        let version = self.version.load(Ordering::Acquire);
        if version == *seen {
            return false;
        }
        let Ok(mut retired) = self.retired.try_lock() else { return false };
        if retired.is_some() {
            return false;
        }
        let Ok(current) = self.current.try_lock() else { return false };
        let new = current.clone();
        drop(current);
        *retired = std::mem::replace(holding, new);
        *seen = version;
        true
    }
}
