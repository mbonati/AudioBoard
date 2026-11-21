//! Parameters shared lock-free between the UI thread and the audio thread.
//!
//! The UI writes parameter values into atomics; the audio thread reads them
//! once per block. Meters flow the other way: the audio thread accumulates
//! peaks and the UI drains them every frame.
//!
//! The vocabulary every node shares (atomics, specs, tapers, units and how
//! values read) comes from the node SDK, `audioboard_node::param`.

use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};

pub use audioboard_node::param::{
    AtomicF32, DIVISIONS, FADER_UNITY, NOTE_NAMES, NOTE_VALUES, ParamSpec, SYNC_DIVISIONS, Taper,
    Unit, fader_gain, format_db, format_hz, format_seconds, gain_to_db, note_label, sync_division,
};

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
/// The Console's channels (it shows 12 or all 24).
pub const CONSOLE_CHANNELS: usize = 24;

// FADER_UNITY and fader_gain (the fader law) come from the node SDK.

/// A mixer's settings, for any number of channels. Each channel also sends
/// (after its fader and pan) to the AUX output; sends and AUX use the fader
/// law.
pub struct MixerParams {
    pub fader: Box<[AtomicF32]>,
    pub pan: Box<[AtomicF32]>,
    pub mute: Box<[AtomicBool]>,
    pub solo: Box<[AtomicBool]>,
    pub aux: Box<[AtomicF32]>,
    pub master: AtomicF32,
    /// Silences the main output (the AUX output goes on).
    pub master_mute: AtomicBool,
    pub aux_master: AtomicF32,
    pub ch_meter: Box<[AtomicF32]>,
    pub master_meter: [AtomicF32; 2],
    pub aux_meter: [AtomicF32; 2],
}

impl MixerParams {
    pub fn new(channels: usize) -> Self {
        let each = |v: f32| (0..channels).map(|_| AtomicF32::new(v)).collect();
        let flags = || (0..channels).map(|_| AtomicBool::new(false)).collect();
        Self {
            fader: each(FADER_UNITY),
            pan: each(0.0),
            mute: flags(),
            solo: flags(),
            aux: each(0.0),
            master: AtomicF32::new(FADER_UNITY),
            master_mute: AtomicBool::new(false),
            aux_master: AtomicF32::new(FADER_UNITY),
            ch_meter: each(0.0),
            master_meter: std::array::from_fn(|_| AtomicF32::new(0.0)),
            aux_meter: std::array::from_fn(|_| AtomicF32::new(0.0)),
        }
    }

    pub fn channels(&self) -> usize {
        self.fader.len()
    }

    /// A new set with the same settings (duplicating a mixer).
    pub fn copy(&self) -> Self {
        let p = Self::new(self.channels());
        let r = Ordering::Relaxed;
        for ch in 0..self.channels() {
            p.fader[ch].set(self.fader[ch].get());
            p.pan[ch].set(self.pan[ch].get());
            p.aux[ch].set(self.aux[ch].get());
            p.mute[ch].store(self.mute[ch].load(r), r);
            p.solo[ch].store(self.solo[ch].load(r), r);
        }
        p.master.set(self.master.get());
        p.master_mute.store(self.master_mute.load(r), r);
        p.aux_master.set(self.aux_master.get());
        p
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

    /// How many samples have been written so far (wrapping): with
    /// [`Self::sample`], a reader can follow the new ones.
    pub fn written(&self) -> usize {
        self.pos.load(Ordering::Acquire)
    }

    /// The sample written at count `i` (see [`Self::written`]); only the
    /// last [`SCOPE_LEN`] are kept.
    pub fn sample(&self, i: usize) -> f32 {
        self.buf[i & (SCOPE_LEN - 1)].get()
    }

    /// Samples the scope keeps.
    pub const LEN: usize = SCOPE_LEN;

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

/// Tempo of a new patch, in BPM.
pub const DEFAULT_TEMPO: f32 = 90.0;
/// Slowest and fastest tempo, in beats per minute.
pub const TEMPO_RANGE: (f32, f32) = (20.0, 300.0);

pub struct EngineShared {
    /// The transport is playing: sources sound and sequencers run. Stopped,
    /// the engine keeps running, so effects and instruments ring out.
    pub running: AtomicBool,
    /// Bumped by Stop: the transport goes back to beat 0, and sequencers
    /// start over on the next Start.
    pub rewinds: AtomicU32,
    /// Transport tempo in BPM, shared by sequencers and arpeggiators.
    pub tempo: AtomicF32,
    /// Transport position in beats (wrapped), for the UI's beat light.
    pub beat: AtomicF32,
    /// DSP load: time spent rendering / time available (0..1+).
    pub load: AtomicF32,
    /// Highest load of one callback since the UI last looked.
    pub peak_load: AtomicF32,
    /// Callbacks that took longer than the audio they produced lasts: the
    /// output ran dry, an audible dropout.
    pub overloads: AtomicU32,
    /// Frames in the last callback: the device's actual buffer size.
    pub frames: AtomicU32,
    /// Deepest gain reduction of the output limiter (dB, positive) since
    /// the UI last looked.
    pub limiting: AtomicF32,
    /// The board's output level (linear), before the limiter.
    pub output: AtomicF32,
    pub scope: Scope,
    /// Where the output goes while the board is being recorded.
    pub tap: super::record::Tap,
    /// What the audio input captures, for the Audio In nodes.
    pub input: super::input::Capture,
    /// MIDI sync: the external clock followed, and the engine's timeline
    /// for the clock sent out.
    pub sync: super::sync::SyncShared,
}

impl EngineShared {
    pub fn new() -> Self {
        Self {
            running: AtomicBool::new(true),
            rewinds: AtomicU32::new(0),
            tempo: AtomicF32::new(DEFAULT_TEMPO),
            beat: AtomicF32::new(0.0),
            load: AtomicF32::new(0.0),
            peak_load: AtomicF32::new(0.0),
            overloads: AtomicU32::new(0),
            frames: AtomicU32::new(0),
            limiting: AtomicF32::new(0.0),
            output: AtomicF32::new(1.0),
            scope: Scope::new(),
            tap: super::record::Tap::new(),
            input: super::input::Capture::new(),
            sync: super::sync::SyncShared::new(),
        }
    }
}
