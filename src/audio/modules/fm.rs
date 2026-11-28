//! A four-operator FM engine after the Yamaha TX81Z (its OPZ tone chip and
//! the owner's manual's voice parameters), shared by Quartz (the TX-style
//! FM module) and Stormo (whose swarm nodes are FM voices).
//!
//! Every operator is an oscillator with one of the eight TX waveforms (W1
//! sine; W2 its "cusped" variant, odd partials; W3/W4 their positive half
//! only; W5–W8 the same at twice the rate in the first half of the period,
//! silent in the second, W7/W8 rectified), a frequency set as a RATIO of
//! the key (0.50–27.57: four families built on 1, √2, π/2 and √3, with a
//! fine step) or FIXED in Hz (eight ranges, 8 Hz to 32 kHz), a small
//! DETUNE, and an envelope with the TX's rates and levels (AR, D1R, D1L,
//! D2R, RR, EG SHIFT) scaled by key (RATE and LEVEL SCALING) and by
//! velocity (KVS). The operators are wired by one of the eight algorithms;
//! operator 4 can feed itself back (0–7). An LFO (saw, square, triangle,
//! sample & hold) moves the pitch (PMS) and, for the operators that ask
//! (AME), the level (AMS). Release can be stretched by the pseudo-reverb.
//!
//! The work is split as in Sibyl: everything slow (envelopes, pitch,
//! levels) runs once per segment of at most [`SEG`] frames and is ramped;
//! only phases, waveforms and the algorithm run per sample.

use crate::audio::fastmath;

/// Frames between control updates.
pub const SEG: usize = 16;
/// Operators a voice holds.
pub const OPS: usize = 4;

pub const WAVE_NAMES: [&str; 8] = ["W1", "W2", "W3", "W4", "W5", "W6", "W7", "W8"];
pub const SHIFTS: [&str; 4] = ["OFF", "48dB", "24dB", "12dB"];
pub const LFO_WAVES: [&str; 4] = ["SAW UP", "SQUARE", "TRIANGLE", "S/HOLD"];

/// The coarse frequency ratios (CRS 0–63), as the TX lists them.
#[allow(clippy::approx_constant)]
pub const RATIOS: [f32; 64] = [
    0.50, 0.71, 0.78, 0.87, 1.00, 1.41, 1.57, 1.73, 2.00, 2.82, 3.00, 3.14, 3.46, 4.00, 4.24, 4.71,
    5.00, 5.19, 5.65, 6.00, 6.28, 6.92, 7.00, 7.07, 7.85, 8.00, 8.48, 8.65, 9.00, 9.42, 9.89,
    10.00, 10.38, 10.99, 11.00, 11.30, 12.00, 12.11, 12.56, 12.72, 13.00, 13.84, 14.00, 14.10,
    14.13, 15.00, 15.55, 15.57, 15.70, 16.96, 17.27, 17.30, 18.37, 18.84, 19.03, 19.78, 20.41,
    20.76, 21.20, 21.98, 22.49, 23.55, 24.22, 25.95,
];

/// The family a coarse ratio belongs to (1, √2, π/2 or √3), halved below 1:
/// FINE adds sixteenths of it (so 1.00 reaches 1.94, 25.95 reaches 27.57).
fn family(ratio: f32) -> f32 {
    const F: [f32; 4] = [1.0, std::f32::consts::SQRT_2, std::f32::consts::FRAC_PI_2, 1.732_051];
    let mut best = (f32::MAX, 1.0);
    for &f in &F {
        let k = ratio / f;
        let err = (k * 2.0 - (k * 2.0).round()).abs() / k.max(0.5);
        if err < best.0 {
            best = (err, f);
        }
    }
    if ratio < 1.0 { best.1 * 0.5 } else { best.1 }
}

/// The ratio of CRS `crs` and FIN `fine`.
pub fn ratio(crs: usize, fine: usize) -> f32 {
    let r = RATIOS[crs.min(63)];
    r + fine.min(15) as f32 * family(r) / 16.0
}

