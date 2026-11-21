//! Larsen: a feedback drone. A floor of noise excites two plucked strings
//! (one a side) inside a loop that saturates: the strings ring, the drive
//! bounds them, two filters and a reverb shape them, and the BODY delay
//! brings it all back into the strings. With enough FEEDBACK the loop
//! sounds on its own and settles where the strings' harmonics and BODY's
//! resonances meet, which is rarely just the note FREQ shows: moving BODY
//! can jump it to another partial. Everything in the loop changes what it
//! sustains, not only its tone.
//!
//! After the loop, an echo ([`echo`]); PONG bounces it from side to side.
//! Keys on the MIDI input can set the pitch (PITCH: BODY then snaps to a
//! whole number of the note's periods and a band-pass keeps the loop on
//! the fundamental) and open the loop only while held (GATE). POLY runs
//! three loops, on FREQ and two intervals or on three keys ([`keys`]),
//! each hearing the others through COUPLING; LOCK tunes the knobs' three
//! to their notes as the keys do.
//!
//! An independent take on the circuit of Drosera (codeskine) and the
//! Audrey II firmware it grew from, written anew for the board.

pub mod echo;
pub mod keys;
pub mod parts;

use std::f32::consts::TAU;
use std::sync::Arc;

use self::echo::{Echo, Shape};
use self::keys::{Keys, STRINGS};
use self::parts::{Band4, Biquad, Kind, Strand, Verb, drive};
use super::{DelayLine, ModuleDef, ModuleParams, Quiet, Rng, silence, soft_limit};
use crate::audio::engine::{Buf, Inputs, ProcessCtx, Processor};
use crate::audio::midi::{MidiState, note_hz};
use crate::audio::params::{ParamSpec, Unit, format_hz, format_seconds, note_label};

pub const FREQ: usize = 0;
pub const FEEDBACK: usize = 1;
pub const BODY: usize = 2;
pub const LOWPASS: usize = 3;
pub const HIGHPASS: usize = 4;
pub const MIX: usize = 5;
pub const DECAY: usize = 6;
pub const SEND: usize = 7;
pub const TIME: usize = 8;
pub const REPEATS: usize = 9;
pub const DRIVE: usize = 10;
pub const TONE: usize = 11;
pub const WIDTH: usize = 12;
pub const GAIN: usize = 13;
pub const INPUT: usize = 14;
pub const LEVEL: usize = 15;
pub const COUPLING: usize = 16;
pub const INTERVAL2: usize = 17;
pub const INTERVAL3: usize = 18;

const PARAMS: [ParamSpec; 19] = [
    ParamSpec::lin("FREQ", 16.0, 72.0, 40.0, Unit::Note),
    ParamSpec::lin("FEEDBACK", -60.0, 12.0, -6.0, Unit::Db),
    ParamSpec::log("BODY", 0.001, 0.1, 0.0099, Unit::Seconds),
    ParamSpec::log("LOWPASS", 100.0, 18_000.0, 10_700.0, Unit::Hz),
    ParamSpec::log("HIGHPASS", 10.0, 4000.0, 33.0, Unit::Hz),
    ParamSpec::lin("REVERB", 0.0, 1.0, 0.3, Unit::Percent),
    ParamSpec::lin("DECAY", 0.2, 1.0, 0.85, Unit::Percent),
    ParamSpec::lin("SEND", 0.0, 1.0, 0.2, Unit::Percent),
    ParamSpec::log("TIME", 0.05, echo::LONGEST, 1.29, Unit::Seconds),
    ParamSpec::lin("REPEATS", 0.0, 1.5, 0.6, Unit::Percent),
    ParamSpec::lin("DRIVE", 0.0, 1.0, 0.0, Unit::Percent),
    ParamSpec::log("TONE", 200.0, 3200.0, echo::CENTER, Unit::Hz),
    ParamSpec::lin("WIDTH", 0.0, 1.0, 1.0, Unit::Percent),
    ParamSpec::lin("GAIN", -30.0, 6.0, 0.0, Unit::Db),
    ParamSpec::lin("INPUT", 0.0, 2.0, 1.0, Unit::Decibel),
    ParamSpec::lin("LEVEL", 0.0, 1.0, 0.49, Unit::Decibel),
    ParamSpec::lin("COUPLING", 0.0, 1.0, 0.0, Unit::Percent),
    ParamSpec::lin("INTERVAL 2", -24.0, 24.0, 7.0, Unit::Semitones).bipolar(),
    ParamSpec::lin("INTERVAL 3", -24.0, 24.0, 12.0, Unit::Semitones).bipolar(),
];

