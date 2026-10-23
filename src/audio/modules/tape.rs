//! Tape: a tape machine, after the classic tape emulation plugins. The
//! signal is recorded (record EQ, then a magnetic curve on 2× oversampling
//! whose drive, asymmetry and memory follow BIAS and the tape), carried by
//! a moving tape (wow, flutter, wear's dropouts, a transport that slows to
//! a stop and spins back up) and played back (playback EQ, head bump, gap
//! loss, hiss).
//!
//! Three machines: STUDIO (a warm 2-track, round low end), MASTER (a clean
//! mastering deck, tighter and more open) and CASSETTE (narrow, hissy and
//! wobbly). Reel machines run at 7½, 15 or 30 ips: faster is brighter, with
//! the head bump higher and less wow. NAB and IEC record/playback EQ
//! curves; CLASSIC, MODERN and HOT tape (more headroom each).

use std::f32::consts::TAU;
use std::sync::Arc;

use super::equalizer::{BELL, Coeffs, HIGH_SHELF, LOW_SHELF, Svf, amp};
use super::oversample::Oversampler;
use super::{DelayLine, ModuleDef, ModuleParams, Quiet, Rng, Wander, approach, mix_gains, silence, soft_limit};
use crate::audio::engine::{Buf, Inputs, ProcessCtx, Processor};
use crate::audio::fastmath;
use crate::audio::params::{ParamSpec, Unit};

pub const INPUT: usize = 0;
pub const BIAS: usize = 1;
pub const WOW: usize = 2;
pub const FLUTTER: usize = 3;
pub const HISS: usize = 4;
pub const AGE: usize = 5;
pub const OUTPUT: usize = 6;
pub const MIX: usize = 7;

const PARAMS: [ParamSpec; 8] = [
    ParamSpec::lin("INPUT", -12.0, 18.0, 0.0, Unit::Db),
    ParamSpec::lin("BIAS", -1.0, 1.0, 0.0, Unit::Percent).bipolar(),
    ParamSpec::lin("WOW", 0.0, 1.0, 0.15, Unit::Percent),
    ParamSpec::lin("FLUTTER", 0.0, 1.0, 0.15, Unit::Percent),
    ParamSpec::lin("HISS", 0.0, 1.0, 0.0, Unit::Percent),
    ParamSpec::lin("AGE", 0.0, 1.0, 0.0, Unit::Percent),
    ParamSpec::lin("OUTPUT", -18.0, 12.0, 0.0, Unit::Db),
    ParamSpec::lin("MIX", 0.0, 1.0, 1.0, Unit::Percent),
];

/// The switches, kept with the settings.
pub mod data {
    /// [`super::MACHINES`].
    pub const MACHINE: usize = 0;
    /// 7½, 15 or 30 ips (reel machines).
    pub const SPEED: usize = 1;
    /// [`super::TAPES`].
    pub const TAPE: usize = 2;
    /// 0 NAB, 1 IEC.
    pub const EQ: usize = 3;
    /// OUTPUT follows INPUT down (and up), so driving the tape doesn't make
    /// it louder.
    pub const AUTO: usize = 4;
    /// The transport runs (off: the tape slows to a stop).
    pub const RUNNING: usize = 5;
    /// The panel: 0 compact, 1 full.
    pub const DETAIL: usize = 6;
    pub const LEN: usize = 7;
}

const DATA: [f32; data::LEN] = [0.0, 1.0, 1.0, 0.0, 1.0, 1.0, 1.0];

/// What the processor shows the panel.
pub mod viz {
    /// VU levels (RMS with a VU's 300 ms, linear): input after INPUT, and
    /// output, left and right.
    pub const VU_IN: usize = 0;
    pub const VU_OUT: usize = 2;
    /// The transport's speed, 0 (stopped) to 1.
    pub const SPEED: usize = 4;
    /// Wow and flutter now: how far the tape is off time, in ms.
    pub const WOBBLE: usize = 5;
    /// Level the tape is squeezing off loud peaks, in dB (≤ 0).
    pub const SAT: usize = 6;
    /// A dropout's depth now (0: none).
    pub const DROP: usize = 7;
    pub const END: usize = 8;
}

