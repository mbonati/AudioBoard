//! The sound of one Fjord channel, built as the Nord Drum builds it: a
//! blend of three components, each with its own envelope.
//!
//! - **Tone**, the body: a waveform (three analog ones, nine drum-like
//!   modal sets, five metallic or percussive ones) at a pitch in half
//!   semitones, bent from above or below at the strike, through a low-pass
//!   filter that a sweep opens at the strike, with an optional punch on
//!   the attack.
//! - **Noise**: white noise tilted by COLOR (more or less bass), through a
//!   low-pass filter of seven types of rising resonance, opened by its own
//!   sweep.
//! - **Click**: a sharp transient at the very start: noise bursts in nine
//!   bands (n), the same with an added attack (c), or pitched ticks (t).
//!
//! Each decay has a Gate shape: gradual (n), a soft knee that holds and
//! then falls (S), or gated, held and then cut (G). The sweep and bend
//! times are shares of the decay, so the sound keeps its proportions when
//! the decay changes. Velocity scales the level, the sweeps, the bend and
//! the punch.
//!
//! Beyond the Nord Drum, the tone has two ADSRs, held as long as the note
//! (a MIDI key, a step's gate, TRIG held down): one can shape its
//! amplitude instead of DECAY and GATE, the other moves its filter up or
//! down by AMOUNT, on top of the sweep.

use super::knob;
use crate::audio::fastmath::{exp2, sin_turns, tan_turns, tanh};
use crate::audio::modules::Rng;

/// A channel's settings: its knobs in [`knob`] order, in panel units.
pub type Sound = [f32; knob::COUNT];

/// The decay shapes, as the display shows them.
pub const GATES: [&str; 3] = ["n", "S", "G"];

/// A knob as a whole number in `lo..=hi`.
pub fn int(s: &Sound, k: usize, lo: i32, hi: i32) -> i32 {
    (s[k].round() as i32).clamp(lo, hi)
}

/// Noise and Tone decay (0…50): seconds to fall 60 dB, 8 ms to 6 s.
pub fn decay_seconds(v: f32) -> f32 {
    0.008 * 750f32.powf(v.clamp(0.0, 50.0) / 50.0)
}

/// Click decay (0…20): 1 ms to 250 ms.
pub fn click_seconds(v: f32) -> f32 {
    0.001 * 250f32.powf(v.clamp(0.0, 20.0) / 20.0)
}

/// An ADSR's attack (0…50): 0.5 ms to 5 s.
pub fn attack_seconds(v: f32) -> f32 {
    0.0005 * 10_000f32.powf(v.clamp(0.0, 50.0) / 50.0)
}

/// The amplitude ADSR's sustain (0…99) as a gain.
pub fn sustain_gain(v: f32) -> f32 {
    let x = v.clamp(0.0, 99.0) / 99.0;
    x * x
}

/// How far a filter AMOUNT step moves the cutoff, in octaves.
pub const AMOUNT_OCTAVES: f32 = 0.8;

/// A Time setting (0…9) as a share of its decay: 9 is all of it.
pub fn time_share(v: f32) -> f32 {
    0.02 * 50f32.powf(v.clamp(0.0, 9.0) / 9.0)
}

/// A filter setting (0…99) in octaves above 20 Hz (99: ten octaves, open).
fn filter_octaves(v: f32) -> f32 {
    v.clamp(0.0, 99.0) / 99.0 * 10.0
}

pub fn filter_hz(v: f32) -> f32 {
    20.0 * 2f32.powf(filter_octaves(v))
}

/// A pitch (MIDI note number, 69 = A 440) in Hz.
pub fn pitch_hz(note: f32) -> f32 {
    440.0 * 2f32.powf((note - 69.0) / 12.0)
}

/// How far a full sweep (10) opens a filter, in octaves.
const SWEEP_OCTAVES: f32 = 0.75;
/// One Bend step, in semitones.
pub const BEND_STEP: f32 = 1.0;
/// Velocity's share in the sweeps and the bend (the rest is always there).
const VEL_SHARE: f32 = 0.7;

// ---------------------------------------------------------------------------
// Waveforms
// ---------------------------------------------------------------------------

/// How a waveform's partials are made.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Shape {
    /// Sines at the partials' ratios.
    Sines,
    Triangle,
    /// A band-limited sawtooth.
    Saw,
    /// Band-limited squares at the partials' ratios (cowbell, 808 metal).
    Squares,
    /// A sine phase-modulated by a sine at `ratio`, by `index`, the index
    /// fading `fade` times faster than the decay.
    Fm { ratio: f32, index: f32, fade: f32 },
    /// The sines, then driven into a soft clip.
    Clip(f32),
}

/// A Tone waveform: its display name ("An1", "b4", "c2"), what it is like,
/// how it is made and its partials (ratio to the pitch, level, how many
/// times faster than the decay it fades).
pub struct Wave {
    pub name: &'static str,
    pub long: &'static str,
    pub shape: Shape,
    pub partials: &'static [(f32, f32, f32)],
}