/// A fixed frequency: CRS picks a 16th of the range, FIN adds single steps
/// (range 0: 8–255 Hz in 1 Hz steps; each range doubles it).
pub fn fixed_hz(crs: usize, fine: usize, range: usize) -> f32 {
    let base = ((crs.min(63) / 4) * 16).max(8) as f32;
    (base + fine.min(15) as f32) * (1u32 << range.min(7)) as f32
}

/// W2's period: each quarter of the sine turned over, a cusp at the top.
#[inline]
fn cusp(x: f32) -> f32 {
    let c = 1.0 - fastmath::cos_turns(x).abs();
    if x < 0.5 { c } else { -c }
}

/// Waveform `w` (0–7 for W1–W8) at phase `t` (turns).
#[inline]
pub fn wave(w: usize, t: f32) -> f32 {
    let x = t - t.floor();
    match w {
        0 => fastmath::sin_turns(x),
        1 => cusp(x),
        2 => {
            if x < 0.5 {
                fastmath::sin_turns(x)
            } else {
                0.0
            }
        }
        3 => {
            if x < 0.5 {
                cusp(x)
            } else {
                0.0
            }
        }
        4 => {
            if x < 0.5 {
                fastmath::sin_turns(2.0 * x)
            } else {
                0.0
            }
        }
        5 => {
            if x < 0.5 {
                cusp(2.0 * x)
            } else {
                0.0
            }
        }
        6 => {
            if x < 0.5 {
                fastmath::sin_turns(2.0 * x).abs()
            } else {
                0.0
            }
        }
        _ => {
            if x < 0.5 {
                cusp(2.0 * x).abs()
            } else {
                0.0
            }
        }
    }
}

/// Points per period of the waveform tables.
const TABLE: usize = 2048;

/// The eight waveforms sampled once (with guard points for interpolation):
/// the sample loop reads these rather than working the shapes out.
fn tables() -> &'static [[f32; TABLE + 2]; 8] {
    static T: std::sync::OnceLock<Box<[[f32; TABLE + 2]; 8]>> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let mut t = Box::new([[0.0; TABLE + 2]; 8]);
        for (w, row) in t.iter_mut().enumerate() {
            for (i, v) in row.iter_mut().enumerate() {
                *v = wave(w, i as f32 / TABLE as f32);
            }
        }
        t
    })
}

/// The algorithms: for each one, the operators (bit k = operator k+1) that
/// modulate operator 1, 2, 3 and 4, and the carriers.
pub const ALGORITHMS: [([u8; OPS], u8); 8] = [
    // 1: 4→3→2→1
    ([0b0010, 0b0100, 0b1000, 0], 0b0001),
    // 2: (3+4)→2→1
    ([0b0010, 0b1100, 0, 0], 0b0001),
    // 3: 3→2→1, 4→1
    ([0b1010, 0b0100, 0, 0], 0b0001),
    // 4: 4→3→1, 2→1
    ([0b0110, 0, 0b1000, 0], 0b0001),
    // 5: 2→1, 4→3
    ([0b0010, 0, 0b1000, 0], 0b0101),
    // 6: 4→1, 4→2, 4→3
    ([0b1000, 0b1000, 0b1000, 0], 0b0111),
    // 7: 4→3, 1, 2
    ([0, 0, 0b1000, 0], 0b0111),
    // 8: all four carriers
    ([0, 0, 0, 0], 0b1111),
];

/// How far a modulator at full level moves its target's phase, in turns.
const MOD_DEPTH: f32 = 1.6;
/// Feedback 0–7, in turns of phase at full level.
const FEEDBACK: [f32; 8] = [0.0, 1.0 / 32.0, 1.0 / 16.0, 1.0 / 8.0, 0.25, 0.5, 1.0, 2.0];
/// The quietest an envelope gets (dB of attenuation).
const FLOOR: f32 = 96.0;