pub static DEF: ModuleDef = ModuleDef {
    name: "Tape",
    prefix: "TAPE",
    hint: "tape machine · saturation · wow",
    inputs: &["IN"],
    outputs: &["OUT"],
    params: &PARAMS,
    cv_depth: &[],
    modes: &[],
    toggle: None,
    toggle_default: false,
    map: false,
    midi_in: false,
    midi_out: false,
    data: &DATA,
    display_h: 0.0,
    controls_h: 0.0,
    viz: viz::END,
};

/// A machine's character.
pub struct Machine {
    pub name: &'static str,
    pub hint: &'static str,
    /// Head bump's lift (dB) and its frequency relative to the speed's.
    bump_db: f32,
    bump_scale: f32,
    /// Gap loss: the playback's top end relative to the speed's.
    hf_scale: f32,
    /// Into the magnetic curve, and its tilt (even harmonics).
    drive: f32,
    asym: f32,
    /// Record pre-emphasis depth (dB).
    emphasis: f32,
    /// Wow's rate (Hz), and how much wow and flutter at full knobs.
    wow_rate: f32,
    wow_scale: f32,
    flutter_scale: f32,
    /// Hiss at full HISS (dB, at 15 ips).
    hiss_db: f32,
    /// Seconds to stop and to get back up to speed.
    stop_secs: f32,
    start_secs: f32,
}

pub const MACHINES: [Machine; 3] = [
    Machine {
        name: "STUDIO",
        hint: "Warm 2-track: round head bump, soft saturation",
        bump_db: 3.0,
        bump_scale: 1.0,
        hf_scale: 1.0,
        drive: 1.0,
        asym: 0.05,
        emphasis: 9.0,
        wow_rate: 0.5,
        wow_scale: 1.0,
        flutter_scale: 1.0,
        hiss_db: -56.0,
        stop_secs: 0.9,
        start_secs: 0.6,
    },
    Machine {
        name: "MASTER",
        hint: "Mastering deck: clean, tight low end, open top",
        bump_db: 1.5,
        bump_scale: 1.15,
        hf_scale: 1.15,
        drive: 0.8,
        asym: 0.02,
        emphasis: 8.0,
        wow_rate: 0.4,
        wow_scale: 0.6,
        flutter_scale: 0.6,
        hiss_db: -62.0,
        stop_secs: 1.2,
        start_secs: 0.7,
    },
    Machine {
        name: "CASSETTE",
        hint: "Portable cassette: narrow, hissy, wobbly (1⅞ ips)",
        bump_db: 2.0,
        bump_scale: 1.0,
        hf_scale: 1.0,
        drive: 1.3,
        asym: 0.08,
        emphasis: 12.0,
        wow_rate: 0.9,
        wow_scale: 2.5,
        flutter_scale: 2.5,
        hiss_db: -46.0,
        stop_secs: 0.45,
        start_secs: 0.3,
    },
];

pub const CASSETTE: usize = 2;

/// What a tape speed sets (all in Hz but the hiss).
#[derive(Clone, Copy)]
pub struct Speed {
    /// In ips.
    pub name: &'static str,
    bump: f32,
    /// Gap loss: where the top end rolls off.
    gap: f32,
    /// The capstan's turning: flutter's rate.
    capstan: f32,
    /// Emphasis corners, NAB and IEC.
    corners: [f32; 2],
    /// Hiss against 15 ips (dB).
    hiss: f32,
}

const fn speed_of(name: &'static str, bump: f32, gap: f32, capstan: f32, corners: [f32; 2], hiss: f32) -> Speed {
    Speed { name, bump, gap, capstan, corners, hiss }
}

pub const SPEEDS: [Speed; 3] = [
    speed_of("7½", 36.0, 12_000.0, 4.8, [3183.0, 2274.0], 3.0),
    speed_of("15", 58.0, 17_500.0, 9.5, [3183.0, 4547.0], 0.0),
    speed_of("30", 100.0, 21_000.0, 19.0, [9095.0, 9095.0], -3.0),
];
/// The cassette's own: 1⅞ ips, 120 µs.
const CASSETTE_SPEED: Speed = speed_of("1⅞", 70.0, 10_000.0, 3.2, [1326.0, 1326.0], 0.0);

