//! The Trig inputs. On the Nord Drum these are jacks for drum pads and
//! acoustic triggers; here they are audio inputs, so a pad through an audio
//! interface, a drum loop or any percussive signal can play a channel (drum
//! replacement).
//!
//! Each input scans for the peak of a hit once it crosses the threshold,
//! then fires with a velocity from that peak and ignores the ringing that
//! follows: for a while not at all, then only above a threshold that falls
//! from the hit's peak more slowly than a drum rings (so its own decay does
//! not retrigger it, a new hit does). Trig Type sets how long it scans and how long it waits, to
//! suit rubber, plastic or mesh pads and triggers on kicks, snares and toms.

/// The Trig Types: name, what they suit, scan (ms), mask (ms), how fast the
/// threshold falls after the mask (ms).
pub const TYPES: [(&str, &str, f32, f32, f32); 6] = [
    ("Pd1", "rubber pads", 1.5, 20.0, 50.0),
    ("Pd2", "plastic-head pads", 2.0, 25.0, 60.0),
    ("Pd3", "mesh pads", 1.0, 15.0, 40.0),
    ("Ac1", "trigger on a kick or snare", 3.0, 40.0, 90.0),
    ("Ac2", "trigger on a high or mid tom", 3.0, 50.0, 120.0),
    ("Ac3", "trigger on a low tom", 4.0, 70.0, 160.0),
];

/// The dynamic response curves: Dn1 linear, Dn2 hotter.
pub const CURVES: [&str; 2] = ["Dn1", "Dn2"];

/// Sens (0…99): the input level that plays the hardest hit, in dB.
pub fn full_db(sens: f32) -> f32 {
    -0.45 * sens.clamp(0.0, 99.0)
}

/// Thres (0…99): the quietest level that triggers, in dB below full.
pub fn threshold_db(thres: f32) -> f32 {
    -48.0 * (1.0 - thres.clamp(0.0, 99.0) / 99.0)
}

/// Velocity through a response curve.
pub fn curve(vel: f32, curve: usize) -> f32 {
    let v = vel.clamp(0.0, 1.0);
    if curve == 1 { v.sqrt() } else { v }
}

/// One input's settings, worked out per block.
#[derive(Clone, Copy)]
pub struct Settings {
    pub full: f32,
    pub threshold: f32,
    pub scan: u32,
    pub mask: u32,
    /// Per-sample fall of the threshold a hit leaves.
    pub fall: f32,
    pub curve: usize,
}

impl Settings {
    pub fn new(kind: usize, curve: usize, thres: f32, sens: f32, sr: f32) -> Self {
        let (_, _, scan, mask, fall) = TYPES[kind.min(TYPES.len() - 1)];
        let full_db = full_db(sens);
        Settings {
            full: 10f32.powf(full_db / 20.0),
            threshold: 10f32.powf((full_db + threshold_db(thres)) / 20.0),
            scan: (scan * 0.001 * sr) as u32,
            mask: (mask * 0.001 * sr) as u32,
            fall: (-1.0 / (fall * 0.001 * sr)).exp(),
            curve: curve.min(1),
        }
    }
}

/// A hit found: its velocity (0…1) and how hard it was against the full
/// level (1: as hard as Sens expects, or harder).
#[derive(Clone, Copy, Debug)]
pub struct Hit {
    pub velocity: f32,
    pub strength: f32,
}

#[derive(Clone, Copy)]
enum State {
    Idle,
    Scan { left: u32, peak: f32 },
    Mask { left: u32 },
}

pub struct Detector {
    state: State,
    /// The threshold left by the last hit, falling.
    shadow: f32,
}

impl Default for Detector {
    fn default() -> Self {
        Detector { state: State::Idle, shadow: 0.0 }
    }
}

impl Detector {
    /// One sample (the louder of left and right); a hit when it fires.
    #[inline]
    pub fn tick(&mut self, x: f32, s: &Settings) -> Option<Hit> {
        let level = x.abs();
        self.shadow *= s.fall;
        match self.state {
            State::Idle => {
                if level > s.threshold.max(self.shadow) {
                    self.state = State::Scan { left: s.scan, peak: level };
                }
                None
            }
            State::Scan { left, peak } => {
                let peak = peak.max(level);
                if left > 0 {
                    self.state = State::Scan { left: left - 1, peak };
                    return None;
                }
                self.state = State::Mask { left: s.mask };
                self.shadow = peak;
                let strength = peak / s.full;
                Some(Hit { velocity: curve(strength, s.curve).max(1.0 / 127.0), strength })
            }
            State::Mask { left } => {
                self.state = if left > 0 { State::Mask { left: left - 1 } } else { State::Idle };
                None
            }
        }
    }
}