/// Seconds a decay rate (0–31) takes over the whole 96 dB; 0 never moves.
pub fn decay_seconds(rate: f32) -> f32 {
    if rate <= 0.0 {
        f32::INFINITY
    } else {
        20.0 * fastmath::exp2(-(rate.min(31.0) - 1.0) * 0.47)
    }
}

/// Seconds the attack takes from silence to the top.
pub fn attack_seconds(rate: f32) -> f32 {
    decay_seconds(rate) * 0.22
}

/// D1L 0–15 as attenuation (15: the top, no first decay; 0: silence).
pub fn d1l_db(d1l: f32) -> f32 {
    let d = d1l.round().clamp(0.0, 15.0);
    if d <= 0.0 { FLOOR } else { (15.0 - d) * 3.0 }
}

/// One operator's settings, worked out from the knobs once a block.
#[derive(Clone, Copy, Default)]
pub struct OpPatch {
    pub on: bool,
    pub fixed: bool,
    /// Ratio to the key, or the fixed frequency in Hz.
    pub freq: f32,
    pub detune_cents: f32,
    pub wave: usize,
    /// Output level 0–99.
    pub out: f32,
    pub ar: f32,
    pub d1r: f32,
    pub d1l: f32,
    pub d2r: f32,
    /// Release 1–15.
    pub rr: f32,
    /// EG SHIFT 0–3.
    pub shift: usize,
    pub rs: f32,
    pub ls: f32,
    pub kvs: f32,
    pub ame: bool,
    pub ebs: f32,
}

/// A voice's settings: four operators and what they share.
#[derive(Clone, Copy, Default)]
pub struct FmPatch {
    pub ops: [OpPatch; OPS],
    pub alg: usize,
    pub feedback: usize,
    /// Pseudo-reverb 0 (off) – 7.
    pub reverb: f32,
}

impl FmPatch {
    /// Silence goes when every carrier is idle.
    pub fn carriers(&self) -> u8 {
        ALGORITHMS[self.alg.min(7)].1
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Default, Debug)]
pub enum Stage {
    Attack,
    Decay1,
    Decay2,
    Release,
    #[default]
    Idle,
}

/// One operator's envelope, in dB of attenuation.
#[derive(Clone, Copy)]
pub struct Eg {
    pub stage: Stage,
    pub att: f32,
}

impl Default for Eg {
    fn default() -> Self {
        Self {
            stage: Stage::Idle,
            att: FLOOR,
        }
    }
}

impl Eg {
    /// Moves the envelope on by `dt` seconds; `boost` is RATE SCALING's
    /// extra rate, `rev` the pseudo-reverb's release rate when it holds.
    fn tick(&mut self, op: &OpPatch, dt: f32, boost: f32, rev: Option<f32>) {
        match self.stage {
            Stage::Attack => {
                if op.ar <= 0.0 {
                    return;
                }
                // Linear in amplitude, as the ear hears an attack.
                let lin = fastmath::exp2(-self.att / 6.0206) + dt / attack_seconds(op.ar + boost);
                if lin >= 1.0 {
                    self.att = 0.0;
                    self.stage = Stage::Decay1;
                } else {
                    self.att = -6.0206 * fastmath::log2(lin);
                }
            }
            Stage::Decay1 => {
                let target = d1l_db(op.d1l);
                if op.d1r > 0.0 {
                    self.att += dt * FLOOR / decay_seconds(op.d1r + boost);
                }
                if self.att >= target || op.d1l >= 15.0 {
                    self.att = self.att.min(target.max(0.0));
                    self.stage = Stage::Decay2;
                }
            }
            Stage::Decay2 => {
                if op.d2r > 0.0 {
                    self.att = (self.att + dt * FLOOR / decay_seconds(op.d2r + boost)).min(FLOOR);
                }
            }
            Stage::Release => {
                let rr = op.rr.max(1.0) * 2.0 + 1.0;
                let rate = match rev {
                    Some(r) => rr.min(r),
                    None => rr,
                };
                self.att += dt * FLOOR / decay_seconds(rate + boost);
                if self.att >= FLOOR {
                    self.att = FLOOR;
                    self.stage = Stage::Idle;
                }
            }
            Stage::Idle => {}
        }
    }

