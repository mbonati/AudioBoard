//! Parameters shared lock-free between the UI thread and the audio thread.
//!
//! The UI writes parameter values into atomics; the audio thread reads them
//! once per block. Meters flow the other way: the audio thread accumulates
//! peaks and the UI drains them every frame.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

/// An `f32` stored in an `AtomicU32`. Relaxed ordering is enough: every
/// value is independent and a one-block delay is inaudible.
#[derive(Debug)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub fn new(v: f32) -> Self {
        Self(AtomicU32::new(v.to_bits()))
    }

    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }

    pub fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed);
    }

    /// Stores the max of the current and given value. For non-negative
    /// floats the bit pattern is monotonic, so an integer max is correct.
    pub fn fetch_max(&self, v: f32) {
        self.0.fetch_max(v.max(0.0).to_bits(), Ordering::Relaxed);
    }

    /// Reads the value and resets it to zero (used to drain peak meters).
    pub fn take(&self) -> f32 {
        f32::from_bits(self.0.swap(0, Ordering::Relaxed))
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Taper {
    Linear,
    Log,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Hz,
    Cents,
    Percent,
    Decibel,
}

/// Static description of a knob-controlled parameter.
pub struct ParamSpec {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub taper: Taper,
    pub unit: Unit,
    pub bipolar: bool,
}

impl ParamSpec {
    pub fn to_norm(&self, v: f32) -> f32 {
        let n = match self.taper {
            Taper::Linear => (v - self.min) / (self.max - self.min),
            Taper::Log => (v / self.min).ln() / (self.max / self.min).ln(),
        };
        n.clamp(0.0, 1.0)
    }

    pub fn denorm(&self, n: f32) -> f32 {
        let n = n.clamp(0.0, 1.0);
        match self.taper {
            Taper::Linear => self.min + n * (self.max - self.min),
            Taper::Log => self.min * (self.max / self.min).powf(n),
        }
    }

    pub fn format(&self, v: f32) -> String {
        match self.unit {
            Unit::Hz => format_hz(v),
            Unit::Cents => format!("{v:+.0} ct"),
            Unit::Percent => format!("{:.0}%", v * 100.0),
            Unit::Decibel => format_db(gain_to_db(v)),
        }
    }
}

pub fn format_hz(v: f32) -> String {
    if v >= 1000.0 {
        format!("{:.2} kHz", v / 1000.0)
    } else if v >= 100.0 {
        format!("{v:.0} Hz")
    } else if v >= 10.0 {
        format!("{v:.1} Hz")
    } else {
        format!("{v:.2} Hz")
    }
}

pub fn format_db(db: f32) -> String {
    if db <= -90.0 {
        "-inf dB".to_owned()
    } else {
        format!("{db:+.1} dB")
    }
}

pub fn gain_to_db(g: f32) -> f32 {
    if g <= 1e-5 { -100.0 } else { 20.0 * g.log10() }
}

// ---------------------------------------------------------------------------
// Oscillator
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Waveform {
    Sine,
    Triangle,
    Saw,
    Square,
    Noise,
}

impl Waveform {
    pub const ALL: [Waveform; 5] = [
        Waveform::Sine,
        Waveform::Triangle,
        Waveform::Saw,
        Waveform::Square,
        Waveform::Noise,
    ];

    pub fn from_u32(v: u32) -> Self {
        Self::ALL.get(v as usize).copied().unwrap_or(Waveform::Sine)
    }

    pub fn name(self) -> &'static str {
        match self {
            Waveform::Sine => "SINE",
            Waveform::Triangle => "TRI",
            Waveform::Saw => "SAW",
            Waveform::Square => "PULSE",
            Waveform::Noise => "NOISE",
        }
    }

    /// What the SHAPE knob does for this waveform.
    pub fn shape_label(self) -> &'static str {
        match self {
            Waveform::Sine | Waveform::Saw => "DRIVE",
            Waveform::Triangle => "SKEW",
            Waveform::Square => "WIDTH",
            Waveform::Noise => "COLOR",
        }
    }
}

pub mod osc {
    pub const FREQ: usize = 0;
    pub const FINE: usize = 1;
    pub const LEVEL: usize = 2;
    pub const SHAPE: usize = 3;
    pub const FM: usize = 4;
    pub const AM: usize = 5;
    pub const COUNT: usize = 6;
}