const SINE: &[(f32, f32, f32)] = &[(1.0, 1.0, 0.0)];

/// An 1–3 analog, b 1–9 drum-like (membrane modes), c 1–5 percussive.
pub const WAVES: [Wave; 17] = [
    Wave { name: "An1", long: "sine", shape: Shape::Sines, partials: SINE },
    Wave { name: "An2", long: "triangle", shape: Shape::Triangle, partials: SINE },
    Wave { name: "An3", long: "sawtooth", shape: Shape::Saw, partials: SINE },
    Wave { name: "b1", long: "kick: a rounded, driven sine", shape: Shape::Clip(1.8), partials: &[(1.0, 1.0, 0.0), (1.594, 0.12, 2.0)] },
    Wave {
        name: "b2",
        long: "tom: four membrane modes",
        shape: Shape::Sines,
        partials: &[(1.0, 1.0, 0.0), (1.594, 0.45, 1.5), (2.136, 0.28, 2.5), (2.296, 0.18, 3.0)],
    },
    Wave {
        name: "b3",
        long: "snare shell: six membrane modes",
        shape: Shape::Sines,
        partials: &[(1.0, 1.0, 0.0), (1.594, 0.7, 1.0), (2.136, 0.55, 1.6), (2.653, 0.42, 2.2), (2.918, 0.35, 2.6), (3.156, 0.28, 3.0)],
    },
    Wave {
        name: "b4",
        long: "timpani: tuned modes, a clear pitch",
        shape: Shape::Sines,
        partials: &[(1.0, 1.0, 0.0), (1.504, 0.65, 0.4), (1.742, 0.25, 1.2), (2.0, 0.45, 0.8), (2.245, 0.2, 1.6), (2.494, 0.25, 1.4)],
    },
    Wave {
        name: "b5",
        long: "tabla: harmonic overtones",
        shape: Shape::Sines,
        partials: &[(1.0, 1.0, 0.0), (2.0, 0.55, 0.5), (3.0, 0.38, 0.9), (4.0, 0.24, 1.3), (5.0, 0.14, 1.8)],
    },
    Wave {
        name: "b6",
        long: "conga: a round body and a quick slap",
        shape: Shape::Sines,
        partials: &[(1.0, 1.0, 0.0), (1.5, 0.6, 0.8), (2.0, 0.3, 1.5), (3.2, 0.35, 6.0), (4.1, 0.3, 8.0)],
    },
    Wave { name: "b7", long: "electronic tom: FM, bright at the strike", shape: Shape::Fm { ratio: 1.41, index: 2.2, fade: 2.5 }, partials: SINE },
    Wave {
        name: "b8",
        long: "steel drum: strong octave, melodic",
        shape: Shape::Sines,
        partials: &[(1.0, 1.0, 0.0), (2.0, 0.8, 0.3), (3.0, 0.35, 0.9), (4.0, 0.15, 1.3)],
    },
    Wave { name: "b9", long: "log drum: hollow, wooden", shape: Shape::Sines, partials: &[(1.0, 1.0, 0.0), (2.9, 0.5, 1.5), (4.95, 0.25, 3.0)] },
    Wave {
        name: "c1",
        long: "bell: hum, prime, minor third, fifth, nominal",
        shape: Shape::Sines,
        partials: &[(1.0, 1.0, 0.0), (2.0, 0.6, 0.2), (2.4, 0.5, 0.4), (3.0, 0.4, 0.6), (4.07, 0.3, 0.9), (5.43, 0.2, 1.2)],
    },
    Wave { name: "c2", long: "cowbell: two squares a fifth-and-a-bit apart", shape: Shape::Squares, partials: &[(1.0, 1.0, 0.0), (1.48, 0.8, 0.3)] },
    Wave {
        name: "c3",
        long: "metal: six squares, like an analog hihat",
        shape: Shape::Squares,
        partials: &[(1.0, 0.6, 0.0), (1.4827, 0.6, 0.0), (1.8003, 0.6, 0.0), (2.546, 0.6, 0.0), (2.6303, 0.6, 0.0), (3.8968, 0.6, 0.0)],
    },
    Wave { name: "c4", long: "gong: clangy FM", shape: Shape::Fm { ratio: 3.5, index: 4.0, fade: 0.6 }, partials: SINE },
    Wave {
        name: "c5",
        long: "bar: free-bar modes (glockenspiel, marimba)",
        shape: Shape::Sines,
        partials: &[(1.0, 1.0, 0.0), (2.756, 0.45, 1.2), (5.404, 0.25, 2.4), (8.933, 0.12, 3.6)],
    },
];

/// Most partials a waveform has.
pub const MAX_PARTIALS: usize = 6;

/// A note held until it is released.
const OPEN: u32 = u32::MAX;

/// The Click types: n (noise), c (noise with an attack), t (pitched).
pub const CLICK_FAMILIES: [&str; 3] = ["n", "c", "t"];

/// Display name of Click type `i` (0…26): "n1" … "t9".
pub fn click_name(i: usize) -> String {
    format!("{}{}", CLICK_FAMILIES[(i / 9).min(2)], i % 9 + 1)
}

