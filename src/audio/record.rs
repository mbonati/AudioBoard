//! Recording the board: what the audio device plays (after the output
//! limiter) is copied, frame by frame, into a lock-free ring ([`Tap`]); a
//! writer thread empties it into a raw file on disk, so a take can be as
//! long as the disk allows and the audio thread never waits. A finished
//! [`Take`] is exported to WAV, AIFF, FLAC, AAC… by `export`.

use std::fs::File;
use std::io::{BufWriter, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicUsize, Ordering};
use std::thread::JoinHandle;
use std::time::Duration;

use super::params::EngineShared;

/// Floats the ring holds: about five seconds of stereo at 48 kHz, far more
/// than the writer ever lets build up.
const TAP_LEN: usize = 1 << 19;

/// Takes started in this process, for unique file names.
static TAKES: AtomicU64 = AtomicU64::new(0);

/// The audio thread's side: a single-producer, single-consumer ring of
/// interleaved stereo frames.
pub struct Tap {
    on: AtomicBool,
    buf: Box<[AtomicU32]>,
    /// Floats written (by the audio thread) and read (by the writer).
    write: AtomicUsize,
    read: AtomicUsize,
    /// Frames lost because the writer fell behind.
    dropped: AtomicU64,
}

impl Default for Tap {
    fn default() -> Self {
        Self::new()
    }
}

impl Tap {
    pub fn new() -> Self {
        Self {
            on: AtomicBool::new(false),
            buf: (0..TAP_LEN).map(|_| AtomicU32::new(0)).collect(),
            write: AtomicUsize::new(0),
            read: AtomicUsize::new(0),
            dropped: AtomicU64::new(0),
        }
    }

    /// The board is being recorded.
    pub fn recording(&self) -> bool {
        self.on.load(Ordering::Relaxed)
    }

    /// One frame, from the audio thread.
    #[inline]
    pub fn push(&self, l: f32, r: f32) {
        let w = self.write.load(Ordering::Relaxed);
        if w - self.read.load(Ordering::Acquire) + 2 > TAP_LEN {
            self.dropped.fetch_add(1, Ordering::Relaxed);
            return;
        }
        self.buf[w % TAP_LEN].store(l.to_bits(), Ordering::Relaxed);
        self.buf[(w + 1) % TAP_LEN].store(r.to_bits(), Ordering::Relaxed);
        self.write.store(w + 2, Ordering::Release);
    }

    /// Everything pushed since the last call, appended to `out`.
    fn drain(&self, out: &mut Vec<f32>) {
        let w = self.write.load(Ordering::Acquire);
        let mut r = self.read.load(Ordering::Relaxed);
        while r < w {
            out.push(f32::from_bits(self.buf[r % TAP_LEN].load(Ordering::Relaxed)));
            r += 1;
        }
        self.read.store(r, Ordering::Release);
    }
}

/// A finished recording: interleaved stereo f32 (little-endian) in a raw
/// file, deleted when the take is dropped.
pub struct Take {
    pub path: PathBuf,
    pub frames: u64,
    pub sample_rate: u32,
    /// The loudest sample.
    pub peak: f32,
    /// Frames lost because the disk couldn't keep up.
    pub dropped: u64,
}

impl Take {
    pub fn seconds(&self) -> f64 {
        self.frames as f64 / self.sample_rate.max(1) as f64
    }
}

impl Drop for Take {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// A recording under way: the writer thread, its stop switch and how much
/// it has written.
struct Running {
    thread: JoinHandle<Result<Take, String>>,
    stop: Arc<AtomicBool>,
    frames: Arc<AtomicU64>,
}

/// Starts and stops recordings; keeps the last take for exporting.
pub struct Recorder {
    shared: Arc<EngineShared>,
    sample_rate: u32,
    running: Option<Running>,
    pub take: Option<Arc<Take>>,
    /// Takes so far (for their names).
    pub count: u32,
}

impl Recorder {
    pub fn new(shared: Arc<EngineShared>) -> Self {
        Self { shared, sample_rate: 48_000, running: None, take: None, count: 0 }
    }

    pub fn recording(&self) -> bool {
        self.running.is_some()
    }

    /// Seconds recorded so far (or of the last take).
    pub fn seconds(&self) -> f64 {
        match &self.running {
            Some(r) => r.frames.load(Ordering::Relaxed) as f64 / self.sample_rate.max(1) as f64,
            None => self.take.as_ref().map_or(0.0, |t| t.seconds()),
        }
    }

    /// Where takes are written while they wait to be exported.
    fn dir() -> PathBuf {
        std::env::temp_dir().join("AudioBoard")
    }

    /// Starts recording what the device plays, at its `sample_rate`.
    pub fn start(&mut self, sample_rate: u32) -> Result<(), String> {
        if self.running.is_some() {
            return Ok(());
        }
        self.sample_rate = sample_rate;
        let dir = Self::dir();
        std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
        self.count += 1;
        let n = TAKES.fetch_add(1, Ordering::Relaxed);
        let path = dir.join(format!("take-{}-{n}.f32", std::process::id()));
        let file = File::create(&path).map_err(|e| e.to_string())?;
        let tap = &self.shared.tap;
        // Start from an empty ring.
        tap.read.store(tap.write.load(Ordering::Acquire), Ordering::Release);
        tap.dropped.store(0, Ordering::Relaxed);
        tap.on.store(true, Ordering::Release);

        let stop = Arc::new(AtomicBool::new(false));
        let frames = Arc::new(AtomicU64::new(0));
        let (shared, sample_rate) = (self.shared.clone(), self.sample_rate);
        let (stop_w, frames_w) = (stop.clone(), frames.clone());
        let thread = std::thread::Builder::new()
            .name("recorder".into())
            .spawn(move || {
                let mut out = BufWriter::with_capacity(1 << 20, file);
                let mut chunk = Vec::with_capacity(1 << 16);
                let mut peak = 0.0f32;
                loop {
                    let last = stop_w.load(Ordering::Acquire);
                    if last {
                        // The audio thread may still be finishing a buffer.
                        std::thread::sleep(Duration::from_millis(40));
                    }
                    chunk.clear();
                    shared.tap.drain(&mut chunk);
                    for s in &chunk {
                        peak = peak.max(s.abs());
                        out.write_all(&s.to_le_bytes()).map_err(|e| e.to_string())?;
                    }
                    frames_w.fetch_add(chunk.len() as u64 / 2, Ordering::Relaxed);
                    if last {
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(15));
                }
                out.flush().map_err(|e| e.to_string())?;
                Ok(Take {
                    path,
                    frames: frames_w.load(Ordering::Relaxed),
                    sample_rate,
                    peak,
                    dropped: shared.tap.dropped.load(Ordering::Relaxed),
                })
            })
            .map_err(|e| e.to_string())?;
        self.running = Some(Running { thread, stop, frames });
        Ok(())
    }

    /// Stops recording; the take becomes the one to export.
    pub fn stop(&mut self) -> Result<Arc<Take>, String> {
        let running = self.running.take().ok_or("not recording")?;
        self.shared.tap.on.store(false, Ordering::Release);
        running.stop.store(true, Ordering::Release);
        let take = Arc::new(running.thread.join().map_err(|_| "the recorder stopped unexpectedly".to_owned())??);
        self.take = Some(take.clone());
        Ok(take)
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        if self.running.is_some() {
            let _ = self.stop();
        }
    }
}
