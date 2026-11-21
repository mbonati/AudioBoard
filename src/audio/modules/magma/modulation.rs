//! Magma's modulation sources: two of each — XLFO (a 16-step LFO /
//! sequencer), envelope generator, envelope follower, MIDI source and XY
//! controller (or slider). Slots (see the parent module) connect them to
//! any knob, other slots' levels included.

use super::{
    Clock, MAX_STEPS, PARAM_COUNT, SYNC_LENGTHS, XY0, choice, data, ef, efp, eg, egp, on, xlfo,
    xlfo_data, xp,
};
use crate::audio::modules::{ModuleParams, Rng};
use crate::audio::params::gain_to_db;

/// Source numbers in the slots: 0 none, then these.
pub const SOURCE_NAMES: [&str; 13] = [
    "—", "XLFO 1", "XLFO 2", "EG 1", "EG 2", "EF 1", "EF 2", "MIDI 1", "MIDI 2", "XY 1 X",
    "XY 1 Y", "XY 2 X", "XY 2 Y",
];
pub const SOURCES: usize = 12;
pub const SRC_XLFO: usize = 1;
pub const SRC_EG: usize = 3;
pub const SRC_EF: usize = 5;
pub const SRC_MIDI: usize = 7;
pub const SRC_XY: usize = 9;

/// XLFO step curves, EG triggers and ranges, EF triggers and modes, MIDI
/// inputs and curves, XY modes.
pub const STEP_CURVES: [&str; 4] = ["LINEAR", "SQR", "SQRT", "SINE"];
pub const MIDI_TRIGGERS: [&str; 3] = ["FREE", "RETRIG", "LEGATO"];
pub const EG_TRIGGERS: [&str; 3] = ["INPUT", "SC", "MIDI"];
pub const EG_RANGES: [&str; 2] = ["NORMAL", "NEUTRAL"];
pub const EF_TRIGGERS: [&str; 2] = ["INPUT", "SC"];
pub const EF_MODES: [&str; 2] = ["ENVELOPE", "TRANSIENT"];
pub const MIDI_INPUTS: [&str; 6] = [
    "VELOCITY",
    "NOTE",
    "PITCH BEND",
    "MOD WHEEL",
    "AFTERTOUCH",
    "CONTROLLER",
];
pub const MIDI_CURVES: [&str; 6] = ["LINEAR", "EXP", "LOG", "SQUARE", "SQRT", "SINE"];
pub const XY_MODES: [&str; 2] = ["XY", "SLIDER"];

/// What MIDI has said so far (from the MIDI cable, or the keyboards).
#[derive(Clone)]
pub struct MidiIn {
    held: [bool; 128],
    pub count: usize,
    pub note: f32,
    pub velocity: f32,
    pub bend: f32,
    pub pressure: f32,
    pub cc: [f32; 128],
    /// A note started this chunk, and one started with none held.
    pub note_on: bool,
    pub first_on: bool,
}

impl Default for MidiIn {
    fn default() -> Self {
        Self {
            held: [false; 128],
            count: 0,
            note: 60.0,
            velocity: 0.0,
            bend: 0.0,
            pressure: 0.0,
            cc: [0.0; 128],
            note_on: false,
            first_on: false,
        }
    }
}

impl MidiIn {
    pub fn handle(&mut self, [status, a, b]: [u8; 3]) {
        let (a, b) = (a & 0x7F, b & 0x7F);
        match status & 0xF0 {
            0x90 if b > 0 => {
                if self.count == 0 {
                    self.first_on = true;
                }
                if !self.held[a as usize] {
                    self.held[a as usize] = true;
                    self.count += 1;
                }
                self.note = a as f32;
                self.velocity = b as f32 / 127.0;
                self.note_on = true;
            }
            0x80 | 0x90 => {
                if self.held[a as usize] {
                    self.held[a as usize] = false;
                    self.count -= 1;
                }
            }
            0xB0 if a == 123 => {
                self.held = [false; 128];
                self.count = 0;
            }
            0xB0 => self.cc[a as usize] = b as f32 / 127.0,
            0xD0 => self.pressure = a as f32 / 127.0,
            0xE0 => self.bend = ((b as i32) << 7 | a as i32) as f32 / 8192.0 - 1.0,
            _ => {}
        }
    }

    /// Clears the per-chunk flags.
    pub fn next_chunk(&mut self) {
        self.note_on = false;
        self.first_on = false;
    }
}