/// Centre of each noise click's band, and each tick's pitch.
const CLICK_HZ: [f32; 9] = [200.0, 380.0, 700.0, 1150.0, 1800.0, 2800.0, 4300.0, 6500.0, 10_000.0];
const TICK_HZ: [f32; 9] = [140.0, 210.0, 320.0, 480.0, 720.0, 1080.0, 1600.0, 2400.0, 3600.0];

/// The noise filter's types (1…7) as resonance (Q).
pub const NOISE_Q: [f32; 7] = [0.55, 0.8, 1.2, 2.0, 3.5, 6.5, 13.0];

// ---------------------------------------------------------------------------
// Building blocks
// ---------------------------------------------------------------------------

/// A decay with its Gate shape, after a very short rise.
#[derive(Clone, Copy, Default)]
struct Env {
    /// 0 gradual (n), 1 soft knee (S), 2 gated (G).
    shape: u8,
    level: f32,
    /// n: per-sample decay; G: per-sample release after the hold.
    mul: f32,
    /// S and G: progress through the decay (S) or the hold (G).
    x: f32,
    dx: f32,
    rise: f32,
    d_rise: f32,
}

/// Per-sample factor for a fall of 60 dB in `seconds`.
fn fall(seconds: f32, sr: f32) -> f32 {
    (-6.9078 / (seconds * sr).max(1.0)).exp()
}

/// Per-sample factor for a time constant of `seconds`.
fn fade(seconds: f32, sr: f32) -> f32 {
    (-1.0 / (seconds * sr).max(1.0)).exp()
}

impl Env {
    fn new(gate: i32, seconds: f32, sr: f32, rise: f32) -> Self {
        let mut e = Env { shape: gate.clamp(0, 2) as u8, level: 1.0, rise: 0.0, d_rise: 1.0 / (rise * sr).max(1.0), ..Default::default() };
        match e.shape {
            0 => e.mul = fall(seconds, sr),
            1 => e.dx = 1.0 / (seconds * sr).max(1.0),
            _ => {
                // Held for a third of the decay, sagging 6 dB, then cut.
                e.dx = 1.0 / (0.35 * seconds * sr).max(1.0);
                e.mul = fade(gate_release(seconds), sr);
            }
        }
        e
    }

    #[inline]
    fn tick(&mut self) -> f32 {
        if self.rise < 1.0 {
            self.rise = (self.rise + self.d_rise).min(1.0);
        }
        match self.shape {
            0 => self.level *= self.mul,
            1 => {
                self.x += self.dx;
                self.level = exp2(-10.0 * self.x * self.x);
            }
            _ => {
                if self.x < 1.0 {
                    self.x += self.dx;
                    self.level = exp2(-self.x.min(1.0));
                } else {
                    self.level *= self.mul;
                }
            }
        }
        self.level * self.rise
    }

    fn done(&self) -> bool {
        match self.shape {
            1 => self.x > 1.25,
            _ => self.level < 1e-4,
        }
    }
}

/// How long a decay with this Gate lasts until silent, in seconds.
pub fn env_seconds(gate: i32, seconds: f32) -> f32 {
    match gate {
        0 => seconds * 1.34,
        1 => seconds * 1.25,
        _ => 0.35 * seconds + gate_release(seconds) * 8.5,
    }
}

/// The gated shape's cut after its hold: a time constant in seconds.
fn gate_release(seconds: f32) -> f32 {
    0.002 + 0.008 * seconds
}

/// An ADSR: a straight attack, a decay towards the sustain (60 dB of the
/// way in its time), held until released, then a release of the same kind.
#[derive(Clone, Copy, Default)]
struct Adsr {
    /// 0 attack, 1 decay, 2 sustain, 3 release, 4 done.
    stage: u8,
    level: f32,
    rise: f32,
    decay: f32,
    sustain: f32,
    release: f32,
}

impl Adsr {
    fn new(a: f32, d: f32, sustain: f32, r: f32, sr: f32) -> Self {
        Adsr { stage: 0, level: 0.0, rise: 1.0 / (a * sr).max(1.0), decay: fall(d, sr), sustain, release: fall(r, sr) }
    }

    #[inline]
    fn tick(&mut self) -> f32 {
        match self.stage {
            0 => {
                self.level += self.rise;
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = 1;
                }
            }
            1 => {
                self.level = self.sustain + (self.level - self.sustain) * self.decay;
                if (self.level - self.sustain).abs() < 1e-4 {
                    self.level = self.sustain;
                    self.stage = 2;
                }
            }
            3 => {
                self.level *= self.release;
                if self.level < 1e-4 {
                    self.level = 0.0;
                    self.stage = 4;
                }
            }
            _ => {}
        }
        self.level
    }

    fn release(&mut self) {
        if self.stage < 3 {
            self.stage = 3;
        }
    }

    fn done(&self) -> bool {
        self.stage == 4 || (self.stage == 2 && self.sustain < 1e-4)
    }
}