    /// The level after EG SHIFT, in dB of attenuation.
    fn shifted(&self, shift: usize) -> f32 {
        const SCALE: [f32; 4] = [1.0, 0.5, 0.25, 0.125];
        self.att * SCALE[shift.min(3)]
    }
}

/// What moves every voice at once, per block.
#[derive(Clone, Copy, Default)]
pub struct Mods {
    /// Pitch offset in semitones (bend, LFO, portamento are per voice).
    pub pitch: f32,
    /// LFO amplitude modulation 0..1 times AMS's depth, in dB.
    pub am_db: f32,
    /// EG bias from the breath controller, 0..1 (EBS scales it per operator).
    pub eg_bias: f32,
}

/// An operator's running state.
#[derive(Clone, Copy, Default)]
struct OpState {
    phase: f32,
    eg: Eg,
    /// Linear level, now and at the end of the segment.
    amp: f32,
    inc: f32,
}

/// One FM voice: four operators and the feedback memory.
#[derive(Clone, Copy, Default)]
pub struct FmVoice {
    ops: [OpState; OPS],
    fb: [f32; 2],
    /// The key's level scaling and rate boost, fixed at the key.
    note: f32,
    vel: f32,
    gate: bool,
    /// The pseudo-reverb has caught the release.
    reverb: bool,
}

impl FmVoice {
    /// Starts the envelopes (from where they are: no click on a retrigger).
    pub fn key_on(&mut self, note: f32, vel: f32, restart_phase: bool) {
        self.note = note;
        self.vel = vel;
        self.gate = true;
        self.reverb = false;
        for o in self.ops.iter_mut() {
            o.eg.stage = Stage::Attack;
            if restart_phase && o.eg.att >= FLOOR - 0.5 {
                o.phase = 0.0;
            }
        }
    }

    pub fn key_off(&mut self) {
        self.gate = false;
        for o in self.ops.iter_mut() {
            if o.eg.stage != Stage::Idle {
                o.eg.stage = Stage::Release;
            }
        }
    }

    pub fn kill(&mut self) {
        *self = Self::default();
    }

    pub fn gate(&self) -> bool {
        self.gate
    }

    /// Still sounding: some carrier is not idle.
    pub fn active(&self, patch: &FmPatch) -> bool {
        let car = patch.carriers();
        (0..OPS).any(|k| car & (1 << k) != 0 && self.ops[k].eg.stage != Stage::Idle)
    }

    /// The loudest carrier's level, 0..1 (the panel's meters).
    pub fn level(&self, patch: &FmPatch) -> f32 {
        let car = patch.carriers();
        (0..OPS)
            .filter(|k| car & (1 << k) != 0)
            .map(|k| self.ops[k].amp)
            .fold(0.0, f32::max)
    }

    /// An operator's envelope level, 0..1 (for displays).
    pub fn op_level(&self, k: usize) -> f32 {
        fastmath::exp2(-self.ops[k].eg.att / 6.0206)
    }