/// Shapes 0…1 by a MIDI source curve.
pub fn midi_curve(curve: usize, u: f32) -> f32 {
    let u = u.clamp(0.0, 1.0);
    match curve {
        1 => (u * 5.0).exp_m1() / 5f32.exp_m1(),
        2 => (u * 5.0).ln_1p() / 5f32.ln_1p(),
        3 => u * u,
        4 => u.sqrt(),
        5 => (u * std::f32::consts::FRAC_PI_2).sin(),
        _ => u,
    }
}

/// An XLFO step's glide curve, 0…1.
fn step_curve(curve: usize, u: f32) -> f32 {
    match curve {
        1 => u * u,
        2 => u.sqrt(),
        3 => 0.5 - 0.5 * (u * std::f32::consts::PI).cos(),
        _ => u,
    }
}

/// An envelope segment: 0…1 along `u`, bent by `slope` (−1 fast start,
/// 0 linear, 1 slow start).
pub fn segment(u: f32, slope: f32) -> f32 {
    u.clamp(0.0, 1.0).powf(4f32.powf(slope.clamp(-1.0, 1.0)))
}

/// An XLFO step's settings: (value, random, curve, glide).
pub fn step(p: &ModuleParams, x: usize, i: usize) -> (f32, bool, usize, f32) {
    let at = xlfo_data(x) + xlfo::STEP0 + i * 4;
    (
        p.data[at].get(),
        p.data[at + 1].get() > 0.5,
        (p.data[at + 2].get().round().max(0.0) as usize).min(3),
        p.data[at + 3].get(),
    )
}

pub fn steps(p: &ModuleParams, x: usize) -> usize {
    (p.data[xlfo_data(x) + xlfo::STEPS].get().round() as usize).clamp(1, MAX_STEPS)
}

pub struct Xlfo {
    phase: f64,
    randoms: [f32; MAX_STEPS],
    step: usize,
    pub out: f32,
    pub shown_phase: f32,
}