/// How long an ADSR lasts until silent when its note is held for `gate`.
pub fn adsr_seconds(a: f32, d: f32, sustain: f32, r: f32, gate: f32) -> f32 {
    let held = gate.max(a);
    let release = held + r * 1.34;
    if sustain > 0.0 { release } else { (a + d * 1.34).min(release) }
}

/// A state-variable filter (topology-preserving transform).
#[derive(Clone, Copy, Default)]
struct Svf {
    ic1: f32,
    ic2: f32,
}

impl Svf {
    /// Low-pass and band-pass outputs for prewarped `g` and damping `k`.
    #[inline]
    fn tick(&mut self, x: f32, g: f32, k: f32) -> (f32, f32) {
        let a1 = 1.0 / (1.0 + g * (g + k));
        let a2 = g * a1;
        let a3 = g * a2;
        let v3 = x - self.ic2;
        let v1 = a1 * self.ic1 + a2 * v3;
        let v2 = self.ic2 + a2 * self.ic1 + a3 * v3;
        self.ic1 = 2.0 * v1 - self.ic1;
        self.ic2 = 2.0 * v2 - self.ic2;
        (v2, v1)
    }
}

/// Prewarped coefficient for a cutoff in Hz (kept below Nyquist).
#[inline]
fn prewarp(hz: f32, sr: f32) -> f32 {
    tan_turns((hz / sr).clamp(1e-5, 0.47) * 0.5)
}

/// PolyBLEP correction around a wrap of a phase stepping by `dt`.
#[inline]
fn blep(t: f32, dt: f32) -> f32 {
    if t < dt {
        let x = t / dt;
        x + x - x * x - 1.0
    } else if t > 1.0 - dt {
        let x = (t - 1.0) / dt;
        x * x + x + x + 1.0
    } else {
        0.0
    }
}

#[inline]
fn wrap(x: f32) -> f32 {
    x - x.floor()
}

// ---------------------------------------------------------------------------
// The voice
// ---------------------------------------------------------------------------

/// What can change while a channel sounds (smoothed by the channel): the
/// pitch and the two filters; the noise's colour (see [`Tilt`]) and the
/// mixer too. Everything else is taken at the strike.
#[derive(Clone, Copy)]
pub struct Live {
    pub pitch: f32,
    pub tone_filter: f32,
    pub noise_filter: f32,
}

/// COLOR as a one-pole filter on the white noise, worked out per block:
/// below 0 a low-pass (more bass, made up in level), above 0 a high-pass.
#[derive(Clone, Copy)]
pub struct Tilt {
    mode: i8,
    k: f32,
    gain: f32,
}

impl Tilt {
    pub fn new(color: f32, sr: f32) -> Self {
        let c = color.clamp(-10.0, 10.0);
        let coeff = |hz: f32| (1.0 - (-std::f32::consts::TAU * hz / sr).exp()).min(1.0);
        if c < -0.01 {
            let hz = 18_000.0 * exp2(c * 0.62);
            Tilt { mode: -1, k: coeff(hz), gain: (18_000.0 / hz).powf(0.35) }
        } else if c > 0.01 {
            Tilt { mode: 1, k: coeff(30.0 * exp2(c * 0.8)), gain: 1.0 }
        } else {
            Tilt { mode: 0, k: 0.0, gain: 1.0 }
        }
    }
}

impl Live {
    pub fn of(s: &Sound) -> Self {
        Live { pitch: s[knob::PITCH], tone_filter: s[knob::T_FILTER], noise_filter: s[knob::N_FILTER] }
    }
}

/// One strike of a channel: the three components after their envelopes,
/// before the mixer.
pub struct Voice {
    pub active: bool,
    sr: f32,
    // Tone.
    shape: Shape,
    count: usize,
    ratios: [f32; MAX_PARTIALS],
    phases: [f32; MAX_PARTIALS],
    gains: [f32; MAX_PARTIALS],
    fades: [f32; MAX_PARTIALS],
    norm: f32,
    mod_phase: f32,
    fm_index: f32,
    fm_fade: f32,
    tone_env: Env,
    tone_svf: Svf,
    tone_on: bool,
    tone_sweep: f32,
    tone_sweep_mul: f32,
    tone_sweep_oct: f32,
    bend: f32,
    bend_mul: f32,
    punch: f32,
    punch_mul: f32,
    /// The tone's ADSRs: the amplitude's (when it replaces DECAY) and the
    /// filter's, with how far it moves the cutoff (octaves).
    adsr: bool,
    amp_adsr: Adsr,
    filter_adsr: Adsr,
    filter_amount: f32,
    /// Samples until the note is let go ([`OPEN`]: held until released).
    gate_left: u32,
    // Noise.
    noise_env: Env,
    noise_svf: Svf,
    noise_on: bool,
    tilt: f32,
    noise_k: f32,
    noise_norm: f32,
    noise_sweep: f32,
    noise_sweep_mul: f32,
    noise_sweep_oct: f32,
    // Click.
    click_env: Env,
    click_svf: Svf,
    click_on: bool,
    family: usize,
    click_g: f32,
    click_k: f32,
    click_gain: f32,
    click_phase: f32,
    click_hz: f32,
    chirp: f32,
    chirp_mul: f32,
    /// Samples of the c clicks' attack still to play, and its step.
    attack_left: u32,
    attack_inc: f32,
    rng: Rng,
}