/// Tape formulations: name, tip, headroom (the level where the curve bends,
/// linear) and extra tilt.
pub const TAPES: [(&str, &str, f32, f32); 3] = [
    ("CLASSIC", "Vintage tape: little headroom, early, warm saturation", 0.5, 0.04),
    ("MODERN", "Modern tape: balanced headroom", 0.71, 0.02),
    ("HOT", "High-output tape: most headroom, cleaner when pushed", 1.0, 0.0),
];

/// BIAS's range, either way, as the knob shows it.
fn bias_label(v: f32) -> String {
    if v.abs() < 0.01 {
        "cal".to_owned()
    } else if v < 0.0 {
        format!("under {:.0}", -v * 100.0)
    } else {
        format!("over {:.0}", v * 100.0)
    }
}

pub fn choice(p: &ModuleParams, i: usize, count: usize) -> usize {
    (p.data[i].get().round().max(0.0) as usize).min(count - 1)
}

pub fn on(p: &ModuleParams, i: usize) -> bool {
    p.data[i].get() > 0.5
}

pub fn machine(p: &ModuleParams) -> &'static Machine {
    &MACHINES[choice(p, data::MACHINE, MACHINES.len())]
}

/// The speed (the cassette has its own).
fn speed(p: &ModuleParams) -> Speed {
    if choice(p, data::MACHINE, MACHINES.len()) == CASSETTE { CASSETTE_SPEED } else { SPEEDS[choice(p, data::SPEED, SPEEDS.len())] }
}

pub fn speed_name(p: &ModuleParams) -> &'static str {
    speed(p).name
}

pub fn spec(i: usize) -> &'static ParamSpec {
    &PARAMS[i]
}

/// Wow and flutter at full knobs: pitch deviation (fraction).
const WOW_MAX: f32 = 0.006;
const FLUTTER_MAX: f32 = 0.002;

/// A knob's value as the panel shows it.
pub fn format(p: &ModuleParams, i: usize) -> String {
    let v = p.get(i);
    let m = machine(p);
    match i {
        INPUT | OUTPUT => format!("{v:+.1} dB"),
        BIAS => bias_label(v),
        WOW => format!("{:.2}%", v * WOW_MAX * m.wow_scale * 100.0),
        FLUTTER => format!("{:.2}%", v * FLUTTER_MAX * m.flutter_scale * 100.0),
        HISS if v <= 0.0 => "off".to_owned(),
        HISS => format!("{:.0} dB", hiss_db(m, speed(p).hiss, v)),
        AGE if v <= 0.0 => "new".to_owned(),
        _ => format!("{:.0}%", v * 100.0),
    }
}

/// Hiss level for the HISS knob (`v` > 0): from 30 dB under the machine's
/// own up to it.
fn hiss_db(m: &Machine, offset: f32, v: f32) -> f32 {
    m.hiss_db + offset + 30.0 * (v - 1.0)
}

/// A delay the tape always has, so wow and flutter can swing both ways.
const BASE_SECONDS: f32 = 0.008;
/// The tape held: enough to slow to a stop and spin back up.
const TAPE_SECONDS: f32 = 4.0;
/// The jump back to the live tape after spinning up, crossfaded.
const CATCH_SECONDS: f32 = 0.03;
/// Samples the oversampling delays the tape by (the dry signal waits as
/// long, so MIX doesn't comb).
const OVERSAMPLING_LAG: usize = 31;

/// Coefficients for both channels, set once per block.
#[derive(Default)]
struct Eq {
    /// Record: emphasis (HF up, NAB's LF down); playback undoes them.
    pre: [Coeffs; 2],
    post: [Coeffs; 2],
    /// Playback head: bump, the dip above it, the low rolloff, gap loss.
    head: [Coeffs; 4],
}

