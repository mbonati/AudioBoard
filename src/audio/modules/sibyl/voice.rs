//! One voice of Sibyl: the Prophet-5's signal path.
//!
//! - **Oscillators** (CEM3340): A has saw and pulse and hard-syncs to B; B
//!   has saw, triangle and pulse, LO FREQ (seven octaves down, an LFO per
//!   voice) and keyboard tracking. They run free (no phase reset on a note)
//!   and are band-limited: each discontinuity (a wrap, a pulse edge, a sync
//!   reset) is smoothed with a polynomial BLEP, the triangle's corners with a
//!   BLAMP, and since a step's correction reaches into the sample before it,
//!   each oscillator speaks one sample late, which makes sync and audio-rate
//!   FM exact. In the P5 models the pulse is inverted against the saw, as on
//!   the original: saw and pulse together hollow each other out.
//! - **Mixer**: A, B and the shared noise (or, with the feedback jumper, the
//!   voice's own output fed back).
//! - **Filter** (CEM3320): a four-pole low pass solved without delay
//!   (topology-preserving, the four stages and the resonance loop at once),
//!   saturating where the loop meets the input: driven, it thickens; past
//!   60 % resonance it sings on its own; resonance thins the bass as on the
//!   hardware. Its cutoff can move at audio rate (oscillator B's poly-mod).
//! - **Envelopes** (CEM3310): the attack charges towards 1.5 and stops at 1
//!   (the curve of the chip), decay and release fall exponentially, a new
//!   note restarts the attack from where the envelope is.
//! - **Amplifier**, then the optional distortion of the voice's own, then
//!   its place in the stereo field.
//!
//! The slow parts (envelopes, glide, pitches, matrix, base cutoff, levels)
//! are worked out once per segment by [`Voice::control`] and ramped across
//! it; [`Voice::render`] runs the per-sample path.

use super::mods::{self, Globals, Matrix, Mods, dest};

/// No offsets (the matrix has no live slot for the voices).
static ZERO_MODS: Mods = [0.0; dest::COUNT];
use super::{
    A_ATTACK, A_DECAY, A_FINE, A_FREQ, A_MODEL, A_OCT, A_PULSE, A_PW, A_RELEASE, A_SAW, A_SUSTAIN,
    A_SYNC, AENV_VEL, ARP_GATE, ARP_LATCH, ARP_MODE, ARP_OCT, ARP_ON, ARP_RATE, ARP_RATES,
    ARP_SWING, B_FINE, B_FREQ, B_INV, B_KBD, B_LO, B_MODEL, B_OCT, B_PULSE, B_PW, B_SAW, B_TRI,
    CC_A, CC_B, CUTOFF, DETUNE, DIST_AMOUNT, DIST_MIX, DIST_ON, DIST_TONE, DIST_TYPE, ENV_AMT,
    ENV_MODEL, F_ATTACK, F_DECAY, F_RELEASE, F_SUSTAIN, FENV_INV, FENV_VEL, FILTER_MODEL, GLIDE,
    KEY_AMT, KEY_PB, LFO_DC, LFO_DIV, LFO_DIVS, LFO_INV, LFO_RATE, LFO_SAW, LFO_SQR, LFO_SYNC,
    LFO_TRI, MAX_VOICES, MIX_A, MIX_B, MIX_FB, MIX_NOISE, ModuleParams, NOTE_PRIO, PAN,
    PARAM_COUNT, PB_DOWN, PB_UP, PM_FENV, PM_FILTER, PM_FREQ_A, PM_OSCB, PM_PW_A, REALLOC, RESO,
    Rng, TUNE, UNI_VOICES, UNISON, VOICES, VOLUME, WM_FILTER, WM_FREQ_A, WM_FREQ_B, WM_HIGH,
    WM_LOW, WM_MIX, WM_PW_A, WM_PW_B, WM_SH, bend_semitones,
};
use crate::audio::fastmath;
use audioboard_node::dsp::{Wander, pan_gains};

/// Envelope stages.
pub const IDLE: u8 = 0;
pub const ATTACK: u8 = 1;
pub const DECAY: u8 = 2;
pub const RELEASE: u8 = 3;
/// PIANO models: the true release after the key is let go.
pub const LET_GO: u8 = 4;

/// Envelope models (see [`super::ENV_MODELS`]).
pub const ENV_IDEAL: usize = 0;
#[allow(dead_code)]
pub const ENV_ANALOG: usize = 1;
pub const ENV_HIGH_SUS: usize = 2;
pub const ENV_ONE_SHOT: usize = 3;
pub const ENV_PIANO_1: usize = 4;
pub const ENV_PIANO_2: usize = 5;

/// Filter models (see [`super::FILTER_MODELS`]).
pub const CRISPY: usize = 0;
pub const ROUNDED: usize = 1;
pub const DRIVEN: usize = 2;
#[allow(dead_code)]
pub const POLY: usize = 3;

/// Oscillator models (see [`super::A_MODELS`], [`super::B_MODELS`]).
pub const OSC_P5: usize = 0;
#[allow(dead_code)]
pub const OSC_P1: usize = 1;
pub const OSC_IDEAL: usize = 2;
pub const OSC_P5_OLD: usize = 3;
pub const OSC_BOTTOM: usize = 4;

/// A voice at full level, before VOLUME: leaves room for a chord.
pub const VOICE_GAIN: f32 = 0.42;
/// An envelope this low has ended.
const ENV_END: f32 = 2e-5;
/// Semitones of LO FREQ.
const LO_FREQ: f32 = -84.0;
/// The cutoff knob's range in semitones (10 Hz to 20 kHz).
const CUTOFF_SPAN: f32 = 131.6;
/// Envelope amount and poly-mod at full, in semitones.
const ENV_SPAN: f32 = 96.0;
const PM_SPAN: f32 = 48.0;
/// Wheel-mod at full: pitch and cutoff, in semitones; pulse width.
const WM_PITCH: f32 = 12.0;
const WM_CUT: f32 = 48.0;

// ---------------------------------------------------------------------------
// Filter coefficients
// ---------------------------------------------------------------------------

/// The table starts at this pitch (MIDI note numbers, fractional) …
const T_LO: f32 = -36.0;
/// … with four entries a semitone.
const T_RES: f32 = 4.0;
const T_LEN: usize = 800;
pub const CUT_MIN: f32 = T_LO;
pub const CUT_MAX: f32 = T_LO + (T_LEN - 2) as f32 / T_RES;

/// The filter stages' gain G = g / (1 + g), g = tan(π f / sr), by cutoff
/// pitch: a lookup instead of a tangent per sample.
pub struct FilterTable {
    g: [f32; T_LEN],
}

impl FilterTable {
    pub fn new(sr: f32) -> Self {
        let g = std::array::from_fn(|i| {
            let pitch = T_LO + i as f32 / T_RES;
            let hz = (440.0 * ((pitch - 69.0) / 12.0).exp2()).min(sr * 0.47);
            let g = (std::f32::consts::PI * hz / sr).tan();
            g / (1.0 + g)
        });
        Self { g }
    }

    #[inline(always)]
    pub fn get(&self, pitch: f32) -> f32 {
        let x = ((pitch - T_LO) * T_RES).clamp(0.0, (T_LEN - 2) as f32);
        let i = (x as usize).min(T_LEN - 2);
        let f = x - i as f32;
        let a = self.g[i];
        a + (self.g[i + 1] - a) * f
    }
}