pub fn spec(i: usize) -> &'static ParamSpec {
    &PARAMS[i]
}

/// The switches, kept beyond the knobs (1 on, 0 off).
pub mod data {
    /// The echo's time warp: a quarter of the time above the shortest.
    pub const WARP: usize = 0;
    /// The echo bounces between left and right.
    pub const PONG: usize = 1;
    /// Keys on the MIDI input set the pitch.
    pub const PITCH: usize = 2;
    /// The loop is open only while a key is held.
    pub const GATE: usize = 3;
    /// Three strings, three loops.
    pub const POLY: usize = 4;
    /// The knobs' three strings tuned to their notes.
    pub const LOCK: usize = 5;
    pub const LEN: usize = 6;
}

const DATA: [f32; data::LEN] = [0.0, 0.0, 0.0, 0.0, 0.0, 1.0];

/// Samples of the output kept for the panel's spectrum.
pub const RING_LEN: usize = 8192;

/// What the processor shows the panel.
pub mod viz {
    use super::{RING_LEN, STRINGS};
    /// Each string's note now (bend and glide included).
    pub const NOTE: usize = 0;
    /// Each loop's BODY delay now, in seconds (snapped when tuned).
    pub const BODY: usize = NOTE + STRINGS;
    /// Each loop's level (0…1).
    pub const LEVEL: usize = BODY + STRINGS;
    /// Each loop's feedback is open (1) or closed (0).
    pub const OPEN: usize = LEVEL + STRINGS;
    /// How far each string is in (the second and third fade in with POLY).
    pub const RUNS: usize = OPEN + STRINGS;
    /// 1 while a key sets the pitch, and while the strings are tuned to
    /// their notes.
    pub const MIDI: usize = RUNS + STRINGS;
    pub const TUNED: usize = MIDI + 1;
    /// The echo's time now, in seconds, and how far PONG is in.
    pub const ECHO: usize = TUNED + 1;
    pub const PONG: usize = ECHO + 1;
    /// The sample rate, the spectrum ring's write position, and the ring.
    pub const SR: usize = PONG + 1;
    pub const WRITE: usize = SR + 1;
    pub const RING: usize = WRITE + 1;
    pub const END: usize = RING + RING_LEN;
}

pub static DEF: ModuleDef = ModuleDef {
    name: "Larsen",
    prefix: "LARSEN",
    hint: "feedback drone · strings in a loop",
    inputs: &["IN"],
    outputs: &["OUT"],
    params: &PARAMS,
    cv_depth: &[],
    modes: &[],
    toggle: None,
    toggle_default: false,
    map: false,
    midi_in: true,
    midi_out: false,
    data: &DATA,
    display_h: 0.0,
    controls_h: 0.0,
    viz: viz::END,
};

pub fn flag(p: &ModuleParams, d: usize) -> bool {
    p.data[d].get() > 0.5
}

pub fn set_flag(p: &ModuleParams, d: usize, on: bool) {
    p.data[d].set(if on { 1.0 } else { 0.0 });
}

/// The echo's time, WARP included: it quarters the time above the
/// shortest, so long echoes get three to four times shorter (a warp more
/// than a halving).
pub fn echo_time(p: &ModuleParams) -> f32 {
    let t = p.get(TIME);
    if flag(p, data::WARP) {
        0.05 + (t - 0.05) / 4.0
    } else {
        t
    }
}

fn db_gain(db: f32) -> f32 {
    10f32.powf(db / 20.0)
}

/// How much the reverb in the loop swells the drone, for its MIX and
/// DECAY (measured: the longer it rings, the more it builds up).
fn swell(mix: f32, decay: f32) -> f32 {
    let wet = 0.73 / (1.0 - 0.98 * decay * decay).sqrt();
    1.0 + mix * (wet - 1.0)
}

/// The output's trim for the reverb's swell: 1 at the defaults, less as it
/// builds up, so a long reverb comes out no louder (the loop itself is
/// left as it is).
pub fn reverb_trim(mix: f32, decay: f32) -> f32 {
    swell(PARAMS[MIX].default, PARAMS[DECAY].default) / swell(mix, decay)
}