/// One channel's filter states.
#[derive(Default, Clone, Copy)]
struct Chain {
    pre: [Svf; 2],
    post: [Svf; 2],
    head: [Svf; 4],
    /// The magnetic curve's memory (at 2×), and hiss's low cut.
    mag: f32,
    hiss_lp: f32,
}

/// The tape's curve for this block.
struct Curve {
    /// Into the curve (1 / headroom × machine × bias), and back out.
    gain: f32,
    tilt: f32,
    tanh_tilt: f32,
    /// tanh's slope at the tilt, undone so small signals pass at unity.
    norm: f32,
    /// How much loud signals smear (the tape can't follow at once).
    memory: f32,
}

pub struct Tape {
    params: Arc<ModuleParams>,
    sr: f32,
    quiet: Quiet,
    over: Oversampler,
    chains: [Chain; 2],
    eq: Eq,
    /// The moving tape, and the dry signal lined up with it.
    tape: [DelayLine; 2],
    dry: [DelayLine; 2],
    rng: Rng,
    drift: Wander,
    wow_phase: f32,
    flutter_phase: [f32; 2],
    scrape: f32,
    /// Transport: speed (0..1), stopped dead, extra delay from running slow,
    /// and a crossfade back to the live tape (its old delay, progress).
    speed: f32,
    halted: bool,
    extra: f32,
    catch: Option<(f32, f32)>,
    /// Dropouts: gain now, where it's going, samples left.
    drop: f32,
    drop_target: f32,
    drop_left: usize,
    gain_in: f32,
    gain_out: f32,
    mix: f32,
    /// VU mean squares (in L, R, out L, R) and the saturation reading.
    vu: [f32; 4],
    sat: f32,
}

impl Tape {
    pub fn new(params: Arc<ModuleParams>, sample_rate: f32) -> Self {
        let len = (TAPE_SECONDS * sample_rate) as usize;
        let base = (BASE_SECONDS * sample_rate) as usize + OVERSAMPLING_LAG + 8;
        let mut rng = Rng::new(0x7a9e);
        let drift = Wander::new(&mut rng);
        Self {
            quiet: Quiet::new((0.5 * sample_rate) as usize),
            over: Oversampler::new(),
            chains: [Chain::default(); 2],
            eq: Eq::default(),
            tape: [DelayLine::new(len), DelayLine::new(len)],
            dry: [DelayLine::new(base), DelayLine::new(base)],
            rng,
            drift,
            wow_phase: 0.0,
            flutter_phase: [0.0, 0.31],
            scrape: 0.0,
            speed: if on(&params, data::RUNNING) { 1.0 } else { 0.0 },
            halted: !on(&params, data::RUNNING),
            extra: 0.0,
            catch: None,
            drop: 1.0,
            drop_target: 1.0,
            drop_left: 0,
            gain_in: 1.0,
            gain_out: 1.0,
            mix: params.get(MIX),
            vu: [0.0; 4],
            sat: 0.0,
            params,
            sr: sample_rate,
        }
    }

    /// Sets this block's EQ from the machine, speed, standard, BIAS and AGE.
    fn tune(&mut self, p: &ModuleParams) {
        let m = machine(p);
        let Speed { bump, gap, corners, .. } = speed(p);
        let nab = choice(p, data::EQ, 2) == 0 || choice(p, data::MACHINE, 3) == CASSETTE;
        let corner = corners[usize::from(!nab)];
        let sr = self.sr;
        let g = |f: f32| fastmath::tan_turns(f.clamp(5.0, sr * 0.45) / (2.0 * sr));
        // Record emphasis lifts the highs into the curve (NAB also cuts the
        // lows); playback brings them back down, so loud highs saturate
        // first, as on tape.
        let lf = if nab { 6.0 } else { 0.0 };
        self.eq.pre = [Coeffs::band(HIGH_SHELF, g(corner), 0.7, amp(m.emphasis)), Coeffs::band(LOW_SHELF, g(50.0), 0.7, amp(-lf))];
        self.eq.post = [Coeffs::band(HIGH_SHELF, g(corner), 0.7, amp(-m.emphasis)), Coeffs::band(LOW_SHELF, g(50.0), 0.7, amp(lf))];
        // Under-bias is brighter, over-bias duller; worn tape loses top.
        let bias = p.get(BIAS);
        let age = p.get(AGE);
        let top = gap * m.hf_scale * fastmath::exp2(-bias * 0.6) * (1.0 - 0.45 * age);
        let bump = bump * m.bump_scale;
        self.eq.head = [
            Coeffs::band(BELL, g(bump), 1.0, amp(m.bump_db)),
            Coeffs::band(BELL, g(bump * 2.2), 1.4, amp(-1.0)),
            Coeffs::svf(g(bump * 0.3), std::f32::consts::SQRT_2, (1.0, -std::f32::consts::SQRT_2, -1.0)),
            Coeffs::svf(g(top), 1.0 / 0.6, (0.0, 0.0, 1.0)),
        ];
    }

