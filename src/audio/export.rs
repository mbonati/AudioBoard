//! Exporting a recorded take. WAV and AIFF are written here (16- and 24-bit
//! with TPDF dither at 16, or 32-bit float); FLAC, Apple Lossless and AAC
//! go through macOS's own encoder (`afconvert`), MP3 through `ffmpeg` or
//! `lame` when one is installed. Options: normalize the peak to −1 dBFS,
//! and trim the silence at both ends.

use std::fs::File;
use std::io::{BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;

use super::modules::Rng;
use super::record::Take;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Format {
    Wav16,
    Wav24,
    WavFloat,
    Aiff16,
    Aiff24,
    Flac,
    Alac,
    Aac,
    Mp3,
}

pub const FORMATS: [Format; 9] =
    [Format::Wav24, Format::Wav16, Format::WavFloat, Format::Aiff24, Format::Aiff16, Format::Flac, Format::Alac, Format::Aac, Format::Mp3];

impl Format {
    pub fn label(self) -> &'static str {
        match self {
            Format::Wav16 => "WAV · 16-bit",
            Format::Wav24 => "WAV · 24-bit",
            Format::WavFloat => "WAV · 32-bit float",
            Format::Aiff16 => "AIFF · 16-bit",
            Format::Aiff24 => "AIFF · 24-bit",
            Format::Flac => "FLAC · lossless",
            Format::Alac => "Apple Lossless (.m4a)",
            Format::Aac => "AAC 256 kbps (.m4a)",
            Format::Mp3 => "MP3 320 kbps",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            Format::Wav16 | Format::Wav24 | Format::WavFloat => "wav",
            Format::Aiff16 | Format::Aiff24 => "aiff",
            Format::Flac => "flac",
            Format::Alac | Format::Aac => "m4a",
            Format::Mp3 => "mp3",
        }
    }

    /// Why it can't be used here, if it can't.
    pub fn missing(self) -> Option<&'static str> {
        match self {
            Format::Flac | Format::Alac | Format::Aac if tool("afconvert").is_none() => Some("needs macOS's afconvert"),
            Format::Mp3 if tool("ffmpeg").is_none() && tool("lame").is_none() => Some("needs ffmpeg or lame installed (e.g. brew install lame)"),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub struct Options {
    pub format: Format,
    /// Raise (or lower) the whole take so its peak sits at −1 dBFS.
    pub normalize: bool,
    /// Drop the silence before the first sound and after the last.
    pub trim: bool,
}

impl Default for Options {
    fn default() -> Self {
        Self { format: Format::Wav24, normalize: false, trim: true }
    }
}

/// Below this a sample counts as silence (−80 dBFS).
const SILENCE: f32 = 1e-4;
/// Kept around the sound when trimming (seconds).
const TRIM_PAD: f64 = 0.01;
/// Normalized peak: −1 dBFS.
const NORMAL_PEAK: f32 = 0.891_25;

/// A program on this machine (a GUI app doesn't see the shell's PATH, so
/// the usual places are tried too).
pub fn tool(name: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.extend(["/usr/bin", "/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    dirs.into_iter().map(|d| d.join(name)).find(|p| p.is_file())
}

/// Reads the take's frames `[from, to)` in chunks, as interleaved stereo.
fn read(take: &Take, from: u64, to: u64, mut each: impl FnMut(&[f32]) -> Result<(), String>) -> Result<(), String> {
    let file = File::open(&take.path).map_err(|e| e.to_string())?;
    let mut reader = BufReader::with_capacity(1 << 20, file);
    let mut skip = from * 8;
    let mut bytes = vec![0u8; 1 << 16];
    let mut left = to.saturating_sub(from) * 8;
    while skip > 0 {
        let n = (skip as usize).min(bytes.len());
        reader.read_exact(&mut bytes[..n]).map_err(|e| e.to_string())?;
        skip -= n as u64;
    }
    let mut samples = Vec::with_capacity(bytes.len() / 4);
    while left > 0 {
        let n = (left as usize).min(bytes.len());
        reader.read_exact(&mut bytes[..n]).map_err(|e| e.to_string())?;
        left -= n as u64;
        samples.clear();
        samples.extend(bytes[..n].as_chunks::<4>().0.iter().map(|b| f32::from_le_bytes(*b)));
        each(&samples)?;
    }
    Ok(())
}

/// The frames worth keeping (all, or trimmed to the sound) and their peak.
fn survey(take: &Take, trim: bool) -> Result<(u64, u64, f32), String> {
    let (mut first, mut last, mut peak, mut at) = (None, 0u64, 0.0f32, 0u64);
    read(take, 0, take.frames, |chunk| {
        for frame in chunk.as_chunks::<2>().0 {
            let m = frame[0].abs().max(frame[1].abs());
            peak = peak.max(m);
            if m > SILENCE {
                first.get_or_insert(at);
                last = at;
            }
            at += 1;
        }
        Ok(())
    })?;
    if !trim {
        return Ok((0, take.frames, peak));
    }
    let Some(first) = first else { return Ok((0, 0, 0.0)) };
    let pad = (TRIM_PAD * take.sample_rate as f64) as u64;
    Ok((first.saturating_sub(pad), (last + 1 + pad).min(take.frames), peak))
}

/// Writes the take to `dest` as `options` say.
pub fn export(take: &Take, options: Options, dest: &Path) -> Result<(), String> {
    if let Some(why) = options.format.missing() {
        return Err(format!("{} {why}", options.format.label()));
    }
    let (from, to, peak) = survey(take, options.trim)?;
    if to <= from {
        return Err("the take is silent".to_owned());
    }
    let gain = if options.normalize && peak > 1e-6 { NORMAL_PEAK / peak } else { 1.0 };
    let pcm = |container, sample, path: &Path| write_pcm(take, from, to, gain, container, sample, path);
    match options.format {
        Format::Wav16 => pcm(Container::Wav, Sample::I16, dest),
        Format::Wav24 => pcm(Container::Wav, Sample::I24, dest),
        Format::WavFloat => pcm(Container::Wav, Sample::F32, dest),
        Format::Aiff16 => pcm(Container::Aiff, Sample::I16, dest),
        Format::Aiff24 => pcm(Container::Aiff, Sample::I24, dest),
        Format::Flac | Format::Alac | Format::Aac | Format::Mp3 => {
            // Through a 24-bit WAV, handed to the encoder.
            static EXPORTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let n = EXPORTS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let wav = std::env::temp_dir().join(format!("audioboard-export-{}-{n}.wav", std::process::id()));
            pcm(Container::Wav, Sample::I24, &wav)?;
            let result = encode(options.format, &wav, dest);
            let _ = std::fs::remove_file(&wav);
            result
        }
    }
}

/// Runs the encoder for a compressed format on a WAV.
fn encode(format: Format, wav: &Path, dest: &Path) -> Result<(), String> {
    let mut cmd = match format {
        Format::Flac | Format::Alac | Format::Aac => {
            let mut c = Command::new(tool("afconvert").ok_or("afconvert not found")?);
            match format {
                Format::Flac => c.args(["-f", "flac", "-d", "flac"]),
                Format::Alac => c.args(["-f", "m4af", "-d", "alac"]),
                _ => c.args(["-f", "m4af", "-d", "aac", "-b", "256000"]),
            };
            c.arg(wav).arg(dest);
            c
        }
        _ => match tool("ffmpeg") {
            Some(ffmpeg) => {
                let mut c = Command::new(ffmpeg);
                c.args(["-y", "-loglevel", "error", "-i"]).arg(wav).args(["-codec:a", "libmp3lame", "-b:a", "320k"]).arg(dest);
                c
            }
            None => {
                let mut c = Command::new(tool("lame").ok_or("no MP3 encoder found")?);
                c.args(["--silent", "-b", "320"]).arg(wav).arg(dest);
                c
            }
        },
    };
    let out = cmd.output().map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        let why = String::from_utf8_lossy(&out.stderr).trim().to_owned();
        Err(if why.is_empty() { format!("the encoder failed ({})", out.status) } else { why })
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Container {
    Wav,
    Aiff,
}

#[derive(Clone, Copy, PartialEq)]
enum Sample {
    I16,
    I24,
    F32,
}

impl Sample {
    fn bytes(self) -> usize {
        match self {
            Sample::I16 => 2,
            Sample::I24 => 3,
            Sample::F32 => 4,
        }
    }
}

/// An IEEE 754 80-bit extended number (AIFF's sample rate).
fn extended(rate: u32) -> [u8; 10] {
    let mut out = [0u8; 10];
    if rate == 0 {
        return out;
    }
    let mut mantissa = rate as u64;
    let mut exponent: u16 = 16383 + 63;
    while mantissa & (1 << 63) == 0 {
        mantissa <<= 1;
        exponent -= 1;
    }
    out[..2].copy_from_slice(&exponent.to_be_bytes());
    out[2..].copy_from_slice(&mantissa.to_be_bytes());
    out
}

/// Frames `[from, to)` of the take, times `gain`, as a WAV or AIFF file.
#[allow(clippy::too_many_arguments)]
fn write_pcm(take: &Take, from: u64, to: u64, gain: f32, container: Container, sample: Sample, dest: &Path) -> Result<(), String> {
    let frames = to - from;
    let data = frames * 2 * sample.bytes() as u64;
    if data > u32::MAX as u64 - 64 {
        return Err("the take is too long for this format (4 GB); try FLAC".to_owned());
    }
    let data = data as u32;
    let file = File::create(dest).map_err(|e| e.to_string())?;
    let mut out = BufWriter::with_capacity(1 << 20, file);
    let mut head = Vec::with_capacity(64);
    let bits = (sample.bytes() * 8) as u16;
    match container {
        Container::Wav => {
            let float = sample == Sample::F32;
            let rate = take.sample_rate;
            head.extend_from_slice(b"RIFF");
            head.extend_from_slice(&(36 + data).to_le_bytes());
            head.extend_from_slice(b"WAVEfmt ");
            head.extend_from_slice(&16u32.to_le_bytes());
            head.extend_from_slice(&(if float { 3u16 } else { 1 }).to_le_bytes());
            head.extend_from_slice(&2u16.to_le_bytes());
            head.extend_from_slice(&rate.to_le_bytes());
            head.extend_from_slice(&(rate * 2 * sample.bytes() as u32).to_le_bytes());
            head.extend_from_slice(&((2 * sample.bytes()) as u16).to_le_bytes());
            head.extend_from_slice(&bits.to_le_bytes());
            head.extend_from_slice(b"data");
            head.extend_from_slice(&data.to_le_bytes());
        }
        Container::Aiff => {
            head.extend_from_slice(b"FORM");
            head.extend_from_slice(&(4 + 26 + 16 + data).to_be_bytes());
            head.extend_from_slice(b"AIFFCOMM");
            head.extend_from_slice(&18u32.to_be_bytes());
            head.extend_from_slice(&2u16.to_be_bytes());
            head.extend_from_slice(&(frames as u32).to_be_bytes());
            head.extend_from_slice(&bits.to_be_bytes());
            head.extend_from_slice(&extended(take.sample_rate));
            head.extend_from_slice(b"SSND");
            head.extend_from_slice(&(8 + data).to_be_bytes());
            head.extend_from_slice(&[0; 8]);
        }
    }
    out.write_all(&head).map_err(|e| e.to_string())?;

    let big = container == Container::Aiff;
    let mut rng = Rng::new(0x5eed);
    let mut bytes = Vec::with_capacity(1 << 18);
    read(take, from, to, |chunk: &[f32]| {
        bytes.clear();
        for &s in chunk {
            let x = s * gain;
            match sample {
                Sample::F32 => bytes.extend_from_slice(&x.clamp(-1.0, 1.0).to_le_bytes()),
                Sample::I16 => {
                    // TPDF dither, one LSB.
                    let d = (rng.unit() - rng.unit()) / 32_768.0;
                    let v = ((x + d) * 32_767.0).round().clamp(-32_768.0, 32_767.0) as i16;
                    bytes.extend_from_slice(&if big { v.to_be_bytes() } else { v.to_le_bytes() });
                }
                Sample::I24 => {
                    let v = (x * 8_388_607.0).round().clamp(-8_388_608.0, 8_388_607.0) as i32;
                    let b = v.to_le_bytes();
                    if big {
                        bytes.extend_from_slice(&[b[2], b[1], b[0]]);
                    } else {
                        bytes.extend_from_slice(&b[..3]);
                    }
                }
            }
        }
        out.write_all(&bytes).map_err(|e| e.to_string())
    })?;
    out.flush().map_err(|e| e.to_string())
}