pub fn format(p: &ModuleParams, i: usize) -> String {
    let v = p.get(i);
    match i {
        FREQ => format!(
            "{} · {}",
            note_label(v.round() as i32),
            format_hz(note_hz(v))
        ),
        TIME => format_seconds(echo_time(p)),
        DRIVE => format!("+{:.1} dB", 12.0 * v),
        INTERVAL2 | INTERVAL3 => format!("{:+} st", v.round() as i32),
        _ => PARAMS[i].format(v),
    }
}

/// A built-in sound: its name, the knobs it sets (the rest at their
/// defaults), and WARP, PONG, PITCH, GATE, POLY and LOCK.
pub type Sound = (&'static str, &'static [(usize, f32)], [bool; data::LEN]);

const PLAIN: [bool; data::LEN] = [false, false, false, false, false, true];

pub const FACTORY: [Sound; 16] = [
    ("Default", &[], PLAIN),
    (
        "Cellar",
        &[
            (FREQ, 20.5),
            (FEEDBACK, -2.4),
            (BODY, 0.0567),
            (LOWPASS, 1035.0),
            (HIGHPASS, 18.2),
            (MIX, 0.35),
            (DECAY, 0.94),
            (SEND, 0.15),
            (TIME, 1.83),
            (REPEATS, 0.53),
        ],
        PLAIN,
    ),
    (
        "Whistle",
        &[
            (FREQ, 60.8),
            (FEEDBACK, -9.6),
            (BODY, 0.0032),
            (LOWPASS, 18_000.0),
            (HIGHPASS, 81.0),
            (MIX, 0.45),
            (DECAY, 0.92),
            (SEND, 0.3),
            (TIME, 0.84),
            (REPEATS, 0.68),
        ],
        PLAIN,
    ),
    (
        "Nave",
        &[
            (FREQ, 41.2),
            (FEEDBACK, -3.8),
            (BODY, 0.031),
            (LOWPASS, 6370.0),
            (HIGHPASS, 24.6),
            (MIX, 0.85),
            (DECAY, 0.985),
            (TIME, 2.14),
            (REPEATS, 0.53),
        ],
        PLAIN,
    ),
    (
        "Drift",
        &[
            (FREQ, 44.0),
            (FEEDBACK, -9.6),
            (BODY, 0.0131),
            (LOWPASS, 2920.0),
            (HIGHPASS, 44.7),
            (MIX, 0.15),
            (SEND, 0.8),
            (TIME, 1.55),
            (REPEATS, 1.28),
        ],
        PLAIN,
    ),
    (
        "Warp",
        &[
            (FREQ, 35.6),
            (LOWPASS, 2255.0),
            (DECAY, 0.9),
            (SEND, 0.75),
            (TIME, 2.48),
            (REPEATS, 1.2),
        ],
        [true, false, false, false, false, true],
    ),
    (
        "Undertow",
        &[
            (FREQ, 27.2),
            (FEEDBACK, 1.2),
            (BODY, 0.0725),
            (LOWPASS, 475.0),
            (HIGHPASS, 18.2),
            (MIX, 0.5),
            (DECAY, 0.95),
            (SEND, 0.4),
            (TIME, 2.83),
            (REPEATS, 0.83),
        ],
        PLAIN,
    ),
    (
        "Howl",
        &[
            (FREQ, 52.4),
            (FEEDBACK, 8.4),
            (BODY, 0.002),
            (LOWPASS, 18_000.0),
            (HIGHPASS, 60.3),
            (MIX, 0.1),
            (DECAY, 0.79),
            (SEND, 0.1),
            (TIME, 0.5),
            (REPEATS, 0.45),
        ],
        PLAIN,
    ),
    (
        "Small room",
        &[
            (FREQ, 32.8),
            (FEEDBACK, -15.4),
            (BODY, 0.0258),
            (LOWPASS, 2255.0),
            (HIGHPASS, 60.3),
            (MIX, 0.5),
            (SEND, 0.0),
            (REPEATS, 0.0),
        ],
        PLAIN,
    ),
    (
        "Keys",
        &[
            (FREQ, 38.4),
            (LOWPASS, 6370.0),
            (MIX, 0.35),
            (DECAY, 0.9),
            (REPEATS, 0.45),
        ],
        [false, false, true, true, false, true],
    ),
    (
        "Resonator (IN)",
        &[
            (FREQ, 44.0),
            (FEEDBACK, -31.2),
            (BODY, 0.021),
            (LOWPASS, 8260.0),
            (MIX, 0.35),
            (DECAY, 0.9),
            (REPEATS, 0.45),
            (INPUT, 1.2),
        ],
        [false, false, true, false, false, true],
    ),
    (
        "Init",
        &[(FEEDBACK, -60.0), (MIX, 0.0), (SEND, 0.0), (REPEATS, 0.0)],
        PLAIN,
    ),
    (
        "Fifths",
        &[
            (FREQ, 38.4),
            (LOWPASS, 8260.0),
            (HIGHPASS, 24.6),
            (MIX, 0.35),
            (DECAY, 0.9),
            (REPEATS, 0.53),
        ],
        [false, false, false, false, true, true],
    ),
    (
        "Sympathetic",
        &[
            (FREQ, 27.2),
            (FEEDBACK, -3.8),
            (BODY, 0.0168),
            (LOWPASS, 3790.0),
            (HIGHPASS, 20.5),
            (MIX, 0.45),
            (DECAY, 0.94),
            (SEND, 0.15),
            (TIME, 1.83),
            (REPEATS, 0.45),
            (COUPLING, 0.4),
            (INTERVAL3, 16.0),
        ],
        [false, false, false, false, true, false],
    ),
    (
        "Chord keys",
        &[
            (FREQ, 38.4),
            (LOWPASS, 6370.0),
            (MIX, 0.1),
            (DECAY, 0.9),
            (REPEATS, 0.45),
            (COUPLING, 0.8),
        ],
        [false, false, true, true, true, true],
    ),
    (
        "Embers",
        &[
            (FREQ, 35.6),
            (BODY, 0.0131),
            (LOWPASS, 4910.0),
            (HIGHPASS, 24.6),
            (MIX, 0.2),
            (DECAY, 0.88),
            (SEND, 0.55),
            (TIME, 1.05),
            (REPEATS, 0.69),
            (DRIVE, 0.8),
            (TONE, 300.0),
        ],
        [false, true, false, false, false, true],
    ),
];