impl Voice {
    pub fn new(sr: f32, seed: u32) -> Self {
        Voice {
            active: false,
            sr,
            shape: Shape::Sines,
            count: 1,
            ratios: [1.0; MAX_PARTIALS],
            phases: [0.0; MAX_PARTIALS],
            gains: [0.0; MAX_PARTIALS],
            fades: [1.0; MAX_PARTIALS],
            norm: 1.0,
            mod_phase: 0.0,
            fm_index: 0.0,
            fm_fade: 1.0,
            tone_env: Env::default(),
            tone_svf: Svf::default(),
            tone_on: false,
            tone_sweep: 0.0,
            tone_sweep_mul: 0.0,
            tone_sweep_oct: 0.0,
            bend: 0.0,
            bend_mul: 0.0,
            punch: 0.0,
            punch_mul: 0.0,
            adsr: false,
            amp_adsr: Adsr::default(),
            filter_adsr: Adsr::default(),
            filter_amount: 0.0,
            gate_left: 0,
            noise_env: Env::default(),
            noise_svf: Svf::default(),
            noise_on: false,
            tilt: 0.0,
            noise_k: 1.0,
            noise_norm: 1.0,
            noise_sweep: 0.0,
            noise_sweep_mul: 0.0,
            noise_sweep_oct: 0.0,
            click_env: Env::default(),
            click_svf: Svf::default(),
            click_on: false,
            family: 0,
            click_g: 0.1,
            click_k: 1.0,
            click_gain: 1.0,
            click_phase: 0.0,
            click_hz: 1000.0,
            chirp: 0.0,
            chirp_mul: 0.0,
            attack_left: 0,
            attack_inc: 0.0,
            rng: Rng::new(seed),
        }
    }

    /// Starts a strike of `s` at velocity `vel` (0…1), its note held for
    /// `gate` seconds (None: until [`Self::release`]).
    pub fn trigger(&mut self, s: &Sound, vel: f32, gate: Option<f32>) {
        use knob::*;
        let sr = self.sr;
        let vel = vel.clamp(0.0, 1.0);
        let vel_scale = 1.0 - VEL_SHARE + VEL_SHARE * vel;
        self.active = true;

        self.gate_left = gate.map_or(OPEN, |g| ((g * sr) as u32).max(1));

        // Tone. With its ADSR, the sweep, bend and partials' times follow
        // the ADSR's decay.
        let wave = &WAVES[int(s, T_WAVE, 0, 16) as usize];
        self.adsr = s[AMP_ENV] > 0.5;
        let decay = decay_seconds(if self.adsr { s[AMP_D] } else { s[T_DECAY] });
        self.tone_env = Env::new(int(s, T_GATE, 0, 2), decay, sr, 0.0002);
        self.amp_adsr = Adsr::new(attack_seconds(s[AMP_A]), decay, sustain_gain(s[AMP_S]), decay_seconds(s[AMP_R]), sr);
        self.filter_adsr = Adsr::new(attack_seconds(s[FLT_A]), decay_seconds(s[FLT_D]), s[FLT_S].clamp(0.0, 99.0) / 99.0, decay_seconds(s[FLT_R]), sr);
        self.filter_amount = s[FLT_AMT].clamp(-10.0, 10.0) * AMOUNT_OCTAVES * vel_scale;
        self.tone_svf = Svf::default();
        self.tone_on = s[TONE] > 0.0;
        self.shape = wave.shape;
        self.count = wave.partials.len().min(MAX_PARTIALS);
        let (mut power, mut sum) = (0.0, 0.0);
        for (k, &(ratio, level, faster)) in wave.partials.iter().take(MAX_PARTIALS).enumerate() {
            self.ratios[k] = ratio;
            self.gains[k] = level;
            self.fades[k] = if faster > 0.0 { fall(decay / faster, sr) } else { 1.0 };
            power += level * level;
            sum += level;
        }
        // Between loudness (RMS) and peak: near-harmonic partials start in
        // phase and add up at the strike.
        self.norm = match wave.shape {
            Shape::Squares => 0.7 / power.sqrt(),
            _ => 1.0 / (0.35 * power.sqrt() + 0.65 * sum),
        };
        // A sawtooth starts at its zero crossing; everything else at 0.
        self.phases = [if wave.shape == Shape::Saw { 0.5 } else { 0.0 }; MAX_PARTIALS];
        self.mod_phase = 0.0;
        if let Shape::Fm { index, fade, .. } = wave.shape {
            self.fm_index = index;
            self.fm_fade = fall(decay / fade.max(0.01), sr);
        }
        self.tone_sweep = s[T_SWEEP].clamp(0.0, 10.0) * SWEEP_OCTAVES * vel_scale;
        self.tone_sweep_oct = 1.0;
        self.tone_sweep_mul = fall(decay * time_share(s[T_TIME]), sr);
        // Bend: positive values start above the pitch and fall to it.
        self.bend = s[BEND].clamp(-9.0, 30.0) * BEND_STEP * vel_scale;
        self.bend_mul = fall(decay * time_share(s[BEND_TIME]), sr);
        let punch = s[T_PUNCH].clamp(0.0, 9.0) / 9.0;
        self.punch = punch * vel * 1.2;
        self.punch_mul = fade(0.0015 + punch * 0.0055, sr);

        // Noise.
        let decay = decay_seconds(s[N_DECAY]);
        self.noise_env = Env::new(int(s, N_GATE, 0, 2), decay, sr, 0.0002);
        self.noise_svf = Svf::default();
        self.noise_on = s[NOISE] > 0.0;
        self.tilt = 0.0;
        let q = NOISE_Q[(int(s, N_TYPE, 1, 7) - 1) as usize];
        self.noise_k = 1.0 / q;
        self.noise_norm = 1.0 / q.max(1.0).sqrt();
        self.noise_sweep = s[N_SWEEP].clamp(0.0, 10.0) * SWEEP_OCTAVES * vel_scale;
        self.noise_sweep_oct = 1.0;
        self.noise_sweep_mul = fall(decay * time_share(s[N_TIME]), sr);

        // Click.
        let kind = int(s, C_TYPE, 0, 26) as usize;
        self.family = kind / 9;
        let band = kind % 9;
        self.click_env = Env::new(int(s, C_GATE, 0, 2), click_seconds(s[C_DECAY]), sr, 0.0);
        self.click_svf = Svf::default();
        self.click_on = s[CLICK] > 0.0;
        let q = if self.family == 0 { 1.4 } else { 0.8 };
        let hz = CLICK_HZ[band].min(0.4 * sr);
        self.click_g = prewarp(hz, sr);
        self.click_k = 1.0 / q;
        self.click_gain = ((0.5 * sr) / (hz / q)).sqrt().min(6.0) * 0.6;
        self.click_phase = 0.0;
        self.click_hz = TICK_HZ[band];
        self.chirp = 2.0;
        self.chirp_mul = fade(0.001, sr);
        self.attack_inc = (2.5 * hz).min(12_000.0) / sr;
        self.attack_left = if self.family == 1 { (1.0 / self.attack_inc) as u32 } else { 0 };
    }