    fn curve(p: &ModuleParams) -> Curve {
        let m = machine(p);
        let tape = TAPES[choice(p, data::TAPE, TAPES.len())];
        let bias = p.get(BIAS);
        let under = (-bias).max(0.0);
        let tilt = m.asym + tape.3 + under * 0.12;
        let tanh_tilt = fastmath::tanh(tilt);
        Curve {
            gain: m.drive * fastmath::exp2(-bias * 0.5) / tape.2,
            tilt,
            tanh_tilt,
            norm: 1.0 / (1.0 - tanh_tilt * tanh_tilt),
            memory: 0.25 + under * 0.8,
        }
    }

    /// One sample through the magnetic curve, at 2×.
    #[inline]
    fn magnetize(mag: &mut f32, x: f32, c: &Curve) -> f32 {
        let u = x * c.gain;
        let y = (fastmath::tanh(u + c.tilt) - c.tanh_tilt) * c.norm;
        // The tape can't follow loud swings at once: they lose top.
        *mag += (y - *mag) / (1.0 + c.memory * u.abs());
        *mag / c.gain
    }

    /// The tape read `lag` samples behind the write head (cubic).
    #[inline]
    fn read(line: &DelayLine, lag: f32) -> f32 {
        let i = lag as usize;
        let t = lag - i as f32;
        let (a, b, c, d) = (line.tap_at(i.saturating_sub(1)), line.tap_at(i), line.tap_at(i + 1), line.tap_at(i + 2));
        // Catmull-Rom between b and c.
        let c1 = 0.5 * (c - a);
        let c2 = a - 2.5 * b + 2.0 * c - 0.5 * d;
        let c3 = 0.5 * (d - a) + 1.5 * (b - c);
        b + t * (c1 + t * (c2 + t * c3))
    }
}

impl Processor for Tape {
    #[allow(clippy::needless_range_loop)] // both channels in lockstep
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        let p = self.params.clone();
        let m = machine(&p);
        let running = on(&p, data::RUNNING);
        let hiss = p.get(HISS);
        let age = p.get(AGE);
        let input = inputs.get(0);
        let silent_in = Quiet::silent(input, n);
        let moving = (running && self.speed < 1.0) || (!running && !self.halted) || self.catch.is_some();
        if silent_in && hiss <= 0.0 && !moving && self.quiet.resting() {
            silence(outputs, n);
            for i in [viz::VU_IN, viz::VU_IN + 1, viz::VU_OUT, viz::VU_OUT + 1, viz::WOBBLE, viz::DROP] {
                p.viz[i].set(0.0);
            }
            self.vu = [0.0; 4];
            return;
        }