    /// Renders `out.len()` frames (at most [`SEG`]) of this voice at `pitch`
    /// (MIDI note, fractional) and adds them into `out`. `bright` scales the
    /// modulators' levels (Stormo's nodes), `sr` is the rate.
    #[allow(clippy::needless_range_loop)]
    pub fn render(
        &mut self,
        patch: &FmPatch,
        mods: &Mods,
        pitch: f32,
        bright: f32,
        sr: f32,
        out: &mut [f32],
    ) {
        let m = out.len().min(SEG);
        if m == 0 {
            return;
        }
        let dt = m as f32 / sr;
        let carriers = ALGORITHMS[patch.alg.min(7)].1;
        let key_hz = 440.0 * fastmath::exp2((pitch + mods.pitch - 69.0) / 12.0);
        // Key scaling: from C1 (24) up.
        let above = (self.note - 24.0).max(0.0);
        let rev = (patch.reverb > 0.0 && self.reverb).then_some(patch.reverb * 2.0 + 1.0);

        let mut amp1 = [0.0f32; OPS];
        let mut inc1 = [0.0f32; OPS];
        for k in 0..OPS {
            let op = &patch.ops[k];
            let s = &mut self.ops[k];
            let boost = op.rs * above / 24.0;
            s.eg.tick(op, dt, boost, rev);
            let mut att = s.eg.shifted(op.shift);
            att += (99.0 - op.out.clamp(0.0, 99.0)) * 0.75;
            att += op.ls / 99.0 * 96.0 * (above / 79.0).min(1.0);
            att += op.kvs / 7.0 * (1.0 - self.vel) * 32.0;
            if op.ame {
                att += mods.am_db;
            }
            att += op.ebs / 7.0 * mods.eg_bias * 40.0;
            let carrier = carriers & (1 << k) != 0;
            let mut a = if op.on && op.out > 0.0 && att < FLOOR {
                fastmath::exp2(-att / 6.0206)
            } else {
                0.0
            };
            if !carrier {
                a *= bright;
            }
            amp1[k] = a;
            let hz = if op.fixed { op.freq } else { key_hz * op.freq };
            let hz = hz * fastmath::exp2(op.detune_cents / 1200.0);
            inc1[k] = (hz / sr).min(0.49);
        }
        // The pseudo-reverb: once operator 1 is 18 dB down after key off.
        if !self.gate && self.ops[0].eg.att >= 18.0 {
            self.reverb = true;
        }

        let fb = FEEDBACK[patch.feedback.min(7)];
        let inv = 1.0 / m as f32;
        let mut amp = [0.0f32; OPS];
        let mut damp = [0.0f32; OPS];
        let mut inc = [0.0f32; OPS];
        let mut dinc = [0.0f32; OPS];
        let mut phase = [0.0f32; OPS];
        let mut waves = [0usize; OPS];
        for k in 0..OPS {
            let s = &self.ops[k];
            amp[k] = s.amp;
            damp[k] = (amp1[k] - s.amp) * inv;
            inc[k] = if s.inc > 0.0 { s.inc } else { inc1[k] };
            dinc[k] = (inc1[k] - inc[k]) * inv;
            phase[k] = s.phase;
            waves[k] = patch.ops[k].wave;
        }
        let ncar = carriers.count_ones().max(1) as usize;
        let norm = [1.0, 0.75, 0.6, 0.5][ncar - 1];
        let mut st = Run {
            phase,
            amp,
            damp,
            inc,
            dinc,
            tabs: {
                let t = tables();
                [&t[waves[0].min(7)], &t[waves[1].min(7)], &t[waves[2].min(7)], &t[waves[3].min(7)]]
            },
            fb0: self.fb[0],
            fb1: self.fb[1],
            fb,
            norm,
        };
        // One loop per algorithm, so the routing is settled before the
        // samples run.
        let out = &mut out[..m];
        macro_rules! go {
            ($a:literal) => {
                st.run::<$a>(out)
            };
        }
        match patch.alg.min(7) {
            0 => go!(0),
            1 => go!(1),
            2 => go!(2),
            3 => go!(3),
            4 => go!(4),
            5 => go!(5),
            6 => go!(6),
            _ => go!(7),
        }
        let (phase, fb0, fb1) = (st.phase, st.fb0, st.fb1);
        self.fb = [fb0, fb1];
        for k in 0..OPS {
            let s = &mut self.ops[k];
            s.phase = phase[k];
            s.amp = amp1[k];
            s.inc = inc1[k];
        }
    }
}