    /// Lets go of the note: the ADSRs go to their release (the Nord Drum's
    /// decays carry on as they are).
    pub fn release(&mut self) {
        self.amp_adsr.release();
        self.filter_adsr.release();
        self.gate_left = 0;
    }

    fn tone_done(&self) -> bool {
        !self.tone_on || if self.adsr { self.amp_adsr.done() } else { self.tone_env.done() }
    }

    /// Tone, noise and click for one sample.
    #[inline]
    pub fn tick(&mut self, live: &Live, tilt: &Tilt) -> [f32; 3] {
        let sr = self.sr;
        let mut out = [0.0; 3];
        if self.gate_left != OPEN && self.gate_left > 0 {
            self.gate_left -= 1;
            if self.gate_left == 0 {
                self.release();
            }
        }

        if !self.tone_done() {
            let env = if self.adsr { self.amp_adsr.tick() } else { self.tone_env.tick() };
            // Pitch with the bend, as one exponent.
            let inc = 440.0 * exp2((live.pitch - 69.0 + self.bend) / 12.0) / sr;
            self.bend *= self.bend_mul;
            let mut x = 0.0;
            match self.shape {
                Shape::Sines | Shape::Clip(_) => {
                    for k in 0..self.count {
                        let step = inc * self.ratios[k];
                        if step < 0.48 {
                            x += self.gains[k] * sin_turns(self.phases[k]);
                        }
                        self.phases[k] = wrap(self.phases[k] + step);
                        self.gains[k] *= self.fades[k];
                    }
                    if let Shape::Clip(drive) = self.shape {
                        x = tanh(drive * x * self.norm) / tanh(drive) / self.norm;
                    }
                }
                Shape::Triangle => {
                    x = 1.0 - 4.0 * (wrap(self.phases[0] + 0.25) - 0.5).abs();
                    self.phases[0] = wrap(self.phases[0] + inc.min(0.48));
                }
                Shape::Saw => {
                    let dt = inc.min(0.48);
                    let t = self.phases[0];
                    x = 2.0 * t - 1.0 - blep(t, dt);
                    self.phases[0] = wrap(t + dt);
                }
                Shape::Squares => {
                    for k in 0..self.count {
                        let dt = inc * self.ratios[k];
                        if dt < 0.48 {
                            let t = self.phases[k];
                            let sq = if t < 0.5 { 1.0 } else { -1.0 } + blep(t, dt) - blep(wrap(t + 0.5), dt);
                            x += self.gains[k] * sq;
                            self.phases[k] = wrap(t + dt);
                        }
                        self.gains[k] *= self.fades[k];
                    }
                }
                Shape::Fm { ratio, .. } => {
                    let m = sin_turns(self.mod_phase) * self.fm_index;
                    x = sin_turns(self.phases[0] + m * 0.159_155);
                    self.phases[0] = wrap(self.phases[0] + inc.min(0.48));
                    self.mod_phase = wrap(self.mod_phase + (inc * ratio).min(0.48));
                    self.fm_index *= self.fm_fade;
                }
            }
            x *= self.norm;
            // The filter, opened by the sweep and moved by its ADSR (99,
            // unmoved: fully open).
            let mut open = self.tone_sweep * self.tone_sweep_oct;
            self.tone_sweep_oct *= self.tone_sweep_mul;
            if self.filter_amount != 0.0 {
                open += self.filter_amount * self.filter_adsr.tick();
            }
            let oct = filter_octaves(live.tone_filter) + open;
            if oct < 10.0 {
                let g = prewarp(20.0 * exp2(oct), sr);
                x = self.tone_svf.tick(x, g, 1.25).0;
            }
            // Punch: a short boost on the attack, its peaks rounded off.
            if self.punch > 1e-4 {
                x += self.punch * tanh(2.0 * x);
                self.punch *= self.punch_mul;
            }
            out[0] = x * env;
        }

        if self.noise_on && !self.noise_env.done() {
            let env = self.noise_env.tick();
            let white = self.rng.bipolar();
            self.tilt += (white - self.tilt) * tilt.k;
            let x = match tilt.mode {
                -1 => self.tilt * tilt.gain,
                1 => white - self.tilt,
                _ => white,
            };
            let open = self.noise_sweep * self.noise_sweep_oct;
            self.noise_sweep_oct *= self.noise_sweep_mul;
            let oct = (filter_octaves(live.noise_filter) + open).min(10.2);
            let g = prewarp(20.0 * exp2(oct), sr);
            let y = self.noise_svf.tick(x * 1.2, g, self.noise_k).0 * self.noise_norm;
            out[1] = y * env;
        }

        if self.click_on && !self.click_env.done() {
            let env = self.click_env.tick();
            let y = match self.family {
                0 | 1 => {
                    let (_, band) = self.click_svf.tick(self.rng.bipolar(), self.click_g, self.click_k);
                    let mut y = band * self.click_k * self.click_gain;
                    if self.attack_left > 0 {
                        self.attack_left -= 1;
                        y += 1.2 * sin_turns(self.click_phase);
                        self.click_phase += self.attack_inc;
                    }
                    y
                }
                _ => {
                    let y = sin_turns(self.click_phase) + 0.25 * sin_turns(2.0 * self.click_phase);
                    self.click_phase = wrap(self.click_phase + (self.click_hz * (1.0 + self.chirp) / sr).min(0.48));
                    self.chirp *= self.chirp_mul;
                    y
                }
            };
            out[2] = y * env;
        }

        self.active = !self.tone_done() || (self.noise_on && !self.noise_env.done()) || (self.click_on && !self.click_env.done());
        out
    }
}