/// Built-in sound `k`, from the defaults. It keeps LEVEL: the player's.
pub fn load_factory(p: &ModuleParams, k: usize) {
    let (_, knobs, flags) = FACTORY[k.min(FACTORY.len() - 1)];
    for (i, spec) in PARAMS.iter().enumerate().filter(|&(i, _)| i != LEVEL) {
        p.set(i, spec.default);
    }
    for &(i, v) in knobs {
        p.set(i, v);
    }
    for (d, on) in flags.into_iter().enumerate() {
        set_flag(p, d, on);
    }
}

/// The noise floor that starts the loop (−90 dBFS).
const NOISE: f32 = 3.16e-5;
/// The pitch's range (FREQ's knob goes to 72, keys and intervals to 84).
const LOWEST: f32 = 16.0;
const HIGHEST: f32 = 84.0;
/// FEEDBACK with the loop closed.
const CLOSED: f32 = -60.0;
/// COUPLING's top, as a linear gain (the knob is square-law).
const MAX_COUPLING: f32 = 0.1;
/// Time constants: the pitch, BODY and the filters, and FEEDBACK.
const PITCH_GLIDE: f32 = 0.03;
const BODY_GLIDE: f32 = 0.15;
const FILTER_GLIDE: f32 = 0.007;
/// The BODY delay follows on its own: slowly from the knob, faster on a
/// note.
const BODY_LAG: f32 = 0.2;
const TUNED_LAG: f32 = 0.03;
/// Going in and out of tune with the notes takes this long.
const TUNING_FADE: f32 = 0.05;
/// What tuning to a note may give back of what the filters take there.
const MAX_MAKEUP: f32 = 8.0;
/// COUPLING's make-up while tuned, in full up to this BODY.
const COUPLING_MAKEUP: f32 = 2.5;
const COUPLING_BODY: f32 = 0.025;
/// The second and third strings fade in and out over this long.
const POLY_FADE: f32 = 0.02;
/// Three strings sum to about the level of one.
const INV_SQRT_3: f32 = 0.577_350_3;
/// The longest BODY delay (snapped and compensated), in seconds.
const LONGEST_BODY: f32 = 0.25;
/// The right side's BODY is this much shorter (four samples at 48 kHz).
const RIGHT_SHORTER: f32 = 4.0 / 48_000.0;