/// A segment's running state, for the sample loop.
struct Run {
    phase: [f32; OPS],
    amp: [f32; OPS],
    damp: [f32; OPS],
    inc: [f32; OPS],
    dinc: [f32; OPS],
    tabs: [&'static [f32; TABLE + 2]; OPS],
    fb0: f32,
    fb1: f32,
    fb: f32,
    norm: f32,
}

impl Run {
    #[inline(always)]
    fn op(&self, k: usize, pm: f32) -> f32 {
        let t = self.phase[k] + pm;
        let f = (t - t.floor()) * TABLE as f32;
        let i = (f as usize).min(TABLE);
        let tab = self.tabs[k];
        let (a, b) = (tab[i], tab[i + 1]);
        (a + (b - a) * (f - i as f32)) * self.amp[k]
    }

    /// The samples of algorithm `A` (see [`ALGORITHMS`]), added into `out`.
    #[inline(always)]
    fn run<const A: usize>(&mut self, out: &mut [f32]) {
        const D: f32 = MOD_DEPTH;
        for o in out.iter_mut() {
            let y4 = self.op(3, (self.fb0 + self.fb1) * 0.5 * self.fb);
            self.fb1 = self.fb0;
            self.fb0 = y4;
            let s = match A {
                0 => {
                    let y3 = self.op(2, D * y4);
                    let y2 = self.op(1, D * y3);
                    self.op(0, D * y2)
                }
                1 => {
                    let y3 = self.op(2, 0.0);
                    let y2 = self.op(1, D * (y3 + y4));
                    self.op(0, D * y2)
                }
                2 => {
                    let y3 = self.op(2, 0.0);
                    let y2 = self.op(1, D * y3);
                    self.op(0, D * (y2 + y4))
                }
                3 => {
                    let y3 = self.op(2, D * y4);
                    let y2 = self.op(1, 0.0);
                    self.op(0, D * (y2 + y3))
                }
                4 => {
                    let y3 = self.op(2, D * y4);
                    let y2 = self.op(1, 0.0);
                    self.op(0, D * y2) + y3
                }
                5 => {
                    let m = D * y4;
                    self.op(0, m) + self.op(1, m) + self.op(2, m)
                }
                6 => {
                    let y3 = self.op(2, D * y4);
                    self.op(0, 0.0) + self.op(1, 0.0) + y3
                }
                _ => {
                    self.op(0, 0.0) + self.op(1, 0.0) + self.op(2, 0.0) + y4
                }
            };
            *o += s * self.norm;
            for k in 0..OPS {
                let p = self.phase[k] + self.inc[k];
                self.phase[k] = if p >= 1.0 { p - 1.0 } else { p };
                self.amp[k] += self.damp[k];
                self.inc[k] += self.dinc[k];
            }
        }
    }
}

/// The LFO every voice of a module shares.
#[derive(Clone, Copy, Default)]
pub struct Lfo {
    pub phase: f32,
    /// Fade-in after a key (LFO DELAY), 0..1.
    pub fade: f32,
    held: f32,
    pub value: f32,
}

impl Lfo {
    /// Speed 0–99 in Hz: about 0.007 Hz to 50 Hz.
    pub fn hz(speed: f32) -> f32 {
        0.007 * fastmath::exp2(speed.clamp(0.0, 99.0) * 12.8 / 99.0)
    }

    /// LFO DELAY 0–99 in seconds (about 15 s at 99).
    pub fn delay_seconds(delay: f32) -> f32 {
        let d = delay.clamp(0.0, 99.0) / 99.0;
        15.0 * d * d
    }

    pub fn restart(&mut self, sync: bool) {
        if sync {
            self.phase = 0.0;
        }
        self.fade = 0.0;
    }