/// How long a strike of `s` lasts, in seconds, its note held for `gate`
/// (components that are mixed in only).
pub fn hit_seconds(s: &Sound, gate: f32) -> f32 {
    use knob::*;
    let mut t: f32 = 0.0;
    if s[TONE] > 0.0 {
        t = t.max(if s[AMP_ENV] > 0.5 {
            adsr_seconds(attack_seconds(s[AMP_A]), decay_seconds(s[AMP_D]), sustain_gain(s[AMP_S]), decay_seconds(s[AMP_R]), gate)
        } else {
            env_seconds(int(s, T_GATE, 0, 2), decay_seconds(s[T_DECAY]))
        });
    }
    if s[NOISE] > 0.0 {
        t = t.max(env_seconds(int(s, N_GATE, 0, 2), decay_seconds(s[N_DECAY])));
    }
    if s[CLICK] > 0.0 {
        t = t.max(env_seconds(int(s, C_GATE, 0, 2), click_seconds(s[C_DECAY])));
    }
    t.clamp(0.02, 8.0)
}

/// A mixer setting (0…10) as a gain.
pub fn mix_gain(v: f32) -> f32 {
    (v.clamp(0.0, 10.0) / 10.0).powf(1.5)
}

/// The channel level (0…99) as a gain.
pub fn level_gain(v: f32) -> f32 {
    let x = v.clamp(0.0, 99.0) / 99.0;
    x * x
}

/// Velocity (0…1) as a gain.
pub fn velocity_gain(vel: f32) -> f32 {
    0.08 + 0.92 * vel.clamp(0.0, 1.0).powf(1.6)
}

/// Which components a channel lets through: tone, noise, click (Solo
/// Edit keeps one).
pub type Parts = [bool; 3];

pub const ALL_PARTS: Parts = [true; 3];

/// The mixer's gains for tone, noise and click, with the channel level.
fn mix_targets(s: &Sound, parts: Parts) -> [f32; 3] {
    let level = level_gain(s[knob::LEVEL]);
    std::array::from_fn(|i| if parts[i] { mix_gain(s[knob::TONE + i]) * level } else { 0.0 })
}