/// One feedback loop, stereo: noise and input into the strings, the drive,
/// the low- and high-pass, the reverb, and the BODY delay back round.
struct Loop {
    strings: [Strand; 2],
    body: [DelayLine; 2],
    /// The BODY delay now and where it is heading (samples), and how fast
    /// it follows.
    delay: f32,
    target: f32,
    lag: f32,
    offset: f32,
    lp: Biquad,
    hp: Biquad,
    verb: Verb,
    mix: f32,
    /// The band-pass on the note while tuned, and its make-up.
    track: Band4,
    tracking: bool,
    makeup: f32,
    /// What it hears of the other loops, on its note while tuned.
    couple: Band4,
    couple_makeup: f32,
    gain: f32,
    gain_target: f32,
    /// Its latest output (what the others hear).
    tap: [f32; 2],
    rng: Rng,
}

impl Loop {
    fn new(sr: f32, seed: u32) -> Self {
        let body = (LONGEST_BODY * sr) as usize + 8;
        Self {
            strings: [
                Strand::new(sr, note_hz(LOWEST) * 0.9),
                Strand::new(sr, note_hz(LOWEST) * 0.9),
            ],
            body: [DelayLine::new(body), DelayLine::new(body)],
            delay: 0.01 * sr,
            target: 0.01 * sr,
            lag: 1.0 / (BODY_LAG * sr),
            offset: RIGHT_SHORTER * sr,
            lp: Biquad::default(),
            hp: Biquad::default(),
            verb: Verb::new(sr),
            mix: 0.0,
            track: Band4::default(),
            tracking: false,
            makeup: 1.0,
            couple: Band4::default(),
            couple_makeup: 1.0,
            gain: 0.0,
            gain_target: 0.0,
            tap: [0.0; 2],
            rng: Rng::new(seed),
        }
    }

    fn clear(&mut self) {
        for s in &mut self.strings {
            s.clear();
        }
        for line in &mut self.body {
            for _ in 0..line.len() {
                line.push(0.0);
            }
        }
        self.lp.clear();
        self.hp.clear();
        self.verb.clear();
        self.track.clear();
        self.couple.clear();
        self.tap = [0.0; 2];
        self.delay = self.target;
    }

    /// Sample `i` of the block, with `noise` the floor's level now.
    #[inline]
    fn process(
        &mut self,
        noise: f32,
        input: f32,
        couple: [f32; 2],
        k_gain: f32,
        i: usize,
    ) -> [f32; 2] {
        self.delay += (self.target - self.delay) * self.lag;
        self.gain += (self.gain_target - self.gain) * k_gain;
        let floor = self.rng.bipolar() * noise;
        let back_l = self.body[0].tap(self.delay - 1.0);
        let back_r = self.body[1].tap((self.delay - self.offset).max(1.0) - 1.0);
        let l = drive(self.strings[0].process(back_l + floor + input + couple[0]));
        let r = drive(self.strings[1].process(back_r + floor + input + couple[1]));
        let l = self.hp.process(self.lp.process(l, 0), 0);
        let r = self.hp.process(self.lp.process(r, 1), 1);
        let (vl, vr) = self.verb.process(l, r, i);
        let l = l - (l - vl) * self.mix;
        let r = r - (r - vr) * self.mix;
        self.tap = [l, r];
        let (mut fl, mut fr) = (l * self.gain, r * self.gain);
        if self.tracking {
            fl = self.track.process(fl, 0) * self.makeup;
            fr = self.track.process(fr, 1) * self.makeup;
        }
        self.body[0].push(fl);
        self.body[1].push(fr);
        [l, r]
    }
}

pub struct Larsen {
    params: Arc<ModuleParams>,
    midi: Arc<MidiState>,
    midi_cursor: u32,
    sr: f32,
    loops: [Loop; STRINGS],
    echo: Echo,
    keys: Keys,
    freq_seen: f32,
    /// Each string's note, BODY and the filters, gliding.
    note: [f32; STRINGS],
    body: f32,
    lp_hz: f32,
    hp_hz: f32,
    /// How far each string is tuned to its note (0…1).
    blend: [f32; STRINGS],
    /// Smoothed per sample: the echo's send, the output, the input and
    /// COUPLING.
    send: f32,
    level: f32,
    input: f32,
    coupling: f32,
    /// The floor of noise, faded out while nothing plays.
    noise: f32,
    /// The second and third strings: fading in or out (0…1), and running.
    poly: f32,
    poly_running: bool,
    limit: f32,
    levels: [f32; STRINGS],
    started: bool,
    quiet: Quiet,
    write: usize,
}