    /// Moves on by `dt` seconds; returns −1..1 (times the delay's fade).
    pub fn tick(&mut self, wave: usize, speed: f32, delay: f32, dt: f32, rnd: f32) -> f32 {
        let before = self.phase;
        self.phase = (self.phase + Self::hz(speed) * dt).fract();
        let wrapped = self.phase < before;
        let t = self.phase;
        let v = match wave {
            0 => 2.0 * t - 1.0,
            1 => {
                if t < 0.5 {
                    1.0
                } else {
                    -1.0
                }
            }
            2 => 1.0 - 4.0 * (t - 0.5).abs(),
            _ => {
                if wrapped {
                    self.held = rnd;
                }
                self.held
            }
        };
        let d = Self::delay_seconds(delay);
        self.fade = if d <= 0.0 {
            1.0
        } else {
            (self.fade + dt / d).min(1.0)
        };
        self.value = v * self.fade;
        self.value
    }
}

/// The microtonal scales: equal, the seven temperaments of the manual's
/// table (cents of each pitch class from the key), quarter and eighth tones.
pub const MICRO_NAMES: [&str; 10] = [
    "EQUAL",
    "PURE MAJ",
    "PURE MIN",
    "MEANTONE",
    "PYTHAGOR",
    "WERCKM",
    "KIRNBERG",
    "VALLOTTI",
    "1/4 TONE",
    "1/8 TONE",
];

#[allow(clippy::excessive_precision)]
const TEMPERAMENTS: [[f32; 12]; 7] = [
    [
        0.0, 70.673, 203.910, 315.641, 386.314, 498.045, 568.718, 701.955, 772.628, 884.359,
        1017.596, 1088.269,
    ],
    [
        0.0, 70.673, 182.404, 315.641, 386.314, 498.045, 568.718, 701.955, 772.628, 884.359,
        1017.596, 1088.269,
    ],
    [
        0.0, 76.049, 193.157, 310.265, 386.314, 503.422, 579.471, 696.578, 772.627, 889.735,
        1006.843, 1082.892,
    ],
    [
        0.0, 113.685, 203.910, 294.135, 407.820, 498.045, 611.730, 701.955, 815.640, 905.865,
        996.090, 1109.775,
    ],
    [
        0.0, 90.225, 192.180, 294.135, 390.225, 498.045, 588.270, 696.090, 792.180, 888.270,
        996.090, 1092.180,
    ],
    [
        0.0, 90.225, 193.157, 294.135, 386.314, 498.045, 590.224, 696.578, 792.180, 889.735,
        996.090, 1088.269,
    ],
    [
        0.0, 94.135, 196.090, 298.045, 392.180, 501.955, 592.180, 698.045, 796.090, 894.135,
        1000.0, 1090.225,
    ],
];

/// A key's pitch (fractional MIDI note) in microtonal scale `scale` built
/// on pitch class `key`.
pub fn micro_pitch(note: f32, scale: usize, key: usize) -> f32 {
    match scale {
        0 => note,
        1..=7 => {
            let n = note.round() as i32;
            let rel = n - key as i32;
            let pc = rel.rem_euclid(12) as usize;
            let oct = rel.div_euclid(12);
            key as f32 + oct as f32 * 12.0 + TEMPERAMENTS[scale - 1][pc] / 100.0 + (note - n as f32)
        }
        // Quarter and eighth tones: steps from middle C (60).
        8 => 60.0 + (note - 60.0) * 0.5,
        _ => 60.0 + (note - 60.0) * 0.25,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ratios_match_the_manual() {
        assert_eq!(ratio(0, 0), 0.5);
        assert!((ratio(0, 15) - 0.97).abs() < 0.01);
        assert!((ratio(4, 15) - 1.94).abs() < 0.01);
        assert!((ratio(63, 15) - 27.57).abs() < 0.02);
        assert_eq!(fixed_hz(0, 0, 0), 8.0);
        assert_eq!(fixed_hz(63, 15, 0), 255.0);
        assert_eq!(fixed_hz(63, 15, 7), 255.0 * 128.0);
    }

    #[test]
    fn waves_are_bounded_and_shaped() {
        for w in 0..8 {
            for i in 0..1000 {
                let v = wave(w, i as f32 / 1000.0);
                assert!(v.abs() <= 1.0001, "W{} at {i}: {v}", w + 1);
            }
        }
        // Half waves rest in the second half.
        for w in 2..8 {
            assert_eq!(wave(w, 0.75), 0.0);
        }
        // W2's cusp at the top.
        assert!((wave(1, 0.25) - 1.0).abs() < 1e-4);
    }
}
