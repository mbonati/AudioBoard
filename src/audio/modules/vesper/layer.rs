//! What makes Vesper's layers sound: [`Sampler`] plays a bank's loops as
//! notes (Pad, Bass, Noise, Ambience), and [`Pings`] strikes Shimmer's small
//! bells.

use super::library::Bank;
use super::sounds::Ping;
use crate::audio::engine::BLOCK;
use crate::audio::fastmath;
use crate::audio::midi::note_hz;
use crate::audio::modules::{Rng, Wander, pan_gains};

/// Voices a sampler can have sounding, tails included.
const CAP: usize = 24;
/// Notes it can be asked for at once.
const MAX_NOTES: usize = 16;
/// When a new sound comes in, the old one's voices fade over this long at
/// most, whatever RELEASE says.
const SWAP_FADE: f32 = 2.0;
/// A voice hurried off to make room fades over this long.
const HURRY: f32 = 0.012;
/// How long each voice's drift takes to reach a new place.
const DRIFT_SEGMENT: f32 = 9.0;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Off,
    Rising,
    Holding,
    Falling,
}

struct Voice {
    stage: Stage,
    /// Which of the sampler's two banks, and which of its loops.
    bank: usize,
    table: usize,
    note: f32,
    /// Read position, a fixed-point number of samples (32.32).
    pos: u64,
    /// Through the attack or release (0…1), and the envelope where it began.
    t: f32,
    from: f32,
    /// The envelope, and the gain after the sway (what the block ramps from).
    env: f32,
    gain: f32,
    hurry: bool,
    /// How long it has held.
    held: f32,
    pan: f32,
    drift: Wander,
    sway: Wander,
}

impl Voice {
    fn new(rng: &mut Rng) -> Self {
        Self {
            stage: Stage::Off,
            bank: 0,
            table: 0,
            note: 0.0,
            pos: 0,
            t: 0.0,
            from: 0.0,
            env: 0.0,
            gain: 0.0,
            hurry: false,
            held: 0.0,
            pan: 0.0,
            drift: Wander::new(rng),
            sway: Wander::new(rng),
        }
    }

    fn sounding(&self) -> bool {
        matches!(self.stage, Stage::Rising | Stage::Holding)
    }

    fn fall(&mut self) {
        self.stage = Stage::Falling;
        self.t = 0.0;
        self.from = self.env;
    }

    fn rise(&mut self) {
        self.stage = Stage::Rising;
        self.t = 0.0;
        self.from = self.env;
        self.hurry = false;
        self.held = 0.0;
    }
}