impl Xlfo {
    fn new() -> Self {
        Self {
            phase: 0.0,
            randoms: [0.0; MAX_STEPS],
            step: usize::MAX,
            out: 0.0,
            shown_phase: 0.0,
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn update(
        &mut self,
        x: usize,
        p: &ModuleParams,
        vals: &[f32; PARAM_COUNT],
        clock: &Clock,
        dt: f32,
        midi: &MidiIn,
        rng: &mut Rng,
    ) {
        let d = xlfo_data(x);
        let n = steps(p, x);
        let sync = choice(p, d + xlfo::SYNC, SYNC_LENGTHS.len() + 1);
        if sync == 0 {
            self.phase += (vals[xp(x, xlfo::RATE)] * dt) as f64;
        } else {
            let cycle_beats = SYNC_LENGTHS[sync - 1].1 * 4.0 * vals[xp(x, xlfo::OFFSET)] as f64;
            if clock.playing {
                self.phase = clock.beat0 / cycle_beats;
            } else {
                self.phase += dt as f64 / (cycle_beats * clock.seconds_per_beat);
            }
        }
        match choice(p, d + xlfo::MIDI_TRIG, 3) {
            1 if midi.note_on => self.phase = 0.0,
            2 if midi.first_on => self.phase = 0.0,
            _ => {}
        }
        self.phase = self.phase.rem_euclid(1.0);

        // BALANCE: how the cycle's time splits between its two halves.
        let split = (0.5 + 0.45 * vals[xp(x, xlfo::BALANCE)]) as f64;
        let ph = self.phase;
        let warped = if ph < split {
            0.5 * ph / split
        } else {
            0.5 + 0.5 * (ph - split) / (1.0 - split)
        };
        let psi = (warped + vals[xp(x, xlfo::PHASE)] as f64).rem_euclid(1.0) as f32;
        self.shown_phase = psi;
        let pos = psi * n as f32;
        let i = (pos as usize).min(n - 1);
        let t = pos - i as f32;
        let value = |k: usize, me: &Self| {
            let (v, random, _, _) = step(p, x, k);
            if random { me.randoms[k] } else { v }
        };
        if i != self.step {
            self.step = i;
            if step(p, x, i).1 {
                self.randoms[i] = rng.bipolar();
            }
            let next = (i + 1) % n;
            if step(p, x, next).1 && next != i {
                self.randoms[next] = rng.bipolar();
            }
        }
        let (_, _, curve, glide) = step(p, x, i);
        let glide = (glide + vals[xp(x, xlfo::GLIDE)]).clamp(0.0, 1.0);
        let here = value(i, self);
        let mut out = if glide > 0.0 && t > 1.0 - glide {
            let u = (t - (1.0 - glide)) / glide;
            here + (value((i + 1) % n, self) - here) * step_curve(curve, u)
        } else {
            here
        };
        if on(p, d + xlfo::SNAP) {
            // Two octaves of semitones.
            out = (out * 12.0).round() / 12.0;
        }
        self.out = out.clamp(-1.0, 1.0);
    }
}

/// Envelope generator stages.
pub const IDLE: u8 = 0;
const DELAY: u8 = 1;
const ATTACK: u8 = 2;
const DECAY: u8 = 3;
const SUSTAIN: u8 = 4;
const HOLD: u8 = 5;
const RELEASE: u8 = 6;

pub struct Eg {
    pub stage: u8,
    t: f32,
    level: f32,
    from: f32,
    gate: bool,
    pub out: f32,
}

impl Eg {
    fn new() -> Self {
        Self {
            stage: IDLE,
            t: 0.0,
            level: 0.0,
            from: 0.0,
            gate: false,
            out: 0.0,
        }
    }

    /// `level` is the trigger signal's level in dB (unused with MIDI).
    fn update(
        &mut self,
        e: usize,
        p: &ModuleParams,
        vals: &[f32; PARAM_COUNT],
        level: f32,
        midi: &MidiIn,
        dt: f32,
    ) {
        let v = |k: usize| vals[egp(e, k)];
        let d = data::EG + e * 2;
        let midi_trig = choice(p, d, 3) == 2;
        let gate = if midi_trig {
            midi.count > 0
        } else {
            let threshold = v(eg::THRESHOLD);
            level > threshold || (self.gate && level > threshold - 3.0)
        };
        if (gate && !self.gate) || (midi_trig && midi.note_on) {
            self.stage = DELAY;
            self.t = 0.0;
        } else if !gate && self.gate && (DELAY..=SUSTAIN).contains(&self.stage) {
            self.stage = HOLD;
            self.t = 0.0;
            self.from = self.level;
        }
        self.gate = gate;
        self.t += dt;
        let sustain = v(eg::SUSTAIN).clamp(0.0, 1.0);
        loop {
            match self.stage {
                DELAY if self.t >= v(eg::DELAY) => {
                    self.t -= v(eg::DELAY);
                    self.stage = ATTACK;
                    self.from = self.level;
                }
                ATTACK => {
                    let u = self.t / v(eg::ATTACK).max(1e-4);
                    self.level = self.from + (1.0 - self.from) * segment(u, v(eg::A_SLOPE));
                    if u < 1.0 {
                        break;
                    }
                    self.t -= v(eg::ATTACK);
                    self.stage = DECAY;
                }
                DECAY => {
                    let u = self.t / v(eg::DECAY).max(1e-4);
                    self.level = 1.0 + (sustain - 1.0) * segment(u, v(eg::D_SLOPE));
                    if u < 1.0 {
                        break;
                    }
                    self.stage = SUSTAIN;
                }
                SUSTAIN => {
                    self.level = sustain;
                    break;
                }
                HOLD if self.t >= v(eg::HOLD) => {
                    self.t -= v(eg::HOLD);
                    self.stage = RELEASE;
                    self.from = self.level;
                }
                RELEASE => {
                    let u = self.t / v(eg::RELEASE).max(1e-4);
                    self.level = self.from * (1.0 - segment(u, v(eg::R_SLOPE)));
                    if u < 1.0 {
                        break;
                    }
                    self.stage = IDLE;
                    self.level = 0.0;
                }
                _ => break,
            }
        }
        let neutral = choice(p, d + 1, 2) == 1;
        self.out = if neutral {
            self.level - sustain
        } else {
            self.level
        };
    }
}

pub struct Ef {
    fast: f32,
    slow: f32,
    pub out: f32,
}

impl Ef {
    fn new() -> Self {
        Self {
            fast: 0.0,
            slow: 0.0,
            out: 0.0,
        }
    }

    /// `peak`: the trigger signal's peak this chunk.
    fn update(
        &mut self,
        f: usize,
        p: &ModuleParams,
        vals: &[f32; PARAM_COUNT],
        peak: f32,
        dt: f32,
    ) {
        let (attack, release) = (vals[efp(f, ef::ATTACK)], vals[efp(f, ef::RELEASE)]);
        let follow = |env: &mut f32, a: f32, r: f32| {
            let tau = if peak > *env { a } else { r };
            *env += (peak - *env) * (1.0 - (-dt / tau.max(1e-4)).exp());
        };
        follow(&mut self.fast, attack, release);
        if choice(p, data::EF + f * 2 + 1, 2) == 1 {
            // Transient: only the bursts above the slower trend.
            follow(&mut self.slow, attack * 8.0, release * 2.0);
            self.out = ((gain_to_db(self.fast) - gain_to_db(self.slow)) / 12.0).clamp(0.0, 1.0);
        } else {
            self.out = ((gain_to_db(self.fast) + 60.0) / 60.0).clamp(0.0, 1.0);
        }
    }
}

/// All sources, and their outputs (index = source number − 1).
pub struct Sources {
    pub xlfos: [Xlfo; 2],
    pub egs: [Eg; 2],
    pub efs: [Ef; 2],
    pub midi: MidiIn,
    pub out: [f32; SOURCES],
    /// Peak followers (50 ms release) of the input and the SC input: what
    /// envelope generators compare with their threshold.
    pub levels: [f32; 2],
    rng: Rng,
}

impl Sources {
    pub fn new() -> Self {
        Self {
            xlfos: [Xlfo::new(), Xlfo::new()],
            egs: [Eg::new(), Eg::new()],
            efs: [Ef::new(), Ef::new()],
            midi: MidiIn::default(),
            out: [0.0; SOURCES],
            levels: [0.0; 2],
            rng: Rng::new(0x3A61_57C1),
        }
    }

    /// One chunk of `dt` seconds. `main` and `side` are the peaks of the
    /// node's input and SC input over it.
    /// Sources not `used` (by a slot, or the panel) are skipped.
    #[allow(clippy::too_many_arguments)]
    pub fn update(
        &mut self,
        p: &ModuleParams,
        vals: &[f32; PARAM_COUNT],
        clock: &Clock,
        dt: f32,
        main: f32,
        side: f32,
        used: &[bool; SOURCES],
    ) {
        // A waveform dips through zero: hold its peaks so a steady sound
        // keeps the envelopes' gate open.
        let fall = (-dt / 0.05).exp();
        for (level, peak) in self.levels.iter_mut().zip([main, side]) {
            *level = peak.max(*level * fall);
        }
        for (x, lfo) in self.xlfos.iter_mut().enumerate() {
            if !used[SRC_XLFO - 1 + x] {
                continue;
            }
            lfo.update(x, p, vals, clock, dt, &self.midi, &mut self.rng);
            self.out[SRC_XLFO - 1 + x] = lfo.out;
        }
        for (e, env) in self.egs.iter_mut().enumerate() {
            if !used[SRC_EG - 1 + e] {
                continue;
            }
            let level = self.levels[usize::from(choice(p, data::EG + e * 2, 3) == 1)];
            env.update(e, p, vals, gain_to_db(level), &self.midi, dt);
            self.out[SRC_EG - 1 + e] = env.out;
        }
        for (f, fol) in self.efs.iter_mut().enumerate() {
            if !used[SRC_EF - 1 + f] {
                continue;
            }
            let peak = if choice(p, data::EF + f * 2, 2) == 1 {
                side
            } else {
                main
            };
            fol.update(f, p, vals, peak, dt);
            self.out[SRC_EF - 1 + f] = fol.out;
        }
        for m in 0..2 {
            let d = data::MIDI + m * 3;
            let curve = choice(p, d + 2, MIDI_CURVES.len());
            let midi = &self.midi;
            let raw = match choice(p, d, MIDI_INPUTS.len()) {
                0 => midi.velocity,
                1 => midi.note / 127.0,
                2 => midi.bend,
                3 => midi.cc[1],
                4 => midi.pressure,
                _ => midi.cc[(p.data[d + 1].get().round().max(0.0) as usize).min(127)],
            };
            self.out[SRC_MIDI - 1 + m] = midi_curve(curve, raw.abs()).copysign(raw);
        }
        for c in 0..2 {
            let unipolar = !on(p, data::XY + c * 2 + 1);
            for axis in 0..2 {
                let v = vals[XY0 + c * 2 + axis];
                self.out[SRC_XY - 1 + c * 2 + axis] = if unipolar { (v + 1.0) * 0.5 } else { v };
            }
        }
        self.midi.next_chunk();
    }
}
