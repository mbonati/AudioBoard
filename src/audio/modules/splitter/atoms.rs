//! Splitter's built-in sources: eight glitch atoms, so the node plays with no
//! sample loaded. Each is a pure function of its position (a second long,
//! counted in frames at [`ATOM_RATE`]), so slices, reverse, stutters and
//! grains read an atom exactly as they read a sample: from anywhere, in any
//! direction. Unlike a sample, an atom's pitch sets its frequency and leaves
//! its timing alone.

use crate::audio::fastmath::{exp2, sin_turns};

/// The rate an atom's position is counted at, and its length in frames.
pub const ATOM_RATE: f32 = 48_000.0;
pub const ATOM_LEN: f64 = 48_000.0;

pub struct Atom {
    pub name: &'static str,
    /// The frequency at PITCH 0.
    pub hz: f32,
    pub tip: &'static str,
}

/// Source 0 is the sample; sources 1…8 are these.
pub const ATOMS: [Atom; 8] = [
    Atom {
        name: "SINE",
        hz: 880.0,
        tip: "A pure sine (880 Hz at pitch 0): tones and long beeps",
    },
    Atom {
        name: "PING",
        hz: 1760.0,
        tip: "A sine that dies away in 200 ms: pings",
    },
    Atom {
        name: "CLICK",
        hz: 3000.0,
        tip: "One sample's impulse with a short ring: clicks and ticks",
    },
    Atom {
        name: "SUB",
        hz: 50.0,
        tip: "A sub-bass pulse falling onto its pitch (50 Hz at pitch 0)",
    },
    Atom {
        name: "NOISE",
        hz: 1000.0,
        tip: "Sample-and-hold noise at 8× the pitch: hiss to data bursts",
    },
    Atom {
        name: "DATA",
        hz: 1000.0,
        tip: "A stream of random bits in packets, 4× the pitch per second: modem and fax chatter",
    },
    Atom {
        name: "PULSE",
        hz: 220.0,
        tip: "A narrow pulse wave: buzzes and digital bleeps",
    },
    Atom {
        name: "CRACK",
        hz: 1000.0,
        tip: "Sparse impulses of random size and sign: crackle and dust",
    },
];

pub const SINE: usize = 0;
pub const PING: usize = 1;
pub const CLICK: usize = 2;
pub const SUB: usize = 3;
pub const NOISE: usize = 4;
pub const DATA: usize = 5;
pub const PULSE: usize = 6;
pub const CRACKLE: usize = 7;

/// A cheap integer hash (lowbias32).
#[inline]
pub fn hash(mut x: u32) -> u32 {
    x ^= x >> 16;
    x = x.wrapping_mul(0x7feb_352d);
    x ^= x >> 15;
    x = x.wrapping_mul(0x846c_a68b);
    x ^= x >> 16;
    x
}

/// −1…1 from a hash.
#[inline]
fn bipolar(h: u32) -> f32 {
    (h >> 8) as f32 * (2.0 / (1u32 << 24) as f32) - 1.0
}

/// Atom `kind` at `frame` (its position), at `hz`; `seed` varies the noisy
/// ones from slice to slice.
#[inline]
pub fn atom(kind: usize, frame: f64, hz: f32, seed: u32) -> f32 {
    let t = (frame / ATOM_RATE as f64).max(0.0);
    let tf = t as f32;
    // Phases in f64: a second of a high tone is many thousand turns.
    let turns = |f: f32| ((f as f64 * t).fract()) as f32;
    match kind {
        SINE => sin_turns(turns(hz)),
        PING => sin_turns(turns(hz)) * exp2(-tf * 50.0),
        CLICK => {
            let spike = if frame < 1.0 { 1.0 } else { 0.0 };
            spike + 0.55 * sin_turns(turns(hz)) * exp2(-tf * 1500.0)
        }
        SUB => {
            // The pitch starts four times higher and falls in about 25 ms:
            // phase = f·t + 3·f·τ·(1 − e^(−t/τ)).
            const TAU: f32 = 0.025;
            let fall = 1.0 - exp2(-tf * (std::f32::consts::LOG2_E / TAU));
            let phase = (hz as f64 * t).fract() as f32 + 3.0 * hz * TAU * fall;
            sin_turns(phase) * 0.95
        }
        NOISE => {
            let cell = (t * hz as f64 * 8.0) as u32;
            bipolar(hash(cell ^ seed.wrapping_mul(0x9E37_79B9))) * 0.7
        }
        DATA => {
            let bit = (t * hz as f64 * 4.0) as u32;
            // Packets of 32 bits, 8 of silence between them.
            if bit % 40 >= 32 {
                0.0
            } else if hash(bit ^ seed.wrapping_mul(0x85EB_CA6B)) & 1 == 1 {
                0.45
            } else {
                -0.45
            }
        }
        PULSE => {
            if turns(hz) < 0.25 {
                0.6
            } else {
                -0.2
            }
        }
        _ => {
            let rate = hz as f64 * 2.0;
            let cell = (t * rate) as u32;
            let h = hash(cell ^ seed.wrapping_mul(0xC2B2_AE35));
            if h & 0xFF < 22 {
                let since = (t - cell as f64 / rate) as f32;
                bipolar(h) * exp2(-since * 9000.0)
            } else {
                0.0
            }
        }
    }
}

/// The frequency of atom `kind` at `semitones` from its own.
pub fn hz(kind: usize, semitones: f32) -> f32 {
    ATOMS[kind.min(ATOMS.len() - 1)].hz * exp2(semitones / 12.0)
}