fn same(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

/// How a sampler's voices move this block.
#[derive(Clone, Copy)]
pub struct Shape {
    pub attack: f32,
    pub release: f32,
    /// How long a note holds before it lets go by itself (infinite: until
    /// it's over).
    pub hold: f32,
    /// How far each voice's pitch drifts, in semitones.
    pub drift: f32,
    /// How far each voice's level sways (0…1).
    pub sway: f32,
    /// How far the voices spread across the stereo field (0…1).
    pub spread: f32,
}

/// Plays a sound's loops as notes: it is told which notes should be
/// sounding, and sees to it, keeping a note that stays, and letting a note
/// that goes ring out on its release.
pub struct Sampler {
    voices: Vec<Voice>,
    /// Two banks: the one playing, and the one its voices are fading out of
    /// while a new sound comes in.
    banks: [Option<Box<Bank>>; 2],
    active: usize,
    wanted: [f32; MAX_NOTES],
    len: usize,
    dirty: bool,
    rng: Rng,
}

impl Sampler {
    pub fn new(seed: u32) -> Self {
        let mut rng = Rng::new(seed);
        Self {
            voices: (0..CAP).map(|_| Voice::new(&mut rng)).collect(),
            banks: [None, None],
            active: 0,
            wanted: [0.0; MAX_NOTES],
            len: 0,
            dirty: false,
            rng,
        }
    }

    /// The sound playing, if one has arrived yet.
    pub fn loaded(&self) -> Option<usize> {
        self.banks[self.active].as_ref().map(|b| b.sound)
    }

    /// Follows the notes (else it plays one fixed pitch).
    pub fn tonal(&self) -> bool {
        self.banks[self.active].as_ref().is_some_and(|b| b.tonal)
    }

    /// The note at which the sound plays as it was made.
    pub fn home(&self) -> f32 {
        self.banks[self.active]
            .as_ref()
            .map_or(48.0, |b| b.tables[0].base)
    }

    /// There is room for a new sound: the last swap has finished.
    pub fn can_install(&self) -> bool {
        self.banks[self.active].is_none() || self.banks[1 - self.active].is_none()
    }

    /// A new sound (see [`Self::can_install`]). The voices on the old one
    /// fade, and new ones start on this.
    pub fn install(&mut self, bank: Box<Bank>) {
        if self.banks[self.active].is_some() {
            self.active = 1 - self.active;
        }
        self.banks[self.active] = Some(bank);
        self.dirty = true;
    }

    /// The old sound, once its last voice has gone: to be freed elsewhere.
    pub fn finished(&mut self) -> Option<Box<Bank>> {
        let old = 1 - self.active;
        if self.banks[old].is_some()
            && !self
                .voices
                .iter()
                .any(|v| v.stage != Stage::Off && v.bank == old)
        {
            return self.banks[old].take();
        }
        None
    }

    /// Takes back what [`Self::finished`] gave, to try again next block.
    pub fn keep(&mut self, bank: Box<Bank>) {
        self.banks[1 - self.active] = Some(bank);
    }

    /// The notes that should be sounding.
    pub fn set_notes(&mut self, notes: &[f32]) {
        let notes = &notes[..notes.len().min(MAX_NOTES)];
        if notes.len() != self.len || notes.iter().zip(&self.wanted).any(|(a, b)| !same(*a, *b)) {
            self.wanted[..notes.len()].copy_from_slice(notes);
            self.len = notes.len();
            self.dirty = true;
        }
    }

    pub fn any(&self) -> bool {
        self.voices.iter().any(|v| v.stage != Stage::Off)
    }

    /// A new chord has begun: notes that let go after HOLD strike again, even
    /// if the chord before had them too.
    pub fn strike_again(&mut self) {
        for v in self.voices.iter_mut().filter(|v| v.sounding()) {
            v.held = 0.0;
        }
        self.dirty = true;
    }

    /// Voices now match the notes wanted. If there's no voice free for a
    /// note, the quietest tail is hurried off, and the note comes next block.
    fn reconcile(&mut self, spread: f32) {
        let active = self.active;
        let Some(bank) = self.banks[active].as_deref() else {
            return;
        };
        let wanted = &self.wanted[..self.len];
        for v in &mut self.voices {
            if v.sounding() && (v.bank != active || !wanted.iter().any(|&n| same(n, v.note))) {
                v.fall();
            }
        }
        let mut pending = false;
        for &note in wanted {
            let on_note = |v: &Voice| v.bank == active && same(v.note, note);
            if self.voices.iter().any(|v| v.sounding() && on_note(v)) {
                continue;
            }
            if let Some(v) = self
                .voices
                .iter_mut()
                .find(|v| v.stage == Stage::Falling && on_note(v))
            {
                v.rise();
                continue;
            }
            if let Some(v) = self.voices.iter_mut().find(|v| v.stage == Stage::Off) {
                v.bank = active;
                v.table = bank.pick(note);
                v.note = note;
                let mask = bank.tables[v.table].mask as u64;
                v.pos = ((self.rng.unit() * mask as f32) as u64) << 32;
                v.env = 0.0;
                v.gain = 0.0;
                v.pan = spread * self.rng.bipolar();
                v.rise();
                continue;
            }
            let quiet = self
                .voices
                .iter_mut()
                .filter(|v| v.stage == Stage::Falling)
                .min_by(|a, b| a.env.total_cmp(&b.env));
            if let Some(v) = quiet {
                v.hurry = true;
            }
            pending = true;
        }
        self.dirty = pending;
    }

    /// Adds this block's voices into `out`. Returns whether any sounded.
    pub fn render(&mut self, s: &Shape, out: &mut [[f32; BLOCK]; 2], n: usize, secs: f32) -> bool {
        if self.dirty {
            self.reconcile(s.spread);
        }
        let inv = 1.0 / n as f32;
        let mut any = false;
        let active = self.active;
        for v in &mut self.voices {
            if v.stage == Stage::Off {
                continue;
            }
            let Some(bank) = self.banks[v.bank].as_deref() else {
                v.stage = Stage::Off;
                continue;
            };
            any = true;
            // The envelope.
            match v.stage {
                Stage::Rising => {
                    v.t = (v.t + secs / s.attack.max(1e-3)).min(1.0);
                    v.env = v.from + (1.0 - v.from) * fastmath::sin_turns(v.t * 0.25);
                    if v.t >= 1.0 {
                        v.stage = Stage::Holding;
                        v.env = 1.0;
                    }
                }
                Stage::Falling => {
                    let time = if v.hurry {
                        HURRY
                    } else if v.bank != active {
                        s.release.min(SWAP_FADE)
                    } else {
                        s.release
                    };
                    v.t = (v.t + secs / time.max(1e-3)).min(1.0);
                    let left = 1.0 - v.t;
                    v.env = v.from * left * left;
                    if v.t >= 1.0 {
                        v.stage = Stage::Off;
                        v.env = 0.0;
                        v.hurry = false;
                    }
                }
                Stage::Holding => {
                    v.held += secs;
                    if v.held > s.hold {
                        v.fall();
                    }
                }
                Stage::Off => {}
            }
            // A voice's own slow life: as much as the sound has (a pipe
            // doesn't wander as a choir does).
            let step = secs / DRIFT_SEGMENT;
            let pitch = v.note + bank.life * s.drift * v.drift.tick(&mut self.rng, step);
            let sway = 1.0 + bank.life * s.sway * v.sway.tick(&mut self.rng, step * 0.7);
            let table = &bank.tables[v.table];
            let inc = (fastmath::exp2((pitch - table.base) / 12.0) as f64 * 4_294_967_296.0) as u64;
            let (gl, gr) = pan_gains(v.pan);
            let target = v.env * sway;
            let (mut g, dg) = (v.gain, (target - v.gain) * inv);
            let (left, right, mask) = (&*table.left, &*table.right, table.mask);
            let mut pos = v.pos;
            let [out_l, out_r] = out;
            for (ol, or) in out_l[..n].iter_mut().zip(&mut out_r[..n]) {
                let i = (pos >> 32) as usize;
                let f = (pos as u32) as f32 * (1.0 / 4_294_967_296.0);
                let (a, b, c, d) = (
                    i.wrapping_sub(1) & mask,
                    i & mask,
                    (i + 1) & mask,
                    (i + 2) & mask,
                );
                *ol += g * gl * hermite(left[a], left[b], left[c], left[d], f);
                *or += g * gr * hermite(right[a], right[b], right[c], right[d], f);
                pos = pos.wrapping_add(inc);
                g += dg;
            }
            v.pos = pos;
            v.gain = target;
        }
        any
    }
}

/// Between `y0` and `y1`, by a cubic through the four samples.
#[inline]
fn hermite(ym1: f32, y0: f32, y1: f32, y2: f32, f: f32) -> f32 {
    let c = (y1 - ym1) * 0.5;
    let v = y0 - y1;
    let w = c + v;
    let a = w + v + (y2 - y0) * 0.5;
    ((a * f - (w + a)) * f + c) * f + y0
}

/// Pings at once (more take the place of the quietest).
const PING_VOICES: usize = 12;

#[derive(Clone, Copy, Default)]
struct PingVoice {
    on: bool,
    carrier: f32,
    modulator: f32,
    c_inc: f32,
    m_inc: f32,
    index: f32,
    env: f32,
    /// Added to the envelope each sample while rising; then what it is
    /// multiplied by.
    rise: f32,
    rising: bool,
    fall: f32,
    gl: f32,
    gr: f32,
}

/// Shimmer: small struck notes, bright at first and then pure.
pub struct Pings {
    voices: [PingVoice; PING_VOICES],
}

impl Pings {
    pub fn new() -> Self {
        Self {
            voices: [PingVoice::default(); PING_VOICES],
        }
    }

    pub fn any(&self) -> bool {
        self.voices.iter().any(|v| v.on)
    }

    /// Strikes `note`. `attack` and `release` are the layer's own, `pan` its
    /// place, `level` how hard.
    #[allow(clippy::too_many_arguments)]
    pub fn strike(
        &mut self,
        ping: &Ping,
        note: f32,
        attack: f32,
        release: f32,
        pan: f32,
        level: f32,
        sr: f32,
    ) {
        let hz = note_hz(note).min(sr * 0.42);
        let k = (0..PING_VOICES).find(|&k| !self.voices[k].on).or_else(|| {
            (0..PING_VOICES).min_by(|&a, &b| self.voices[a].env.total_cmp(&self.voices[b].env))
        });
        let Some(k) = k else { return };
        let tau = (release * ping.ring).max(0.02) * sr;
        let (gl, gr) = pan_gains(pan);
        let (gl, gr) = (gl * level, gr * level);
        self.voices[k] = PingVoice {
            on: true,
            carrier: 0.0,
            modulator: 0.0,
            c_inc: hz / sr,
            m_inc: (hz * ping.ratio).min(sr * 0.45) / sr,
            index: ping.index,
            env: 0.0,
            rise: 1.0 / (attack * sr).max(8.0),
            rising: true,
            fall: (-1.0 / tau).exp(),
            gl,
            gr,
        };
    }

    /// Adds this block's pings into `out`.
    #[allow(clippy::needless_range_loop)] // the two sides at once
    pub fn render(&mut self, out: &mut [[f32; BLOCK]; 2], n: usize) {
        for v in self.voices.iter_mut().filter(|v| v.on) {
            for i in 0..n {
                if v.rising {
                    v.env += v.rise;
                    if v.env >= 1.0 {
                        v.env = 1.0;
                        v.rising = false;
                    }
                } else {
                    v.env *= v.fall;
                }
                let bend = if v.index > 0.0 {
                    v.index * v.env * fastmath::sin_turns(v.modulator)
                } else {
                    0.0
                };
                let y = fastmath::sin_turns((v.carrier + bend).rem_euclid(1.0)) * v.env;
                v.carrier += v.c_inc;
                if v.carrier >= 1.0 {
                    v.carrier -= 1.0;
                }
                v.modulator += v.m_inc;
                if v.modulator >= 1.0 {
                    v.modulator -= 1.0;
                }
                out[0][i] += y * v.gl;
                out[1][i] += y * v.gr;
            }
            if !v.rising && v.env < 2e-4 {
                v.on = false;
            }
        }
    }
}