/// Retriggered, the old strike fades out this fast instead of stopping
/// dead (no click).
const TAIL_SECONDS: f32 = 0.002;

/// A channel: its strike, the fading tail of the previous one, and its
/// smoothed live settings and mixer.
pub struct Channel {
    voice: Voice,
    tail: Voice,
    tail_gain: f32,
    tail_step: f32,
    sr: f32,
    live: Live,
    mix: [f32; 3],
    /// Per block: the mixer's gains to glide to, and the noise's colour.
    want: [f32; 3],
    tilt: Tilt,
    /// Velocity gain of the strike (and of the tail).
    amp: f32,
    tail_amp: f32,
}

impl Channel {
    pub fn new(sr: f32, seed: u32, s: &Sound) -> Self {
        Channel {
            voice: Voice::new(sr, seed),
            tail: Voice::new(sr, seed ^ 0x5bd1_e995),
            tail_gain: 0.0,
            tail_step: 1.0 / (TAIL_SECONDS * sr).max(1.0),
            sr,
            live: Live::of(s),
            mix: [0.0; 3],
            want: [0.0; 3],
            tilt: Tilt::new(s[knob::N_COLOR], sr),
            amp: 0.0,
            tail_amp: 0.0,
        }
    }

    pub fn active(&self) -> bool {
        self.voice.active || self.tail_gain > 0.0
    }

    /// Once a block: the mixer's targets and the noise's colour.
    pub fn prepare(&mut self, s: &Sound, parts: Parts) {
        self.want = mix_targets(s, parts);
        self.tilt = Tilt::new(s[knob::N_COLOR], self.sr);
    }

    /// A strike, its note held for `gate` seconds (None: until
    /// [`Self::release`]).
    pub fn trigger(&mut self, s: &Sound, vel: f32, parts: Parts, gate: Option<f32>) {
        if self.voice.active {
            std::mem::swap(&mut self.voice, &mut self.tail);
            self.tail_gain = 1.0;
            self.tail_amp = self.amp;
        } else if self.tail_gain <= 0.0 {
            // Silent until now: the mixer starts where it is set, so the
            // first strike is not softened by the smoothing.
            self.mix = mix_targets(s, parts);
        }
        self.live = Live::of(s);
        self.amp = velocity_gain(vel);
        self.voice.trigger(s, vel, gate);
    }

    /// Lets go of the strike's note.
    pub fn release(&mut self) {
        self.voice.release();
    }

    /// One sample of the channel, after its mixer and level (see
    /// [`Self::prepare`]). `k`: the smoothing coefficient for live changes.
    #[inline]
    pub fn tick(&mut self, s: &Sound, k: f32) -> f32 {
        if !self.active() {
            return 0.0;
        }
        let target = Live::of(s);
        self.live.pitch += (target.pitch - self.live.pitch) * k;
        self.live.tone_filter += (target.tone_filter - self.live.tone_filter) * k;
        self.live.noise_filter += (target.noise_filter - self.live.noise_filter) * k;
        for (mix, want) in self.mix.iter_mut().zip(self.want) {
            *mix += (want - *mix) * k;
        }
        let mut y = 0.0;
        if self.voice.active {
            let [t, n, c] = self.voice.tick(&self.live, &self.tilt);
            y = (t * self.mix[0] + n * self.mix[1] + c * self.mix[2]) * self.amp;
        }
        if self.tail_gain > 0.0 {
            let [t, n, c] = self.tail.tick(&self.live, &self.tilt);
            y += (t * self.mix[0] + n * self.mix[1] + c * self.mix[2]) * self.tail_amp * self.tail_gain;
            self.tail_gain -= self.tail_step;
            if self.tail_gain <= 0.0 {
                self.tail_gain = 0.0;
                self.tail.active = false;
            }
        }
        y
    }
}

/// For the panel: one strike of `s` at velocity `vel`, its note held for
/// `gate` seconds, its three components after the mixer (tone, noise,
/// click), `n` samples at `sr`.
pub fn render(s: &Sound, vel: f32, gate: f32, sr: f32, n: usize) -> [Vec<f32>; 3] {
    let mut voice = Voice::new(sr, 0x0F10_2D00);
    voice.trigger(s, vel, Some(gate));
    let live = Live::of(s);
    let tilt = Tilt::new(s[knob::N_COLOR], sr);
    let level = level_gain(s[knob::LEVEL]) * velocity_gain(vel);
    let gains: [f32; 3] = std::array::from_fn(|i| mix_gain(s[knob::TONE + i]) * level);
    let mut out = [vec![0.0; n], vec![0.0; n], vec![0.0; n]];
    for i in 0..n {
        let parts = voice.tick(&live, &tilt);
        for ((buf, part), gain) in out.iter_mut().zip(parts).zip(gains) {
            buf[i] = part * gain;
        }
        if !voice.active {
            break;
        }
    }
    out
}