/// A tanh look-alike: exact at 0, 1 with a flat top from |x| = 3.
#[inline(always)]
pub fn sat(x: f32) -> f32 {
    let x = x.clamp(-3.0, 3.0);
    let x2 = x * x;
    x * (27.0 + x2) / (27.0 + 9.0 * x2)
}

/// Triangle folding into ±1.
#[inline(always)]
fn fold(x: f32) -> f32 {
    let t = (x + 1.0) * 0.25;
    1.0 - 4.0 * (t - t.floor() - 0.5).abs()
}

/// A step of height `h`, `d` of a sample ago: the polynomial BLEP's
/// corrections to the sample before and to this one.
#[inline(always)]
fn blep(h: f32, d: f32) -> (f32, f32) {
    let e = 1.0 - d;
    (h * d * d * 0.5, -h * e * e * 0.5)
}

/// A change of slope of `m` per sample, `d` of a sample ago (BLAMP).
#[inline(always)]
fn blamp(m: f32, d: f32) -> (f32, f32) {
    let e = 1.0 - d;
    (m * d * d * d * (1.0 / 6.0), m * e * e * e * (1.0 / 6.0))
}

/// 2^x for audio-rate FM: a cubic, within 0.02 % (a fifth of a cent),
/// cheaper than the general one.
#[inline(always)]
fn exp2_fm(x: f32) -> f32 {
    let x = x.clamp(-24.0, 24.0);
    let i = x.floor();
    let f = x - i;
    let p = 1.0 + f * (0.695_556_2 + f * (0.226_173_6 + f * 0.078_037_5));
    p * f32::from_bits(((i as i32 + 127) as u32) << 23)
}

/// Frequency of a pitch in MIDI note numbers, fast.
#[inline(always)]
fn hz(pitch: f32) -> f32 {
    440.0 * fastmath::exp2((pitch - 69.0) * (1.0 / 12.0))
}

// ---------------------------------------------------------------------------
// The patch, decoded once per block
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
pub struct EnvSet {
    pub attack: f32,
    pub decay: f32,
    pub sustain: f32,
    pub release: f32,
}

/// The filter model's character.
#[derive(Clone, Copy, Default)]
pub struct FilterModel {
    /// Cutoff shift, semitones.
    pub offset: f32,
    /// Into the saturating loop.
    pub drive: f32,
    /// Resonance scale.
    pub k_scale: f32,
    /// How much of the bass resonance takes away comes back.
    pub comp: f32,
    /// The highest the knob alone reaches (POLY: modulation goes further).
    pub knob_max: f32,
}

pub fn filter_model(m: usize) -> FilterModel {
    match m {
        CRISPY => FilterModel {
            offset: 0.0,
            drive: 0.8,
            k_scale: 1.0,
            comp: 0.3,
            knob_max: CUT_MAX,
        },
        ROUNDED => FilterModel {
            offset: -3.0,
            drive: 0.9,
            k_scale: 0.93,
            comp: 0.25,
            knob_max: CUT_MAX,
        },
        DRIVEN => FilterModel {
            offset: -1.0,
            drive: 1.9,
            k_scale: 1.0,
            comp: 0.45,
            knob_max: CUT_MAX,
        },
        _ => FilterModel {
            offset: -5.0,
            drive: 1.1,
            k_scale: 0.97,
            comp: 0.2,
            knob_max: 127.0,
        },
    }
}

/// Resonance (0..1) to loop gain: self-oscillation from 60 %.
#[inline]
pub fn loop_gain(res: f32, scale: f32) -> f32 {
    let r = res.clamp(0.0, 1.0);
    scale
        * if r < 0.6 {
            r * (4.0 / 0.6)
        } else {
            4.0 + (r - 0.6) * 3.0
        }
}

/// Every setting in the units the voices use.
#[derive(Clone, Default)]
pub struct Patch {
    pub vsr: f32,
    // Oscillator A.
    pub a_semi: f32,
    pub a_saw: f32,
    pub a_pul: f32,
    pub a_pw: f32,
    pub a_sync: bool,
    pub a_model: usize,
    // Oscillator B.
    pub b_semi: f32,
    pub b_saw: f32,
    pub b_tri: f32,
    pub b_pul: f32,
    pub b_pw: f32,
    pub b_lo: bool,
    pub b_kbd: bool,
    pub b_model: usize,
    // Mixer.
    pub mix_a: f32,
    pub mix_b: f32,
    pub mix_n: f32,
    pub feedback: bool,
    // Filter.
    pub cut: f32,
    pub res: f32,
    pub env_amt: f32,
    pub env_sign: f32,
    pub key_amt: f32,
    pub key_pb: bool,
    pub fenv_vel: f32,
    pub model: FilterModel,
    // Envelopes.
    pub fenv: EnvSet,
    pub aenv: EnvSet,
    /// −log2(e) / (time · rate) for attack, decay and release.
    pub fenv_k: [f32; 3],
    pub aenv_k: [f32; 3],
    pub aenv_vel: f32,
    pub env_model: usize,
    // Poly-mod.
    pub pm_fenv: f32,
    pub pm_oscb: f32,
    pub pm_freq_a: bool,
    pub pm_pw_a: bool,
    pub pm_filter: bool,
    // Wheel-mod and LFO.
    pub wm_mix: f32,
    pub wm_low: f32,
    pub wm_high: f32,
    pub wm_freq_a: bool,
    pub wm_freq_b: bool,
    pub wm_pw_a: bool,
    pub wm_pw_b: bool,
    pub wm_filter: bool,
    pub wm_sh: bool,
    pub lfo_rate: f32,
    pub lfo_sync: bool,
    pub lfo_beats: f64,
    pub lfo_saw: bool,
    pub lfo_tri: bool,
    pub lfo_sqr: bool,
    pub lfo_dc: bool,
    pub lfo_inv: bool,
    // Performance.
    pub unison: bool,
    pub glide: f32,
    pub detune: f32,
    pub voices: usize,
    pub uni_voices: usize,
    pub prio: usize,
    pub realloc: bool,
    pub tune: f32,
    pub volume: f32,
    pub bend_up: f32,
    pub bend_down: f32,
    pub pans: [f32; MAX_VOICES],
    // Distortion.
    pub dist_on: bool,
    pub dist_type: usize,
    pub dist_db: f32,
    pub dist_tone: f32,
    pub dist_mix: f32,
    // Arpeggiator.
    pub arp_on: bool,
    pub arp_mode: usize,
    pub arp_beats: f64,
    pub arp_oct: usize,
    pub arp_gate: f32,
    pub arp_swing: f32,
    pub arp_latch: bool,
    pub cc_a: f32,
    pub cc_b: f32,
}