pub const OSC_PARAMS: [ParamSpec; osc::COUNT] = [
    ParamSpec {
        name: "FREQ",
        min: 0.1,
        max: 20_000.0,
        default: 220.0,
        taper: Taper::Log,
        unit: Unit::Hz,
        bipolar: false,
    },
    ParamSpec {
        name: "FINE",
        min: -100.0,
        max: 100.0,
        default: 0.0,
        taper: Taper::Linear,
        unit: Unit::Cents,
        bipolar: true,
    },
    ParamSpec {
        name: "LEVEL",
        min: 0.0,
        max: 1.0,
        default: 0.5,
        taper: Taper::Linear,
        unit: Unit::Decibel,
        bipolar: false,
    },
    ParamSpec {
        name: "SHAPE",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        taper: Taper::Linear,
        unit: Unit::Percent,
        bipolar: false,
    },
    ParamSpec {
        name: "FM",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        taper: Taper::Linear,
        unit: Unit::Percent,
        bipolar: false,
    },
    ParamSpec {
        name: "AM",
        min: 0.0,
        max: 1.0,
        default: 0.0,
        taper: Taper::Linear,
        unit: Unit::Percent,
        bipolar: false,
    },
];

pub struct OscParams {
    pub wave: AtomicU32,
    pub values: [AtomicF32; osc::COUNT],
    /// Output peak since the UI last looked.
    pub meter: AtomicF32,
}

impl OscParams {
    pub fn new() -> Self {
        Self {
            wave: AtomicU32::new(Waveform::Sine as u32),
            values: std::array::from_fn(|i| AtomicF32::new(OSC_PARAMS[i].default)),
            meter: AtomicF32::new(0.0),
        }
    }

    pub fn get(&self, i: usize) -> f32 {
        self.values[i].get()
    }

    pub fn set(&self, i: usize, v: f32) {
        self.values[i].set(v);
    }

    pub fn waveform(&self) -> Waveform {
        Waveform::from_u32(self.wave.load(Ordering::Relaxed))
    }

    pub fn set_waveform(&self, w: Waveform) {
        self.wave.store(w as u32, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Mixer
// ---------------------------------------------------------------------------

pub const MIXER_CHANNELS: usize = 4;

/// Fader position (0..1) that corresponds to unity gain.
pub const FADER_UNITY: f32 = 0.793_700_5; // cbrt(0.5)

/// Cubic fader law: 0 → -inf, unity at ~79% travel, +6 dB at the top.
pub fn fader_gain(pos: f32) -> f32 {
    let p = pos.clamp(0.0, 1.0);
    2.0 * p * p * p
}

pub struct MixerParams {
    pub fader: [AtomicF32; MIXER_CHANNELS],
    pub pan: [AtomicF32; MIXER_CHANNELS],
    pub mute: [AtomicBool; MIXER_CHANNELS],
    pub solo: [AtomicBool; MIXER_CHANNELS],
    pub master: AtomicF32,
    pub ch_meter: [AtomicF32; MIXER_CHANNELS],
    pub master_meter: [AtomicF32; 2],
}

impl MixerParams {
    pub fn new() -> Self {
        Self {
            fader: std::array::from_fn(|_| AtomicF32::new(FADER_UNITY)),
            pan: std::array::from_fn(|_| AtomicF32::new(0.0)),
            mute: std::array::from_fn(|_| AtomicBool::new(false)),
            solo: std::array::from_fn(|_| AtomicBool::new(false)),
            master: AtomicF32::new(0.63), // about -6 dB: friendly to ears
            ch_meter: std::array::from_fn(|_| AtomicF32::new(0.0)),
            master_meter: std::array::from_fn(|_| AtomicF32::new(0.0)),
        }
    }
}

// ---------------------------------------------------------------------------
// Engine-wide shared state
// ---------------------------------------------------------------------------

const SCOPE_LEN: usize = 8192; // power of two

/// Ring buffer of the final mono output, read by the monitor panel.
pub struct Scope {
    buf: Box<[AtomicF32]>,
    pos: AtomicUsize,
}

impl Scope {
    fn new() -> Self {
        Self {
            buf: (0..SCOPE_LEN).map(|_| AtomicF32::new(0.0)).collect(),
            pos: AtomicUsize::new(0),
        }
    }

    pub fn write(&self, samples: &[f32]) {
        let mut p = self.pos.load(Ordering::Relaxed);
        for &s in samples {
            self.buf[p & (SCOPE_LEN - 1)].set(s);
            p = p.wrapping_add(1);
        }
        self.pos.store(p, Ordering::Release);
    }

    /// Copies the most recent `len` samples (oldest first) into `out`.
    pub fn snapshot(&self, out: &mut Vec<f32>, len: usize) {
        let len = len.min(SCOPE_LEN);
        let end = self.pos.load(Ordering::Acquire);
        out.clear();
        out.extend((0..len).map(|i| {
            let idx = end.wrapping_sub(len).wrapping_add(i);
            self.buf[idx & (SCOPE_LEN - 1)].get()
        }));
    }
}

pub struct EngineShared {
    pub running: AtomicBool,
    /// DSP load: time spent rendering / time available (0..1+).
    pub load: AtomicF32,
    pub scope: Scope,
}

impl EngineShared {
    pub fn new() -> Self {
        Self {
            running: AtomicBool::new(true),
            load: AtomicF32::new(0.0),
            scope: Scope::new(),
        }
    }
}