impl Larsen {
    pub fn new(params: Arc<ModuleParams>, midi: Arc<MidiState>, sample_rate: f32) -> Self {
        let sr = sample_rate;
        params.viz[viz::SR].set(sr);
        let freq = params.get(FREQ);
        Self {
            midi_cursor: midi.stream_head(),
            midi,
            sr,
            loops: std::array::from_fn(|s| Loop::new(sr, 0x001A_75E2 + 977 * s as u32)),
            echo: Echo::new(sr),
            keys: Keys::new(),
            freq_seen: freq,
            note: [freq; STRINGS],
            body: params.get(BODY),
            lp_hz: params.get(LOWPASS),
            hp_hz: params.get(HIGHPASS),
            blend: [0.0; STRINGS],
            send: 0.0,
            level: 0.0,
            input: 0.0,
            coupling: 0.0,
            noise: 0.0,
            poly: 0.0,
            poly_running: false,
            limit: 0.0,
            levels: [0.0; STRINGS],
            started: false,
            quiet: Quiet::new(((echo::LONGEST + 1.0) * sr) as usize),
            write: 0,
            params,
        }
    }

    /// After a non-finite sample: everything cleared, the settings kept.
    fn recover(&mut self) {
        for l in &mut self.loops {
            l.clear();
        }
        self.echo.clear();
        self.limit = 0.0;
    }
}