impl Patch {
    pub fn update(&mut self, s: &[f32; PARAM_COUNT], _p: &ModuleParams, vsr: f32) {
        let on = |k: usize| s[k] > 0.5;
        let idx = |k: usize| s[k].round().max(0.0) as usize;
        self.vsr = vsr;
        self.a_model = idx(A_MODEL).min(3);
        self.b_model = idx(B_MODEL).min(4);
        // P5: the pulse is upside down against the saw.
        let a_pinv = if matches!(self.a_model, OSC_P5 | OSC_P5_OLD) {
            -1.0
        } else {
            1.0
        };
        let b_pinv = if matches!(self.b_model, OSC_P5 | OSC_P5_OLD | OSC_BOTTOM) {
            -1.0
        } else {
            1.0
        };
        self.a_semi = (s[A_OCT].round() - 1.0) * 12.0 + s[A_FREQ].round() + s[A_FINE] * 0.01;
        self.a_saw = if on(A_SAW) { 1.0 } else { 0.0 };
        self.a_pul = if on(A_PULSE) { a_pinv } else { 0.0 };
        self.a_pw = s[A_PW];
        self.a_sync = on(A_SYNC);
        self.b_semi = (s[B_OCT].round() - 1.0) * 12.0 + s[B_FREQ].round() + s[B_FINE] * 0.01;
        self.b_saw = if on(B_SAW) {
            if on(B_INV) { -1.0 } else { 1.0 }
        } else {
            0.0
        };
        self.b_tri = if on(B_TRI) {
            if self.b_model == OSC_BOTTOM { 1.6 } else { 1.0 }
        } else {
            0.0
        };
        self.b_pul = if on(B_PULSE) { b_pinv } else { 0.0 };
        self.b_pw = s[B_PW];
        self.b_lo = on(B_LO);
        self.b_kbd = on(B_KBD);
        self.mix_a = s[MIX_A];
        self.mix_b = s[MIX_B];
        self.mix_n = s[MIX_NOISE];
        self.feedback = on(MIX_FB);
        self.model = filter_model(idx(FILTER_MODEL));
        self.cut = (69.0 + 12.0 * (s[CUTOFF].max(1.0) / 440.0).log2()).min(self.model.knob_max);
        self.res = s[RESO];
        self.env_amt = s[ENV_AMT];
        self.env_sign = if on(FENV_INV) { -1.0 } else { 1.0 };
        self.key_amt = s[KEY_AMT];
        self.key_pb = on(KEY_PB);
        self.fenv_vel = s[FENV_VEL];
        self.fenv = EnvSet {
            attack: s[F_ATTACK],
            decay: s[F_DECAY],
            sustain: s[F_SUSTAIN],
            release: s[F_RELEASE],
        };
        self.aenv = EnvSet {
            attack: s[A_ATTACK],
            decay: s[A_DECAY],
            sustain: s[A_SUSTAIN],
            release: s[A_RELEASE],
        };
        let k = |set: &EnvSet| {
            let c = -std::f32::consts::LOG2_E / vsr;
            [
                c / set.attack.clamp(0.0005, 30.0),
                c / set.decay.clamp(0.001, 30.0),
                c / set.release.clamp(0.001, 110.0),
            ]
        };
        self.fenv_k = k(&self.fenv);
        self.aenv_k = k(&self.aenv);
        self.aenv_vel = s[AENV_VEL];
        self.env_model = idx(ENV_MODEL).min(5);
        self.pm_fenv = s[PM_FENV];
        self.pm_oscb = s[PM_OSCB];
        self.pm_freq_a = on(PM_FREQ_A);
        self.pm_pw_a = on(PM_PW_A);
        self.pm_filter = on(PM_FILTER);
        self.wm_mix = s[WM_MIX];
        self.wm_low = s[WM_LOW];
        self.wm_high = s[WM_HIGH];
        self.wm_freq_a = on(WM_FREQ_A);
        self.wm_freq_b = on(WM_FREQ_B);
        self.wm_pw_a = on(WM_PW_A);
        self.wm_pw_b = on(WM_PW_B);
        self.wm_filter = on(WM_FILTER);
        self.wm_sh = on(WM_SH);
        self.lfo_rate = s[LFO_RATE];
        self.lfo_sync = on(LFO_SYNC);
        self.lfo_beats = LFO_DIVS[idx(LFO_DIV).min(LFO_DIVS.len() - 1)].1;
        self.lfo_saw = on(LFO_SAW);
        self.lfo_tri = on(LFO_TRI);
        self.lfo_sqr = on(LFO_SQR);
        self.lfo_dc = on(LFO_DC);
        self.lfo_inv = on(LFO_INV);
        self.unison = on(UNISON);
        self.glide = glide_seconds(s[GLIDE]);
        self.detune = s[DETUNE];
        self.voices = idx(VOICES).clamp(1, MAX_VOICES);
        self.uni_voices = idx(UNI_VOICES).clamp(1, MAX_VOICES);
        self.prio = idx(NOTE_PRIO).min(2);
        self.realloc = on(REALLOC);
        self.tune = s[TUNE];
        self.volume = s[VOLUME];
        self.bend_up = bend_semitones(s[PB_UP]);
        self.bend_down = bend_semitones(s[PB_DOWN]);
        for (v, pan) in self.pans.iter_mut().enumerate() {
            *pan = s[PAN + v];
        }
        self.dist_on = on(DIST_ON);
        self.dist_type = idx(DIST_TYPE).min(3);
        self.dist_db = s[DIST_AMOUNT];
        self.dist_tone = s[DIST_TONE];
        self.dist_mix = s[DIST_MIX];
        self.arp_on = on(ARP_ON);
        self.arp_mode = idx(ARP_MODE);
        self.arp_beats = ARP_RATES[idx(ARP_RATE).min(ARP_RATES.len() - 1)].1;
        self.arp_oct = idx(ARP_OCT).clamp(1, 4);
        self.arp_gate = s[ARP_GATE];
        self.arp_swing = s[ARP_SWING];
        self.arp_latch = on(ARP_LATCH);
        self.cc_a = s[CC_A].round();
        self.cc_b = s[CC_B].round();
    }
}

/// GLIDE (0..1) in seconds: the slew's time constant.
pub fn glide_seconds(g: f32) -> f32 {
    if g <= 0.001 { 0.0 } else { 0.002 + 2.5 * g * g }
}

// ---------------------------------------------------------------------------
// Envelope
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Default)]
pub struct Env {
    pub level: f32,
    pub stage: u8,
    /// PIANO models: the key went up while the envelope still ran.
    pub key_up: bool,
}

/// One segment's envelope settings, in samples.
#[derive(Clone, Copy)]
struct EnvRun {
    /// −log2(e) / τ for each stage.
    a: f32,
    d: f32,
    r: f32,
    sustain: f32,
    /// Where the attack aims (it stops at 1).
    peak: f32,
    model: usize,
}

impl Env {
    fn advance(&mut self, e: &EnvRun, len: f32) {
        match self.stage {
            ATTACK => {
                self.level = e.peak + (self.level - e.peak) * fastmath::exp2(e.a * len);
                // HIGH SUS: a high sustain is never quite reached, the top jumps.
                if e.model == ENV_HIGH_SUS && e.sustain > 0.85 && self.level >= 0.85 {
                    self.level = 1.0;
                }
                if self.level >= 1.0 {
                    self.level = 1.0;
                    self.stage = DECAY;
                }
            }
            DECAY => {
                self.level = e.sustain + (self.level - e.sustain) * fastmath::exp2(e.d * len);
                let one_shot = matches!(e.model, ENV_ONE_SHOT | ENV_PIANO_1 | ENV_PIANO_2);
                if one_shot && (self.level - e.sustain).abs() < 0.01 {
                    self.stage = if self.key_up && e.model != ENV_ONE_SHOT {
                        LET_GO
                    } else {
                        RELEASE
                    };
                }
            }
            RELEASE => {
                self.level *= fastmath::exp2(e.r * len);
                self.end();
            }
            LET_GO => {
                // The piano's damper: a release several times faster.
                let k = if e.model == ENV_PIANO_1 { 2.5 } else { 7.0 };
                self.level *= fastmath::exp2(e.r * k * len);
                self.end();
            }
            _ => {}
        }
    }

    fn end(&mut self) {
        if self.level < ENV_END {
            self.level = 0.0;
            self.stage = IDLE;
        }
    }

    fn gate_on(&mut self) {
        self.stage = ATTACK;
        self.key_up = false;
    }

