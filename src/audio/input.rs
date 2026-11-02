//! Audio input: the input device's callback writes what it captures into a
//! lock-free ring ([`Capture`]); at the start of each output buffer the
//! engine takes as many frames as it is about to play, and the Audio In
//! nodes read their channels from them. Input and output may be different
//! devices whose clocks drift apart: the ring keeps a small cushion, fills
//! it again after a dropout and skips ahead when it grows too deep.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

use cpal::{FromSample, Sample};

use super::engine::{BLOCK, Buf};
use super::params::AtomicF32;

/// Input channels an Audio In node can pick from.
pub const MAX_CHANNELS: usize = 16;

/// Frames the ring holds: about 0.7 s at 48 kHz, far more than the cushion.
const RING: usize = 1 << 15;

pub struct Capture {
    /// The open input's channels (0: none open).
    channels: AtomicUsize,
    /// Frames of `MAX_CHANNELS` samples.
    ring: Box<[AtomicU32]>,
    /// Frames written by the device, and taken by the engine.
    write: AtomicUsize,
    read: AtomicUsize,
    /// The largest buffer the device has delivered: the cushion kept.
    chunk: AtomicUsize,
    /// Set when the input changes: the engine starts over from what is new.
    reset: AtomicBool,
    /// The cushion is there; until it is, the engine waits (silence).
    primed: AtomicBool,
    /// The engine's window for the current output buffer: frames from
    /// `start`, the first `valid` of them captured (the rest silence).
    start: AtomicUsize,
    valid: AtomicUsize,
    /// Counts output buffers, so the nodes know when a new window starts.
    render: AtomicU32,
    /// Peak per channel since the UI last looked.
    peaks: [AtomicF32; MAX_CHANNELS],
    /// Times the input ran dry (a short gap), and ran so far ahead that
    /// frames were skipped.
    pub dropouts: AtomicU32,
    pub skips: AtomicU32,
}

impl Default for Capture {
    fn default() -> Self {
        Self::new()
    }
}

impl Capture {
    pub fn new() -> Self {
        Self {
            channels: AtomicUsize::new(0),
            ring: (0..RING * MAX_CHANNELS).map(|_| AtomicU32::new(0)).collect(),
            write: AtomicUsize::new(0),
            read: AtomicUsize::new(0),
            chunk: AtomicUsize::new(0),
            reset: AtomicBool::new(false),
            primed: AtomicBool::new(false),
            start: AtomicUsize::new(0),
            valid: AtomicUsize::new(0),
            render: AtomicU32::new(0),
            peaks: std::array::from_fn(|_| AtomicF32::new(0.0)),
            dropouts: AtomicU32::new(0),
            skips: AtomicU32::new(0),
        }
    }

    /// An input with `channels` is about to start (UI thread).
    pub fn open(&self, channels: usize) {
        self.chunk.store(0, Ordering::Relaxed);
        self.reset.store(true, Ordering::Release);
        self.channels.store(channels, Ordering::Release);
    }

    /// The input stopped.
    pub fn close(&self) {
        self.channels.store(0, Ordering::Release);
        for p in &self.peaks {
            p.take();
        }
    }

    /// The open input's channels (0: none).
    pub fn channels(&self) -> usize {
        self.channels.load(Ordering::Relaxed)
    }

    /// The loudest sample per channel since the last call.
    pub fn take_peaks(&self) -> [f32; MAX_CHANNELS] {
        std::array::from_fn(|c| self.peaks[c].take())
    }

    /// One buffer from the device: `channels` interleaved (device thread).
    pub fn push<T>(&self, data: &[T], channels: usize)
    where
        T: Sample,
        f32: FromSample<T>,
    {
        let channels = channels.max(1);
        let frames = data.len() / channels;
        self.chunk.fetch_max(frames, Ordering::Relaxed);
        let w = self.write.load(Ordering::Relaxed);
        // Never over the engine's current window.
        let room = RING - 1 - (w - self.start.load(Ordering::Acquire)).min(RING - 1);
        let frames = frames.min(room);
        let keep = channels.min(MAX_CHANNELS);
        let mut peaks = [0.0f32; MAX_CHANNELS];
        for (f, frame) in data.chunks_exact(channels).take(frames).enumerate() {
            let at = ((w + f) % RING) * MAX_CHANNELS;
            for (c, s) in frame[..keep].iter().enumerate() {
                let x: f32 = s.to_sample();
                peaks[c] = peaks[c].max(x.abs());
                self.ring[at + c].store(x.to_bits(), Ordering::Relaxed);
            }
        }
        self.write.store(w + frames, Ordering::Release);
        for (p, v) in self.peaks.iter().zip(&peaks[..keep]) {
            p.fetch_max(*v);
        }
    }

    /// The engine is about to play `frames`: takes as many (audio thread).
    pub fn begin_render(&self, frames: usize) {
        let channels = self.channels.load(Ordering::Acquire);
        let w = self.write.load(Ordering::Acquire);
        let mut r = self.read.load(Ordering::Relaxed);
        let mut primed = self.primed.load(Ordering::Relaxed);
        if self.reset.swap(false, Ordering::AcqRel) || channels == 0 {
            r = w;
            primed = false;
        }
        // One device buffer of cushion: enough for the two callbacks to
        // come in any order.
        let cushion = self.chunk.load(Ordering::Relaxed).max(BLOCK);
        let mut avail = w - r;
        if channels > 0 && !primed && avail >= frames + cushion {
            primed = true;
        }
        let mut valid = 0;
        if primed {
            if avail < frames {
                // Ran dry: play what there is, then build the cushion again.
                self.dropouts.fetch_add(1, Ordering::Relaxed);
                primed = false;
                valid = avail;
            } else {
                // Far ahead (the input's clock is faster): back to the
                // cushion, so the latency stays low.
                if avail > 2 * frames + 4 * cushion {
                    self.skips.fetch_add(1, Ordering::Relaxed);
                    r += avail - (frames + 2 * cushion);
                    avail = frames + 2 * cushion;
                }
                valid = frames.min(avail);
            }
        }
        self.start.store(r, Ordering::Release);
        self.valid.store(valid, Ordering::Relaxed);
        self.read.store(r + valid, Ordering::Release);
        self.primed.store(primed, Ordering::Relaxed);
        self.render.fetch_add(1, Ordering::Release);
    }

    /// The current output buffer's number.
    pub fn render_id(&self) -> u32 {
        self.render.load(Ordering::Acquire)
    }

    /// Frames `at..at + n` of the current window: channel `left` and
    /// `right` (0-based) into `out`; silence where nothing was captured.
    pub fn read(&self, at: usize, n: usize, left: usize, right: usize, out: &mut Buf) {
        let start = self.start.load(Ordering::Acquire);
        let valid = self.valid.load(Ordering::Relaxed);
        let channels = self.channels.load(Ordering::Relaxed).min(MAX_CHANNELS);
        let [out_l, out_r] = out;
        for (i, (l, r)) in out_l[..n].iter_mut().zip(&mut out_r[..n]).enumerate() {
            let k = at + i;
            (*l, *r) = if k < valid && left < channels {
                let base = ((start + k) % RING) * MAX_CHANNELS;
                let sample = |c: usize| f32::from_bits(self.ring[base + c.min(channels - 1)].load(Ordering::Relaxed));
                (sample(left), sample(right))
            } else {
                (0.0, 0.0)
            };
        }
    }
}