        self.tune(&p);
        let curve = Self::curve(&p);
        let sr = self.sr;
        let Speed { capstan, hiss: hiss_offset, .. } = speed(&p);
        let gain_in_t = fastmath::exp2(p.get(INPUT) / 6.020_6);
        let mut out_db = p.get(OUTPUT);
        if on(&p, data::AUTO) {
            out_db -= p.get(INPUT);
        }
        let gain_out_t = fastmath::exp2(out_db / 6.020_6);
        let mix_t = p.get(MIX);
        let hiss_gain = if hiss > 0.0 { fastmath::exp2(hiss_db(m, hiss_offset, hiss) / 6.020_6) * 1.7 } else { 0.0 };
        // Wow: a slow swing plus a wandering drift; flutter: the capstan and
        // the pinch roller, plus scrape. Depths in samples of delay.
        let wow_dev = p.get(WOW) * WOW_MAX * m.wow_scale;
        let flutter_dev = p.get(FLUTTER) * FLUTTER_MAX * m.flutter_scale;
        let wow_amp = wow_dev * sr / (TAU * m.wow_rate);
        let flutter_amp = flutter_dev * sr / (TAU * capstan);
        let base = BASE_SECONDS * sr;
        let max_extra = (TAPE_SECONDS - 0.2) * sr;
        let (down, up) = (1.0 / (m.stop_secs * sr), 1.0 / (m.start_secs * sr));
        let catch_step = 1.0 / (CATCH_SECONDS * sr);

        // Dropouts: a couple a second at full AGE, a few tens of ms each.
        if self.drop_left == 0 {
            self.drop_target = 1.0;
            if age > 0.0 && self.rng.unit() < age * 2.0 * n as f32 / sr {
                self.drop_left = ((0.02 + 0.13 * self.rng.unit()) * sr) as usize;
                self.drop_target = 1.0 - age * (0.3 + 0.5 * self.rng.unit());
            }
        }
        let drop_k = 1.0 / (0.004 * sr);

        let mut peak = 0.0f32;
        let mut inner = 0.0f32;
        let mut ms = [0.0f32; 4];
        let (mut sat_in, mut sat_out) = (0.0f32, 0.0f32);
        let mut wobble = 0.0;
        for i in 0..n {
            approach(&mut self.gain_in, gain_in_t, ctx.smooth);
            approach(&mut self.gain_out, gain_out_t, ctx.smooth);
            approach(&mut self.mix, mix_t, ctx.smooth);
            let x = match input {
                Some(b) => [b[0][i], b[1][i]],
                None => [0.0; 2],
            };

            // Record: INPUT, emphasis, the curve at 2×, de-emphasis.
            let mut rec = [0.0; 2];
            for ch in 0..2 {
                let v = x[ch] * self.gain_in;
                ms[ch] += v * v;
                let c = &mut self.chains[ch];
                let lifted = c.pre[0].apply(&self.eq.pre[0], v);
                rec[ch] = c.pre[1].apply(&self.eq.pre[1], lifted);
            }
            let twice = self.over.up(rec);
            let mut mag = [[0.0; 2]; 2];
            for (k, s) in twice.iter().enumerate() {
                for ch in 0..2 {
                    mag[k][ch] = Self::magnetize(&mut self.chains[ch].mag, s[ch], &curve);
                    sat_in += s[ch].abs();
                    sat_out += mag[k][ch].abs();
                }
            }
            let rec = self.over.down(mag);
            for ch in 0..2 {
                let c = &mut self.chains[ch];
                let lowered = c.post[0].apply(&self.eq.post[0], rec[ch]);
                let y = c.post[1].apply(&self.eq.post[1], lowered);
                self.tape[ch].push(y);
                self.dry[ch].push(x[ch]);
            }

            // Transport.
            if running {
                if self.halted {
                    // Starting from rest: from the live tape, slowly.
                    self.halted = false;
                    self.extra = 0.0;
                }
                self.speed = (self.speed + up).min(1.0);
            } else {
                self.speed = (self.speed - down).max(0.0);
                if self.speed == 0.0 {
                    self.halted = true;
                }
            }
            if !self.halted {
                self.extra = (self.extra + 1.0 - self.speed).min(max_extra);
                // Up to speed again: back to the live tape.
                if self.speed >= 1.0 && self.extra > 0.0 && self.catch.is_none() {
                    self.catch = Some((self.extra, 0.0));
                    self.extra = 0.0;
                }
            }
            let r = self.speed;

            // Wow and flutter (slower with the tape).
            self.wow_phase = (self.wow_phase + m.wow_rate * r / sr).fract();
            self.flutter_phase[0] = (self.flutter_phase[0] + capstan * r / sr).fract();
            self.flutter_phase[1] = (self.flutter_phase[1] + capstan * 1.63 * r / sr).fract();
            let drift = self.drift.tick(&mut self.rng, 0.4 * r / sr);
            self.scrape += (self.rng.bipolar() - self.scrape) * 0.01;
            let wow = 0.6 * fastmath::sin_turns(self.wow_phase) + 0.4 * drift;
            let flutter = 0.7 * fastmath::sin_turns(self.flutter_phase[0])
                + 0.3 * fastmath::sin_turns(self.flutter_phase[1])
                + 0.5 * self.scrape;
            let swing = wow_amp * wow + flutter_amp * flutter;
            wobble = swing;

            // Play back.
            let lag = base + self.extra + swing;
            let mut played = [0.0; 2];
            if !self.halted {
                for ch in 0..2 {
                    let mut y = Self::read(&self.tape[ch], lag);
                    if let Some((old, t)) = self.catch {
                        let before = Self::read(&self.tape[ch], base + old + swing);
                        y = before + (y - before) * t;
                    }
                    // The head's output falls with the tape's speed.
                    played[ch] = y * r;
                }
            }
            if let Some((old, t)) = self.catch {
                self.catch = (t < 1.0).then_some((old, (t + catch_step).min(1.0)));
            }
            self.drop += (self.drop_target - self.drop) * drop_k;

            let (dry_g, wet_g) = mix_gains(self.mix);
            for ch in 0..2 {
                let c = &mut self.chains[ch];
                let mut y = played[ch];
                if hiss_gain > 0.0 && !self.halted {
                    // Hiss, its lows thinned out.
                    let w = self.rng.bipolar() * hiss_gain * r;
                    c.hiss_lp += (w - c.hiss_lp) * 0.04;
                    y += w - c.hiss_lp;
                }
                for (s, k) in c.head.iter_mut().zip(&self.eq.head) {
                    y = s.apply(k, y);
                }
                let wet = y * self.drop * self.gain_out;
                inner = inner.max(wet.abs());
                let out = soft_limit(self.dry[ch].tap_at(base as usize + OVERSAMPLING_LAG) * dry_g + wet * wet_g);
                outputs[0][ch][i] = out;
                ms[2 + ch] += out * out;
                peak = peak.max(out.abs());
            }
        }
        self.drop_left = self.drop_left.saturating_sub(n);