    fn gate_off(&mut self, model: usize) {
        match model {
            ENV_ONE_SHOT => {}
            ENV_PIANO_1 | ENV_PIANO_2 => {
                self.key_up = true;
                if self.stage == RELEASE {
                    self.stage = LET_GO;
                }
            }
            _ => {
                if self.stage != IDLE {
                    self.stage = RELEASE;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The voice
// ---------------------------------------------------------------------------

/// What the per-sample path ramps across a segment.
#[derive(Clone, Copy, Default)]
struct Ramp {
    inc_a: f32,
    inc_b: f32,
    pw_a: f32,
    pw_b: f32,
    /// Oscillator B → A's pitch, octaves per unit.
    fm: f32,
    /// Oscillator B → A's pulse width.
    pm_pw: f32,
    /// Oscillator B → cutoff, semitones per unit.
    pm_cut: f32,
    cut: f32,
    k: f32,
    la: f32,
    lb: f32,
    ln: f32,
    amp: f32,
    pl: f32,
    pr: f32,
    dgain: f32,
    dmix: f32,
    /// The distortion's TONE, −1..1.
    dtone: f32,
}

/// How this voice differs from the others (as real voice cards do), each
/// −1..1, scaled by VOICE DETUNE and the models.
#[derive(Clone, Copy)]
struct Character {
    a: f32,
    b: f32,
    cut: f32,
    res: f32,
    pw: f32,
    env: f32,
    gain: f32,
    peak: f32,
    /// −1..1, the Voice Detune matrix source.
    random: f32,
    /// This voice's attack target in the non-ideal models, and its log.
    peak_ln: (f32, f32),
    drift_rate: [f32; 2],
}

pub struct Voice {
    pub index: usize,
    pub note: u8,
    /// Its key (or the pedal) holds it.
    pub gate: bool,
    pub velocity: f32,
    /// Allocation order.
    pub started: u64,
    /// Place in the unison stack, −1..1.
    pub unison: f32,
    pitch: f32,
    target: f32,
    /// Starting from silence: no ramps from stale values.
    fresh: bool,
    // Oscillators: phases and the late sample each holds.
    pa: f32,
    pb: f32,
    ya: f32,
    yb: f32,
    // Filter states, and the output fed back.
    z: [f32; 4],
    last: f32,
    pub fenv: Env,
    pub aenv: Env,
    cur: Ramp,
    next: Ramp,
    character: Character,
    drift: [Wander; 2],
    rng: Rng,
    pub mods: mods::VoiceState,
    /// Seconds since the note started (the Trigger source).
    pub since: f32,
    /// VOICE DETUNE and the bend's range, as last seen (for the matrix).
    detune_amt: f32,
    bend_semis: f32,
    /// The pan last worked out, and its gains.
    pan: (f32, f32, f32),
    /// The cutoff at the start of the segment being rendered.
    cut_from: f32,
    // Distortion: tilt filter, sample-rate reducer.
    tilt: f32,
    hold: f32,
    hold_phase: f32,
}

impl Voice {
    pub fn new(index: usize, rng: &mut Rng) -> Self {
        let mut own = Rng::new(rng.next_u32());
        let character = Character {
            a: own.bipolar(),
            b: own.bipolar(),
            cut: own.bipolar(),
            res: own.bipolar(),
            pw: own.bipolar(),
            env: own.bipolar(),
            gain: own.bipolar(),
            peak: own.bipolar(),
            random: own.bipolar(),
            peak_ln: (0.0, 0.0),
            drift_rate: [0.07 + 0.25 * own.unit(), 0.07 + 0.25 * own.unit()],
        };
        let mut character = character;
        let peak = 1.5 + 0.12 * character.peak;
        character.peak_ln = (peak, (peak / (peak - 1.0)).ln());
        let drift = [Wander::new(&mut own), Wander::new(&mut own)];
        Self {
            index,
            note: 60,
            gate: false,
            velocity: 0.0,
            started: 0,
            unison: 0.0,
            pitch: 60.0,
            target: 60.0,
            fresh: true,
            pa: own.unit(),
            pb: own.unit(),
            ya: 0.0,
            yb: 0.0,
            z: [0.0; 4],
            last: 0.0,
            fenv: Env::default(),
            aenv: Env::default(),
            cur: Ramp::default(),
            next: Ramp::default(),
            character,
            drift,
            rng: own,
            mods: mods::VoiceState::default(),
            since: 0.0,
            detune_amt: 0.0,
            bend_semis: 2.0,
            pan: (f32::NAN, 0.0, 0.0),
            cut_from: 60.0,
            tilt: 0.0,
            hold: 0.0,
            hold_phase: 0.0,
        }
    }

    /// Sounding (its amplifier envelope runs).
    #[inline]
    pub fn active(&self) -> bool {
        self.aenv.stage != IDLE
    }

    pub fn level(&self) -> f32 {
        self.aenv.level
    }

    /// The pitch it plays now (after glide).
    pub fn pitch(&self) -> f32 {
        self.pitch
    }

    /// The Voice Detune source: this voice's own offset, −1..1, times VOICE DETUNE.
    pub fn detune_source(&self) -> f32 {
        self.character.random * self.detune_amt
    }

    /// Semitones of bend up (for the Key + Bend source).
    pub fn bend_range(&self) -> f32 {
        self.bend_semis
    }

    /// A note starts: envelopes from where they are, the pitch glides there
    /// if GLIDE is up and the voice was sounding.
    pub fn trigger(&mut self, note: u8, vel: f32, clock: u64, patch: &Patch) {
        let was = self.active();
        self.note = note;
        self.target = note as f32;
        if !was || patch.glide <= 0.0 {
            self.pitch = self.target;
        }
        if !was {
            self.fresh = true;
            self.fenv.level = 0.0;
            self.aenv.level = 0.0;
            self.mods.reset();
        }
        self.velocity = vel;
        self.started = clock;
        self.gate = true;
        self.since = 0.0;
        self.fenv.gate_on();
        self.aenv.gate_on();
    }

    /// Unison going back to a held key: only the pitch moves.
    pub fn glide_to(&mut self, note: u8, patch: &Patch) {
        self.note = note;
        self.target = note as f32;
        if patch.glide <= 0.0 {
            self.pitch = self.target;
        }
    }

    pub fn release(&mut self, patch: &Patch) {
        self.gate = false;
        self.fenv.gate_off(patch.env_model);
        self.aenv.gate_off(patch.env_model);
    }

    /// Silence at once (panic, all sound off).
    pub fn kill(&mut self) {
        self.gate = false;
        self.fenv = Env::default();
        self.aenv = Env::default();
        self.reset_filter();
    }

    pub fn reset_filter(&mut self) {
        self.z = [0.0; 4];
        self.last = 0.0;
        self.ya = 0.0;
        self.yb = 0.0;
    }

    /// One envelope's run for this segment: its stages' coefficients for this
    /// voice (its timing `inv_scale`, matrix offsets `m`).
    fn env_run(
        &self,
        set: &EnvSet,
        base: &[f32; 3],
        patch: &Patch,
        inv_scale: f32,
        m: [f32; 4],
    ) -> EnvRun {
        let model = patch.env_model;
        // Where the attack aims, and ln(peak / (peak − 1)): worked out once.
        let (peak, ln_peak) = if model == ENV_IDEAL {
            (1.5, std::f32::consts::LN_2 + 0.405_465_1)
        } else {
            self.character.peak_ln
        };
        let sustain = (set.sustain + m[2]).clamp(0.0, 1.0);
        if m[0] == 0.0 && m[1] == 0.0 && m[3] == 0.0 {
            return EnvRun {
                a: base[0] * ln_peak * inv_scale,
                d: base[1] * 3.0 * inv_scale,
                r: base[2] * 3.0 * inv_scale,
                sustain,
                peak,
                model,
            };
        }
        // Matrix offsets move the knobs' positions (log tapers: 2 ms to 15 or 55 s).
        let sr = patch.vsr;
        let at = |t: f32, max: f32, off: f32| {
            if off == 0.0 {
                t
            } else {
                t * fastmath::exp2(off * (max / 0.002).log2())
            }
        };
        let attack = at(set.attack, 15.0, m[0]).clamp(0.0005, 30.0);
        let decay = at(set.decay, 15.0, m[1]).clamp(0.001, 30.0);
        let release = at(set.release, 55.0, m[3]).clamp(0.001, 110.0);
        let k = -std::f32::consts::LOG2_E * inv_scale;
        EnvRun {
            a: k * ln_peak / (attack * sr),
            d: k * 3.0 / (decay * sr),
            r: k * 3.0 / (release * sr),
            sustain,
            peak,
            model,
        }
    }

    /// The slow part, once per segment of `len` samples: envelopes, glide,
    /// the matrix, pitches and levels; sets the ramps towards them.
    #[inline(never)]
    pub fn control(&mut self, patch: &Patch, matrix: &Matrix, g: &Globals, len: f32) {
        let sr = patch.vsr;
        let dt = len / sr;
        let ch = self.character;
        let det = patch.detune;
        self.detune_amt = det;
        self.bend_semis = if g.bend >= 0.0 {
            patch.bend_up
        } else {
            patch.bend_down
        };
        let mut live = [0.0f32; dest::COUNT];
        let m: &Mods = if matrix.voices_live() {
            matrix.voice(self, g, dt, &mut live);
            &live
        } else {
            &ZERO_MODS
        };

        // Envelopes (each voice's timing a little off, unless IDEAL).
        let inv_scale = if patch.env_model == ENV_IDEAL {
            1.0
        } else {
            1.0 / (1.0 + ch.env * (0.04 + 0.16 * det))
        };
        let fr = self.env_run(
            &patch.fenv,
            &patch.fenv_k,
            patch,
            inv_scale,
            [
                m[dest::F_ATTACK],
                m[dest::F_DECAY],
                m[dest::F_SUSTAIN],
                m[dest::F_RELEASE],
            ],
        );
        let ar = self.env_run(
            &patch.aenv,
            &patch.aenv_k,
            patch,
            inv_scale,
            [
                m[dest::A_ATTACK],
                m[dest::A_DECAY],
                m[dest::A_SUSTAIN],
                m[dest::A_RELEASE],
            ],
        );
        self.fenv.advance(&fr, len);
        self.aenv.advance(&ar, len);
        self.since += dt;

        // Glide.
        let glide = if m[dest::GLIDE] == 0.0 {
            patch.glide
        } else {
            glide_seconds((glide_knob(patch.glide) + m[dest::GLIDE]).clamp(0.0, 1.0))
        };
        if glide > 0.0 && self.pitch != self.target {
            self.pitch += (self.target - self.pitch)
                * (1.0 - fastmath::exp2(-std::f32::consts::LOG2_E * dt / glide));
            if (self.pitch - self.target).abs() < 1e-4 {
                self.pitch = self.target;
            }
        } else {
            self.pitch = self.target;
        }

        // Drift: slow wandering of each oscillator, the old models more.
        let osc_scale = |model: usize| match model {
            OSC_IDEAL => 0.0,
            OSC_P5_OLD => 3.0,
            _ => 1.0,
        };
        let drift_cents = 1.2 + 5.0 * det;
        let wa = self.drift[0].tick(&mut self.rng, dt * ch.drift_rate[0]);
        let wb = self.drift[1].tick(&mut self.rng, dt * ch.drift_rate[1]);
        let drift_a = wa * drift_cents * 0.01 * osc_scale(patch.a_model);
        let drift_b = wb * drift_cents * 0.01 * osc_scale(patch.b_model);

        let fe = self.fenv.level
            * (1.0 - (patch.fenv_vel + m[dest::F_VEL]).clamp(0.0, 1.0) * (1.0 - self.velocity));
        let bend = if g.bend >= 0.0 {
            g.bend * patch.bend_up
        } else {
            g.bend * patch.bend_down
        };
        let spread = self.unison * det * 0.3;
        let base = self.pitch + patch.tune + bend + m[dest::PITCH] * 12.0 + spread;

        // Oscillator A.
        let pm_fenv = (patch.pm_fenv + m[dest::PM_FENV]).clamp(0.0, 1.0);
        let pm_oscb = (patch.pm_oscb + m[dest::PM_OSCB]).clamp(0.0, 1.0);
        let wm = g.wm;
        let mut a_pitch =
            base + patch.a_semi + ch.a * det * 0.22 * osc_scale(patch.a_model).max(0.3) + drift_a;
        a_pitch += m[dest::A_FREQ] * 24.0 + m[dest::A_FINE] * 0.4;
        if patch.pm_freq_a {
            a_pitch += fe * pm_fenv * PM_SPAN;
        }
        if patch.wm_freq_a {
            a_pitch += wm * WM_PITCH;
        }
        // Oscillator B.
        let b_note = if patch.b_kbd {
            base
        } else {
            48.0 + patch.tune + bend
        };
        let mut b_pitch =
            b_note + patch.b_semi + ch.b * det * 0.22 * osc_scale(patch.b_model).max(0.3) + drift_b;
        b_pitch += m[dest::B_FREQ] * 24.0 + m[dest::B_FINE] * 2.0;
        if patch.wm_freq_b {
            b_pitch += wm * WM_PITCH;
        }
        if patch.b_lo {
            b_pitch += LO_FREQ;
        }
        let nyq = 0.45;
        let mut t = Ramp {
            inc_a: (hz(a_pitch) / sr).min(nyq),
            inc_b: (hz(b_pitch) / sr).min(nyq),
            ..Ramp::default()
        };
        let mut pw_a = patch.a_pw + ch.pw * det * 0.05 + m[dest::A_PW];
        if patch.pm_pw_a {
            pw_a += fe * pm_fenv * 0.5;
        }
        if patch.wm_pw_a {
            pw_a += wm * 0.5;
        }
        let mut pw_b = patch.b_pw - ch.pw * det * 0.05 + m[dest::B_PW];
        if patch.wm_pw_b {
            pw_b += wm * 0.5;
        }
        t.pw_a = pw_a.clamp(0.0, 1.0);
        t.pw_b = pw_b.clamp(0.0, 1.0);
        t.fm = if patch.pm_freq_a {
            pm_oscb * pm_oscb * 4.0
        } else {
            0.0
        };
        t.pm_pw = if patch.pm_pw_a { pm_oscb * 0.5 } else { 0.0 };
        t.pm_cut = if patch.pm_filter {
            pm_oscb * pm_oscb * PM_SPAN
        } else {
            0.0
        };

        // Filter.
        let key_note = if patch.key_pb {
            self.pitch + bend
        } else {
            self.pitch
        };
        let key_amt = (patch.key_amt + m[dest::KEY_AMT] * 1.33).max(0.0);
        let env_amt =
            (patch.env_amt + m[dest::ENV_AMT]).clamp(0.0, 1.0) * ENV_SPAN * patch.env_sign;
        let mut cut = patch.cut
            + patch.model.offset
            + ch.cut * det * 3.0
            + key_amt * (key_note - 60.0)
            + env_amt * fe;
        cut += m[dest::CUTOFF] * CUTOFF_SPAN;
        if patch.pm_filter {
            cut += fe * pm_fenv * PM_SPAN;
        }
        if patch.wm_filter {
            cut += wm * WM_CUT;
        }
        t.cut = cut.clamp(CUT_MIN, CUT_MAX);
        t.k = loop_gain(
            patch.res + m[dest::RESO] + ch.res * det * 0.03,
            patch.model.k_scale,
        );

        // Mixer.
        t.la = (patch.mix_a + m[dest::MIX_A]).clamp(0.0, 1.0);
        t.lb = (patch.mix_b + m[dest::MIX_B]).clamp(0.0, 1.0);
        t.ln = (patch.mix_n + m[dest::MIX_NOISE]).clamp(0.0, 1.0);

        // Amplifier: envelope, velocity, VOLUME, the stack's share.
        let vel = 1.0 - (patch.aenv_vel + m[dest::A_VEL]).clamp(0.0, 1.0) * (1.0 - self.velocity);
        let stack = if patch.unison && patch.uni_voices > 1 {
            (1.8 / (patch.uni_voices as f32).sqrt()).min(1.0)
        } else {
            1.0
        };
        let volume = (patch.volume + m[dest::VOLUME]).clamp(0.0, 1.0);
        let gain = VOICE_GAIN * (1.0 + ch.gain * det * 0.1) * stack * volume * volume;
        t.amp = self.aenv.level * vel * gain;
        let pan = (patch.pans[self.index] + m[dest::PAN] * 2.0).clamp(-1.0, 1.0);
        if pan != self.pan.0 {
            let (pl, pr) = pan_gains(pan);
            self.pan = (pan, pl, pr);
        }
        t.pl = self.pan.1;
        t.pr = self.pan.2;

        // Distortion.
        if patch.dist_on {
            let db = (patch.dist_db + m[dest::DIST_AMOUNT] * 60.0).clamp(0.0, 60.0);
            t.dgain = fastmath::exp2(db * (std::f32::consts::LOG2_10 / 20.0));
        }
        t.dmix = (patch.dist_mix + m[dest::DIST_MIX]).clamp(0.0, 1.0);
        t.dtone = (patch.dist_tone + m[dest::DIST_TONE] * 2.0).clamp(-1.0, 1.0);
        self.next = t;
        if self.fresh {
            // From silence: start right there (the amplifier still ramps up from 0).
            self.cur = Ramp { amp: 0.0, ..t };
            self.fresh = false;
        }
    }

    /// The oscillators and the mixer over a segment of `len` samples:
    /// writes the mix (A and B; noise and feedback are added by the filter)
    /// and the cutoff, sample by sample, for [`filter_group`].
    #[inline(never)]
    pub fn osc(
        &mut self,
        patch: &Patch,
        len: usize,
        lane: usize,
        pre: &mut LaneBuf,
        cut: &mut LaneBuf,
    ) {
        let inv = 1.0 / len as f32;
        let (cur, next) = (self.cur, self.next);
        let (mut inc_a, mut inc_b, mut pw_a, mut pw_b, mut c) =
            (cur.inc_a, cur.inc_b, cur.pw_a, cur.pw_b, cur.cut);
        let (d_inc_a, d_inc_b) = ((next.inc_a - inc_a) * inv, (next.inc_b - inc_b) * inv);
        let (d_pw_a, d_pw_b, d_cut) = (
            (next.pw_a - pw_a) * inv,
            (next.pw_b - pw_b) * inv,
            (next.cut - c) * inv,
        );
        // The filter's drive is applied here, to the mix.
        let drive = patch.model.drive * 0.5;
        let (fm, pm_pw, pm_cut, la, lb) = (
            next.fm,
            next.pm_pw,
            next.pm_cut,
            next.la * drive,
            next.lb * drive,
        );
        self.cut_from = c;
        let (a_saw, a_pul, b_saw, b_tri, b_pul) = (
            patch.a_saw,
            patch.a_pul,
            patch.b_saw,
            patch.b_tri,
            patch.b_pul,
        );
        let sync = patch.a_sync;
        let (mut pa, mut pb, mut ya, mut yb) = (self.pa, self.pb, self.ya, self.yb);

        for i in 0..len.min(SEG) {
            // --- Oscillator B (saw, triangle, pulse) ---
            let ib = inc_b;
            let pwb = pw_b;
            let p0 = pb;
            let p1raw = p0 + ib;
            let mut p1 = p1raw;
            let (mut prev_b, mut cur_b) = (0.0f32, 0.0f32);
            let mut b_wrapped = false;
            let mut db = 0.0f32;
            if p1raw >= 1.0 {
                p1 -= 1.0;
                b_wrapped = true;
                db = (p1 / ib).min(1.0);
                // Saw falls by 2, pulse rises (if it is ever high), the
                // triangle turns down.
                let h = -2.0 * b_saw + if pwb > 0.0 { 2.0 * b_pul } else { 0.0 };
                let (x, y) = blep(h, db);
                let (u, w) = blamp(-8.0 * ib * b_tri, db);
                prev_b += x + u;
                cur_b += y + w;
            }
            if b_pul != 0.0 {
                if p0 < pwb && pwb <= p1raw.min(1.0) {
                    let (x, y) = blep(-2.0 * b_pul, ((p1raw - pwb) / ib).min(1.0));
                    prev_b += x;
                    cur_b += y;
                } else if b_wrapped && pwb <= p1 {
                    let (x, y) = blep(-2.0 * b_pul, ((p1 - pwb) / ib).min(1.0));
                    prev_b += x;
                    cur_b += y;
                }
            }
            if b_tri != 0.0 && p0 < 0.5 && p1raw >= 0.5 {
                let (x, y) = blamp(8.0 * ib * b_tri, ((p1raw - 0.5) / ib).min(1.0));
                prev_b += x;
                cur_b += y;
            }
            let pul_b = if p1 < pwb { 1.0 } else { -1.0 };
            let naive_b =
                b_saw * (2.0 * p1 - 1.0) + b_tri * (4.0 * (p1 - 0.5).abs() - 1.0) + b_pul * pul_b;
            let out_b = yb + prev_b;
            yb = naive_b + cur_b;
            pb = p1;

            // --- Oscillator A (saw, pulse), FM'd and synced by B ---
            let ia = if fm != 0.0 {
                (inc_a * exp2_fm(fm * out_b)).min(0.45)
            } else {
                inc_a
            };
            let pwa = (pw_a + pm_pw * out_b).clamp(0.0, 1.0);
            let q0 = pa;
            let (mut prev_a, mut cur_a) = (0.0f32, 0.0f32);
            let q1 = if sync && b_wrapped {
                // B wrapped `db` of a sample ago: A restarts there, after
                // whatever it did itself before that.
                let t = 1.0 - db;
                let mut qa = q0 + ia * t;
                if qa >= 1.0 {
                    qa -= 1.0;
                    let dw = (1.0 - (1.0 - q0) / ia.max(1e-9)).clamp(0.0, 1.0);
                    let (x, y) = blep(-2.0 * a_saw + if pwa > 0.0 { 2.0 * a_pul } else { 0.0 }, dw);
                    prev_a += x;
                    cur_a += y;
                } else if a_pul != 0.0 && q0 < pwa && pwa <= qa {
                    let (x, y) = blep(
                        -2.0 * a_pul,
                        (1.0 - (pwa - q0) / ia.max(1e-9)).clamp(0.0, 1.0),
                    );
                    prev_a += x;
                    cur_a += y;
                }
                let before = a_saw * (2.0 * qa - 1.0) + a_pul * if qa < pwa { 1.0 } else { -1.0 };
                let after = -a_saw + a_pul * if pwa > 0.0 { 1.0 } else { -1.0 };
                let (x, y) = blep(after - before, db);
                prev_a += x;
                cur_a += y;
                let q = db * ia;
                if a_pul != 0.0 && pwa <= q {
                    let (x, y) = blep(-2.0 * a_pul, ((q - pwa) / ia.max(1e-9)).min(1.0));
                    prev_a += x;
                    cur_a += y;
                }
                q
            } else {
                let q1raw = q0 + ia;
                let mut q = q1raw;
                let mut wrapped = false;
                if q1raw >= 1.0 {
                    q -= 1.0;
                    wrapped = true;
                    let da = (q / ia).min(1.0);
                    let (x, y) = blep(-2.0 * a_saw + if pwa > 0.0 { 2.0 * a_pul } else { 0.0 }, da);
                    prev_a += x;
                    cur_a += y;
                }
                if a_pul != 0.0 {
                    if q0 < pwa && pwa <= q1raw.min(1.0) {
                        let (x, y) = blep(-2.0 * a_pul, ((q1raw - pwa) / ia).min(1.0));
                        prev_a += x;
                        cur_a += y;
                    } else if wrapped && pwa <= q {
                        let (x, y) = blep(-2.0 * a_pul, ((q - pwa) / ia).min(1.0));
                        prev_a += x;
                        cur_a += y;
                    }
                }
                q
            };
            let pul_a = if q1 < pwa { 1.0 } else { -1.0 };
            let naive_a = a_saw * (2.0 * q1 - 1.0) + a_pul * pul_a;
            let out_a = ya + prev_a;
            ya = naive_a + cur_a;
            pa = q1;

            pre[i].0[lane] = la * out_a + lb * out_b;
            cut[i].0[lane] = c + pm_cut * out_b;
            inc_a += d_inc_a;
            inc_b += d_inc_b;
            pw_a += d_pw_a;
            pw_b += d_pw_b;
            c += d_cut;
        }
        self.cur = Ramp {
            amp: cur.amp,
            ..next
        };
        self.pa = pa;
        self.pb = pb;
        // A late sample that went astray (a wild FM setting) starts over.
        self.ya = if ya.is_finite() { ya } else { 0.0 };
        self.yb = if yb.is_finite() { yb } else { 0.0 };
    }
}

/// Voices filtered together: the filter runs in lanes, so the CPU works on
/// four voices' (independent) feedback loops at once.
pub const LANES: usize = 4;
/// The longest segment, in samples at the voices' rate (a tick at HQ).
pub const SEG: usize = 2 * super::TICK;
/// A segment of four lanes, sample by sample (a lane per voice).
pub type LaneBuf = [F4; SEG];

/// Four lanes of f32, one per voice; the operators work lane by lane and
/// compile to packed instructions.
#[derive(Clone, Copy, Default, Debug, PartialEq)]
#[repr(C, align(16))]
pub struct F4(pub [f32; 4]);

impl F4 {
    #[inline(always)]
    pub const fn splat(x: f32) -> F4 {
        F4([x; 4])
    }

    #[inline(always)]
    fn map2(self, o: F4, f: impl Fn(f32, f32) -> f32) -> F4 {
        F4([
            f(self.0[0], o.0[0]),
            f(self.0[1], o.0[1]),
            f(self.0[2], o.0[2]),
            f(self.0[3], o.0[3]),
        ])
    }

    #[inline(always)]
    fn min(self, o: F4) -> F4 {
        self.map2(o, |a, b| if a < b { a } else { b })
    }

    #[inline(always)]
    fn max(self, o: F4) -> F4 {
        self.map2(o, |a, b| if a > b { a } else { b })
    }
}

impl std::ops::Add for F4 {
    type Output = F4;
    #[inline(always)]
    fn add(self, o: F4) -> F4 {
        F4([
            self.0[0] + o.0[0],
            self.0[1] + o.0[1],
            self.0[2] + o.0[2],
            self.0[3] + o.0[3],
        ])
    }
}

impl std::ops::Sub for F4 {
    type Output = F4;
    #[inline(always)]
    fn sub(self, o: F4) -> F4 {
        F4([
            self.0[0] - o.0[0],
            self.0[1] - o.0[1],
            self.0[2] - o.0[2],
            self.0[3] - o.0[3],
        ])
    }
}

impl std::ops::Mul for F4 {
    type Output = F4;
    #[inline(always)]
    fn mul(self, o: F4) -> F4 {
        F4([
            self.0[0] * o.0[0],
            self.0[1] * o.0[1],
            self.0[2] * o.0[2],
            self.0[3] * o.0[3],
        ])
    }
}

impl std::ops::Div for F4 {
    type Output = F4;
    #[inline(always)]
    fn div(self, o: F4) -> F4 {
        F4([
            self.0[0] / o.0[0],
            self.0[1] / o.0[1],
            self.0[2] / o.0[2],
            self.0[3] / o.0[3],
        ])
    }
}

/// What a lane's filter keeps from segment to segment, gathered from the voices.
struct LaneState {
    z: [F4; 4],
    last: F4,
}

/// The filter's inner loop over a segment. `PER_SAMPLE`: the cutoff moves at
/// audio rate (oscillator B's poly-mod), so its coefficient is looked up
/// every sample; otherwise it is ramped across the segment. `FEEDBACK`: the
/// mixer's third input is the voice's own output rather than the noise.
///
/// The loop is written for a short feedback chain: the states' sum takes
/// coefficients worked out from G beside it, and each stage is
/// y = G·x + (1 − G)·z with the second term ready before x is.
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn filter_loop<const PER_SAMPLE: bool, const FEEDBACK: bool>(
    st: &mut LaneState,
    table: &FilterTable,
    noise: &[f32],
    pre: &LaneBuf,
    cut: &LaneBuf,
    out: &mut LaneBuf,
    len: usize,
    (k, ln, fb_scale): (F4, F4, F4),
    (mut amp, d_amp): (F4, F4),
    (g0, g1): (F4, F4),
) {
    let one = F4::splat(1.0);
    let two = F4::splat(2.0);
    let third_cube = F4::splat(4.0 / 27.0);
    let lo = F4::splat(-1.5);
    let hi = F4::splat(1.5);
    let inv = 1.0 / len as f32;
    let mut g = g0;
    let dg = (g1 - g0) * F4::splat(inv);
    let [mut z1, mut z2, mut z3, mut z4] = st.z;
    let mut last = st.last;
    for i in 0..len {
        if PER_SAMPLE {
            let c = cut[i].0;
            g = F4([
                table.get(c[0]),
                table.get(c[1]),
                table.get(c[2]),
                table.get(c[3]),
            ]);
        }
        // Beside the chain: G's powers, the loop's denominator.
        let h = one - g;
        let g2 = g * g;
        let g3 = g2 * g;
        let g4 = g2 * g2;
        let den = one / (one + k * g4);
        let third = if FEEDBACK {
            let x = (last * fb_scale).max(lo).min(hi);
            x - x * x * x * third_cube
        } else {
            F4::splat(noise[i])
        };
        let u = pre[i] + ln * third;
        // The chain.
        let s = (z1 * (g3 * h) + z2 * (g2 * h)) + (z3 * (g * h) + z4 * h);
        let y4e = (g4 * den) * u + den * s;
        let x = (u - k * y4e).max(lo).min(hi);
        let xin = x - (third_cube * x) * (x * x);
        let y1 = g * xin + h * z1;
        let y2 = g * y1 + h * z2;
        let y3 = g * y2 + h * z3;
        let y4 = g * y3 + h * z4;
        z1 = two * y1 - z1;
        z2 = two * y2 - z2;
        z3 = two * y3 - z3;
        z4 = two * y4 - z4;
        let y = y4 * amp;
        out[i] = y;
        if FEEDBACK {
            last = y;
        }
        amp = amp + d_amp;
        if !PER_SAMPLE {
            g = g + dg;
        }
    }
    st.z = [z1, z2, z3, z4];
    st.last = last;
}

/// The filter, the amplifier and the voice's distortion for up to
/// [`LANES`] voices (`which`, indices into `voices`) whose oscillators wrote
/// their lanes of `pre` and `cut`; adds them, panned, into the bus.
#[allow(clippy::too_many_arguments, clippy::needless_range_loop)] // the lanes of several arrays at once
#[inline(never)]
pub fn filter_group(
    voices: &mut [Voice],
    which: &[usize],
    patch: &Patch,
    table: &FilterTable,
    noise: &[f32],
    pre: &LaneBuf,
    cut: &LaneBuf,
    out: &mut LaneBuf,
    len: usize,
    out_l: &mut [f32],
    out_r: &mut [f32],
) {
    let len = len
        .min(SEG)
        .min(noise.len())
        .min(out_l.len())
        .min(out_r.len());
    let inv = 1.0 / len as f32;
    let ref_level = (VOICE_GAIN * patch.volume * patch.volume).max(1e-3);
    let inv_ref = 1.0 / ref_level;
    let drive = patch.model.drive * 0.5;
    let out_gain = 1.0 / drive;
    let mut st = LaneState {
        z: [F4::default(); 4],
        last: F4::default(),
    };
    let (mut k, mut gain, mut ln) = (F4::default(), F4::default(), F4::default());
    let (mut amp, mut d_amp) = (F4::default(), F4::default());
    let (mut g0, mut g1) = (F4::splat(0.1), F4::splat(0.1));
    let mut per_sample = false;
    let lanes = which.len().min(LANES);
    for (l, &v) in which.iter().enumerate().take(LANES) {
        let v = &voices[v];
        for s in 0..4 {
            st.z[s].0[l] = v.z[s];
        }
        // The feedback path hears the output scaled back to the voice's level.
        st.last.0[l] = v.last * ref_level;
        amp.0[l] = v.cur.amp;
        d_amp.0[l] = (v.next.amp - v.cur.amp) * inv;
        k.0[l] = v.next.k;
        gain.0[l] = out_gain * (1.0 + patch.model.comp * v.next.k);
        ln.0[l] = v.next.ln;
        g0.0[l] = table.get(v.cut_from);
        g1.0[l] = table.get(v.next.cut);
        per_sample |= v.next.pm_cut != 0.0;
    }
    // `last` is the output at the voice's level; fed back, it is scaled up
    // to the oscillators' and saturated.
    // The drive is folded into the oscillators' mix (see `Voice::osc`) and
    // the third input; the output gain into the amplifier's ramp.
    let d = F4::splat(drive);
    let consts = (k, ln * d, F4::splat(2.5 * inv_ref));
    let ramp = (amp * gain, d_amp * gain);
    match (per_sample, patch.feedback) {
        (false, false) => filter_loop::<false, false>(
            &mut st,
            table,
            noise,
            pre,
            cut,
            out,
            len,
            consts,
            ramp,
            (g0, g1),
        ),
        (true, false) => filter_loop::<true, false>(
            &mut st,
            table,
            noise,
            pre,
            cut,
            out,
            len,
            consts,
            ramp,
            (g0, g1),
        ),
        (false, true) => filter_loop::<false, true>(
            &mut st,
            table,
            noise,
            pre,
            cut,
            out,
            len,
            consts,
            ramp,
            (g0, g1),
        ),
        (true, true) => filter_loop::<true, true>(
            &mut st,
            table,
            noise,
            pre,
            cut,
            out,
            len,
            consts,
            ramp,
            (g0, g1),
        ),
    }

    // Each voice's distortion, lane by lane.
    if patch.dist_on {
        let dtype = patch.dist_type;
        let tilt_k = 1.0 - (-std::f32::consts::TAU * 700.0 / patch.vsr).exp();
        let hold_len = 1.0 + (patch.dist_db / 60.0) * 63.0;
        for (l, &vi) in which.iter().enumerate().take(LANES) {
            let v = &mut voices[vi];
            let (dgain, dmix) = (v.next.dgain, v.next.dmix);
            // TONE tilts what goes in; on CORRODE it is CRUSH, the bits.
            let tone = v.next.dtone;
            let (lo_gain, hi_gain) = (1.0 - 0.8 * tone, 1.0 + 0.8 * tone);
            let levels = fastmath::exp2(16.0 - 14.0 * (tone * 0.5 + 0.5));
            let (mut tilt, mut hold, mut hold_phase) = (v.tilt, v.hold, v.hold_phase);
            for frame in out[..len].iter_mut() {
                let y = frame.0[l];
                tilt += (y - tilt) * tilt_k;
                let tilted = (tilt * lo_gain + (y - tilt) * hi_gain) * inv_ref;
                let wet = match dtype {
                    0 => sat(tilted * dgain * 0.7),
                    1 => (tilted * dgain * 0.5).clamp(-1.0, 1.0),
                    2 => fold(tilted * dgain * 0.25),
                    _ => {
                        hold_phase += 1.0;
                        if hold_phase >= hold_len {
                            hold_phase -= hold_len;
                            hold = (y * inv_ref * levels).round() / levels;
                        }
                        hold
                    }
                };
                frame.0[l] = y + (wet * ref_level - y) * dmix;
            }
            v.tilt = if tilt.abs() < 1e-20 { 0.0 } else { tilt };
            v.hold = hold;
            v.hold_phase = hold_phase;
        }
    }

    // Panned into the bus.
    let (mut pl, mut pr) = (F4::default(), F4::default());
    for (l, &v) in which.iter().enumerate().take(LANES) {
        pl.0[l] = voices[v].next.pl;
        pr.0[l] = voices[v].next.pr;
    }
    for i in 0..len {
        let a = out[i] * pl;
        let b = out[i] * pr;
        out_l[i] += (a.0[0] + a.0[1]) + (a.0[2] + a.0[3]);
        out_r[i] += (b.0[0] + b.0[1]) + (b.0[2] + b.0[3]);
    }

    // Back into the voices, out of denormals and away from runaway values.
    let tidy = |v: f32| {
        if v.abs() < 1e-20 || !v.is_finite() {
            0.0
        } else {
            v.clamp(-8.0, 8.0)
        }
    };
    for (l, &v) in which.iter().enumerate().take(lanes) {
        let v = &mut voices[v];
        v.z = [
            tidy(st.z[0].0[l]),
            tidy(st.z[1].0[l]),
            tidy(st.z[2].0[l]),
            tidy(st.z[3].0[l]),
        ];
        v.last = tidy(st.last.0[l] * inv_ref);
        v.cur.amp = v.next.amp;
    }
}

/// The GLIDE knob position for a glide time (the inverse of [`glide_seconds`]).
fn glide_knob(seconds: f32) -> f32 {
    if seconds <= 0.0 {
        0.0
    } else {
        ((seconds - 0.002).max(0.0) / 2.5).sqrt()
    }
}