impl Processor for Larsen {
    fn process(&mut self, ctx: &ProcessCtx, inputs: &Inputs, outputs: &mut [Buf], n: usize) {
        let p = self.params.clone();
        let sr = self.sr;
        let secs = n as f32 / sr;
        let first = !self.started;
        self.started = true;

        // MIDI: the cable if patched, else the keyboards.
        let poly = flag(&p, data::POLY);
        self.keys.set_poly(poly);
        let keys = &mut self.keys;
        if inputs.midi_connected {
            for e in inputs.midi.events() {
                keys.handle(e.data);
            }
            self.midi_cursor = self.midi.stream_head();
        } else {
            self.midi
                .read_stream(&mut self.midi_cursor, |msg| keys.handle(msg));
        }
        // FREQ moved with no key down takes the pitch back.
        let freq = p.get(FREQ);
        if freq != self.freq_seen {
            if !self.keys.any_held() {
                self.keys.played = false;
            }
            self.freq_seen = freq;
        }
        let midi = flag(&p, data::PITCH) && self.keys.played && self.keys.latest().is_some();
        let tuned = midi || (poly && flag(&p, data::LOCK));
        let gated = flag(&p, data::GATE);
        let held = self.keys.any_held();
        // Like every source it plays with the transport (or keys held).
        let running = ctx.playing || held;

        // Each string's note, and whether its loop is open.
        let mut target = [
            freq,
            freq + p.get(INTERVAL2).round(),
            freq + p.get(INTERVAL3).round(),
        ];
        let mut open = [running; STRINGS];
        for s in 0..STRINGS {
            let (note, down) = match (midi, poly) {
                (true, true) => self.keys.string(s),
                (true, false) => (self.keys.latest(), held),
                _ => (None, held),
            };
            if midi {
                match note {
                    Some(k) => target[s] = k as f32 + self.keys.bend,
                    // Never played: it stays closed.
                    None => (target[s], open[s]) = (self.note[s], false),
                }
            }
            if gated && !down {
                open[s] = false;
            }
            target[s] = target[s].clamp(LOWEST, HIGHEST);
        }

        let input = inputs.get(0);
        let input_silent = Quiet::silent(input, n);
        if !running && input_silent && self.quiet.resting() {
            // Died away, and nothing to wake it: nothing to compute.
            silence(outputs, n);
            for s in 0..STRINGS {
                p.viz[viz::LEVEL + s].set(0.0);
                p.viz[viz::OPEN + s].set(0.0);
            }
            return;
        }

        if poly && !self.poly_running {
            // The two strings start where they are meant to, not gliding.
            self.poly_running = true;
            self.note[1..].copy_from_slice(&target[1..]);
        }
        let glide = |tau: f32| {
            if first {
                1.0
            } else {
                1.0 - (-secs / tau).exp()
            }
        };
        for (note, t) in self.note.iter_mut().zip(target) {
            *note += (t - *note) * glide(PITCH_GLIDE);
        }
        self.body += (p.get(BODY) - self.body) * glide(BODY_GLIDE);
        self.lp_hz += (p.get(LOWPASS) - self.lp_hz) * glide(FILTER_GLIDE);
        self.hp_hz += (p.get(HIGHPASS) - self.hp_hz) * glide(FILTER_GLIDE);

        // Each loop's settings for the block.
        let strings = if self.poly_running { STRINGS } else { 1 };
        let feedback = p.get(FEEDBACK);
        let (mix, decay) = (p.get(MIX), p.get(DECAY));
        for (s, lp) in self.loops.iter_mut().enumerate().take(strings) {
            let f = note_hz(self.note[s]);
            let w = TAU * f / sr;
            lp.lp.set(Kind::Low, self.lp_hz, 0.9, sr);
            lp.hp.set(Kind::High, self.hp_hz, 0.9, sr);
            let to = if tuned { 1.0 } else { 0.0 };
            self.blend[s] = if first || !poly {
                to
            } else {
                self.blend[s] + (to - self.blend[s]).clamp(-secs / TUNING_FADE, secs / TUNING_FADE)
            };
            // The string's filters lag a little: shorten it by as much, so
            // that it rings on the note.
            let string = Strand::response(sr, w);
            let period = (1.0 + self.blend[s] * string.arg() / TAU) / f;
            for st in &mut lp.strings {
                st.set_period(period * sr);
            }
            let (delay, lag) = if tuned {
                // BODY on a whole number of periods, less what the filters
                // in the loop lag there, and a band-pass on the note.
                let t = 1.0 / f;
                let whole = (self.body / t).round().max(2.0);
                let outer = string.mul(lp.lp.response(w)).mul(lp.hp.response(w));
                lp.track.set(f, sr);
                lp.couple.set(f, sr);
                if !lp.tracking {
                    lp.track.clear();
                    lp.tracking = true;
                }
                lp.makeup = (1.0 / outer.abs().max(1e-3)).min(MAX_MAKEUP);
                lp.couple_makeup = COUPLING_MAKEUP * (COUPLING_BODY / self.body).min(1.0).powi(3);
                (
                    (whole * t + outer.arg() / (TAU * f)).max(1.0 / sr),
                    TUNED_LAG,
                )
            } else {
                lp.tracking = false;
                // The others' BODY follows their interval.
                (
                    self.body * ((self.note[0] - self.note[s]) / 12.0).exp2(),
                    BODY_LAG,
                )
            };
            lp.target = (delay * sr).min(LONGEST_BODY * sr);
            lp.lag = 1.0 / (lag * sr);
            lp.gain_target = db_gain(if open[s] { feedback } else { CLOSED });
            lp.mix = mix;
            lp.verb.begin(decay, n);
            if first {
                lp.delay = lp.target;
                lp.gain = lp.gain_target;
            }
            p.viz[viz::BODY + s].set(lp.target / sr);
        }
        let shape = Shape {
            drive: p.get(DRIVE),
            tone: p.get(TONE),
            width: p.get(WIDTH),
            level: db_gain(p.get(GAIN)),
        };
        self.echo.set(
            echo_time(&p),
            p.get(REPEATS),
            flag(&p, data::PONG),
            shape,
            first,
        );

        let k = ctx.smooth;
        let k_gain = 1.0 - (-1.0 / (FILTER_GLIDE * sr)).exp();
        let k_noise = 1.0 - (-1.0 / (0.05 * sr)).exp();
        let poly_step = 1.0 / (POLY_FADE * sr);
        let (send_t, input_t) = (p.get(SEND) * p.get(SEND), p.get(INPUT));
        let level_t = p.get(LEVEL) * reverb_trim(mix, decay);
        let coupling_t = MAX_COUPLING * p.get(COUPLING) * p.get(COUPLING);
        let noise_t = if running || !input_silent { NOISE } else { 0.0 };
        if first {
            (self.send, self.level, self.input, self.coupling) =
                (send_t, level_t, input_t, coupling_t);
        }
        // The limiter: fast to catch a peak, a second to let go.
        let attack = 1.0 - (-1.0 / (0.0004 * sr)).exp();
        let release = 1.0 / sr;
        let receive = open.map(|o| if o { 1.0 } else { 0.0 });
        let mut levels = [0.0f32; STRINGS];
        let mut peak = 0.0f32;
        let out = &mut outputs[0];
        for i in 0..n {
            self.send += (send_t - self.send) * k;
            self.level += (level_t - self.level) * k;
            self.input += (input_t - self.input) * k;
            self.coupling += (coupling_t - self.coupling) * k;
            self.noise += (noise_t - self.noise) * k_noise;
            let x = input.map_or(0.0, |b| 0.5 * (b[0][i] + b[1][i])) * self.input;
            let (l, r) = if self.poly_running {
                self.poly = if poly {
                    (self.poly + poly_step).min(1.0)
                } else {
                    (self.poly - poly_step).max(0.0)
                };
                // Each loop hears the other two as they were a sample ago.
                let taps = [self.loops[0].tap, self.loops[1].tap, self.loops[2].tap];
                let weight = [1.0, self.poly, self.poly];
                let mut outs = [[0.0f32; 2]; STRINGS];
                for (s, o) in outs.iter_mut().enumerate() {
                    let (a, b) = ((s + 1) % STRINGS, (s + 2) % STRINGS);
                    let gain = self.coupling * receive[s];
                    let lp = &mut self.loops[s];
                    let mut c = std::array::from_fn(|ch| {
                        gain * (weight[a] * taps[a][ch] + weight[b] * taps[b][ch])
                    });
                    if tuned {
                        c = [
                            lp.couple.process(c[0], 0) * lp.couple_makeup,
                            lp.couple.process(c[1], 1) * lp.couple_makeup,
                        ];
                    }
                    *o = lp.process(self.noise, x, c, k_gain, i);
                    levels[s] = levels[s].max(o[0].abs()).max(o[1].abs());
                }
                let scale = 1.0 + self.poly * (INV_SQRT_3 - 1.0);
                let mono =
                    |ch: usize| (outs[0][ch] + self.poly * (outs[1][ch] + outs[2][ch])) * scale;
                (mono(0), mono(1))
            } else {
                let o = self.loops[0].process(self.noise, x, [0.0; 2], k_gain, i);
                levels[0] = levels[0].max(o[0].abs()).max(o[1].abs());
                (o[0], o[1])
            };
            let (wl, wr) = self.echo.process(l * self.send, r * self.send);
            let gain = 0.5 * 0.7 * self.level;
            let (ol, or) = ((l + wl) * gain, (r + wr) * gain);
            let a = ol.abs().max(or.abs());
            self.limit += (a - self.limit) * if a > self.limit { attack } else { release };
            let g = if self.limit > 1.0 {
                1.0 / self.limit
            } else {
                1.0
            };
            let (ol, or) = (
                soft_limit(ol * g).clamp(-1.0, 1.0),
                soft_limit(or * g).clamp(-1.0, 1.0),
            );
            out[0][i] = ol;
            out[1][i] = or;
            peak = peak.max(ol.abs()).max(or.abs());
            p.viz[viz::RING + self.write].set(0.5 * (ol + or));
            self.write = (self.write + 1) % RING_LEN;
        }
        if self.poly_running && !poly && self.poly == 0.0 {
            self.poly_running = false;
            self.loops[1].clear();
            self.loops[2].clear();
        }

        // A loop this wild must never keep a NaN.
        if !out[0][..n]
            .iter()
            .chain(&out[1][..n])
            .all(|s| s.is_finite())
        {
            self.recover();
            silence(outputs, n);
            peak = 0.0;
        }
        self.quiet.update(input_silent && !running, peak, n);

        for s in 0..STRINGS {
            let fall = (-secs / 0.15).exp();
            self.levels[s] = levels[s].max(self.levels[s] * fall);
            p.viz[viz::NOTE + s].set(self.note[s]);
            p.viz[viz::LEVEL + s].set(self.levels[s].min(1.0));
            p.viz[viz::OPEN + s].set(if open[s] && s < strings { 1.0 } else { 0.0 });
            p.viz[viz::RUNS + s].set(if s == 0 { 1.0 } else { self.poly });
        }
        p.viz[viz::MIDI].set(if midi { 1.0 } else { 0.0 });
        p.viz[viz::TUNED].set(if tuned { 1.0 } else { 0.0 });
        p.viz[viz::ECHO].set(self.echo.time());
        p.viz[viz::PONG].set(self.echo.pong());
        p.viz[viz::WRITE].set(self.write as f32);
    }
}