        // The panel's readings: VU (300 ms), saturation, transport.
        let k = 1.0 - (-(n as f32) / (0.3 * sr)).exp();
        for (vu, s) in self.vu.iter_mut().zip(ms) {
            *vu += (s / n as f32 - *vu) * k;
        }
        for (j, vu) in self.vu.iter().enumerate() {
            p.viz[viz::VU_IN + j].set(vu.sqrt());
        }
        let squeeze = if sat_in > 1e-6 { 20.0 * (sat_out / sat_in).max(1e-3).log10() } else { 0.0 };
        self.sat += (squeeze.min(0.0) - self.sat) * k;
        p.viz[viz::SAT].set(self.sat);
        p.viz[viz::SPEED].set(self.speed);
        p.viz[viz::WOBBLE].set(wobble / sr * 1000.0);
        p.viz[viz::DROP].set(1.0 - self.drop);
        self.quiet.update(silent_in, inner, n);
        p.meter.fetch_max(peak);
    }
}

/// For the collapsed node and the panel: what is set.
pub fn readout(p: &ModuleParams) -> (String, String) {
    let m = machine(p);
    let reel = choice(p, data::MACHINE, MACHINES.len()) != CASSETTE;
    let eq = if !reel { "" } else if choice(p, data::EQ, 2) == 0 { " · NAB" } else { " · IEC" };
    let state = if on(p, data::RUNNING) { "" } else { " · stopped" };
    (format!("{} {} ips", m.name, speed_name(p)), format!("{} tape · {:+.1} dB{eq}{state}", TAPES[choice(p, data::TAPE, TAPES.len())].0, p.get(INPUT)))
}
