//! ALEA: throws of the dice for Fjord. *Alea* is Latin for the die and for
//! chance and fortune, and it gave aleatoric music its name: pieces left
//! partly to chance. One throw makes new sounds, new steps, or both, and
//! CHANCE sets how far it goes: a little, and it nudges what is there; all
//! the way, and it starts afresh.
//!
//! New sounds are bred rather than drawn blind: each section of a channel
//! (noise, tone, click, mixer, the tone's envelopes) comes from the same
//! channel of a factory program (so channel 1 stays kick-like in a kit,
//! a percussion pad in a percussion program), the sections crossed between
//! programs, then jittered, a few choices (waveform, filter type, click
//! type, gates) thrown anew, now and then the tone's ADSRs switched on.
//! New steps follow what each channel's sound suggests it is (a kick, a
//! snare, a hihat, a tom or percussion): kicks on the beat with a few
//! syncopations, snares on the backbeat with ghost notes, hihats on a
//! steady grid with accents, toms and percussion on Euclidean rhythms with
//! a fill now and then.

use super::bank::FACTORY;
use super::voice::Sound;
use super::{CHANNELS, MAX_LEN, RANGES, ch, data, knob, length, set_sound, set_step, snap, sound};
use crate::audio::modules::{ModuleParams, Rng};
use crate::audio::params::DIVISIONS;

/// What a channel plays in a groove, guessed from its sound.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Kick,
    Snare,
    Hat,
    Tom,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Role::Kick => "kick",
            Role::Snare => "snare",
            Role::Hat => "hihat",
            Role::Tom => "tom · perc",
        }
    }
}

/// The c3 waveform (six squares, metallic).
const METAL: f32 = 14.0;

pub fn role(s: &Sound) -> Role {
    let (tone, noise) = (s[knob::TONE], s[knob::NOISE]);
    let metal = s[knob::T_WAVE].round() == METAL && s[knob::PITCH] >= 78.0 && tone > 0.0;
    if (noise >= tone && noise > 0.0 && s[knob::N_COLOR] >= 4.0) || metal {
        Role::Hat
    } else if tone >= noise && s[knob::PITCH] < 40.0 {
        Role::Kick
    } else if noise >= 5.0 {
        Role::Snare
    } else {
        Role::Tom
    }
}

/// The knobs that are choices rather than amounts: thrown anew, not moved.
fn choice(k: usize) -> bool {
    matches!(
        k,
        knob::N_TYPE
            | knob::N_GATE
            | knob::T_WAVE
            | knob::T_GATE
            | knob::C_TYPE
            | knob::C_GATE
            | knob::AMP_ENV
    )
}

/// How far a knob may wander from the bred sound, in its own units.
fn jitter(k: usize) -> f32 {
    match k {
        knob::PITCH => 5.0,
        knob::BEND => 4.0,
        knob::LEVEL => 6.0,
        knob::N_DECAY | knob::T_DECAY | knob::AMP_D | knob::AMP_R | knob::FLT_D | knob::FLT_R => {
            6.0
        }
        knob::C_DECAY => 3.0,
        knob::N_FILTER | knob::T_FILTER => 14.0,
        knob::TONE | knob::NOISE | knob::CLICK => 1.5,
        _ => {
            let (_, lo, hi, _) = RANGES[k];
            (hi - lo) * 0.15
        }
    }
}

/// The sections a sound is bred from, each from one program.
const SECTIONS: [std::ops::Range<usize>; 6] = [0..7, 7..17, 17..20, 20..24, 24..29, 29..34];

fn below(rng: &mut Rng, n: usize) -> usize {
    ((rng.unit() * n as f32) as usize).min(n.saturating_sub(1))
}

fn between(rng: &mut Rng, lo: f32, hi: f32) -> f32 {
    lo + (hi - lo) * rng.unit()
}

/// A new sound for channel `c`.
pub fn bred(rng: &mut Rng, c: usize) -> Sound {
    use knob::*;
    let mut s = [0.0; COUNT];
    let mut donor = below(rng, FACTORY.len());
    for section in SECTIONS {
        if rng.unit() < 0.5 {
            donor = below(rng, FACTORY.len());
        }
        for k in section {
            s[k] = FACTORY[donor].channels[c][k];
        }
    }
    for (k, v) in s.iter_mut().enumerate() {
        let (_, lo, hi, _) = RANGES[k];
        if choice(k) {
            if k != AMP_ENV && rng.unit() < 0.2 {
                *v = (lo + below(rng, (hi - lo) as usize + 1) as f32).min(hi);
            }
        } else {
            *v += rng.bipolar() * jitter(k);
        }
    }
    // Now and then the tone's envelopes join in.
    if rng.unit() < 0.15 {
        s[AMP_ENV] = 1.0;
        s[AMP_A] = between(rng, 0.0, 30.0);
        s[AMP_D] = between(rng, 15.0, 40.0);
        s[AMP_S] = between(rng, 0.0, 90.0);
        s[AMP_R] = between(rng, 10.0, 35.0);
    }
    if rng.unit() < 0.2 {
        let opens = rng.unit() < 0.7;
        s[FLT_AMT] = between(rng, 3.0, 10.0) * if opens { 1.0 } else { -1.0 };
        s[FLT_A] = between(rng, 0.0, 20.0);
        s[FLT_D] = between(rng, 10.0, 35.0);
        s[FLT_S] = between(rng, 0.0, 60.0);
        s[FLT_R] = between(rng, 10.0, 30.0);
        if opens {
            s[T_FILTER] = s[T_FILTER].min(between(rng, 25.0, 60.0));
        }
    }
    // Always heard, never too loud or too faint.
    if s[TONE].max(s[NOISE]).max(s[CLICK]) < 3.0 {
        s[TONE] = 8.0;
    }
    s[LEVEL] = s[LEVEL].clamp(60.0, 92.0);
    std::array::from_fn(|k| snap(k, s[k]))
}

/// A throw for the sounds of `channels`: each moves `amount` (0…1) of the
/// way to a newly bred sound, and takes its choices with that chance.
pub fn throw_sound(p: &ModuleParams, seed: u32, amount: f32, channels: &[usize]) {
    let mut rng = Rng::new(seed);
    let amount = amount.clamp(0.0, 1.0);
    for &c in channels.iter().filter(|&&c| c < CHANNELS) {
        let now = sound(p, c);
        let new = bred(&mut rng, c);
        let mut out = now;
        for k in 0..knob::COUNT {
            if choice(k) {
                if rng.unit() < amount {
                    out[k] = new[k];
                }
            } else {
                out[k] = now[k] + (new[k] - now[k]) * amount;
            }
        }
        set_sound(p, c, &out);
    }
}

/// A groove of `len` steps (0 off … 3 accent) for a role, `per_beat` steps
/// to a beat.
pub fn groove(rng: &mut Rng, role: Role, len: usize, per_beat: usize) -> [usize; MAX_LEN] {
    let mut g = [0; MAX_LEN];
    let beat = per_beat.max(1);
    let half = (beat / 2).max(1);
    let bar = beat * 4;
    let len = len.min(MAX_LEN);
    match role {
        Role::Kick => {
            for (s, v) in g.iter_mut().enumerate().take(len) {
                *v = if s % bar == 0 {
                    3
                } else if s % beat == 0 {
                    if rng.unit() < 0.55 { 2 } else { 0 }
                } else if s % half == 0 && rng.unit() < 0.2 {
                    1 + below(rng, 2)
                } else if rng.unit() < 0.05 {
                    1
                } else {
                    0
                };
            }
        }
        Role::Snare => {
            for (s, v) in g.iter_mut().enumerate().take(len) {
                *v = if s % (2 * beat) == beat {
                    if rng.unit() < 0.95 { 3 } else { 2 }
                } else if ((s + 1) % (2 * beat) == beat && rng.unit() < 0.25) || rng.unit() < 0.1 {
                    // Ghost notes, more often just before a backbeat.
                    1
                } else {
                    0
                };
            }
        }
        Role::Hat => {
            // Eighths, sixteenths, or the offbeats only.
            let style = below(rng, 3);
            for (s, v) in g.iter_mut().enumerate().take(len) {
                let on = match style {
                    0 => s % half == 0,
                    1 => true,
                    _ => s % beat == half % beat,
                };
                *v = if !on || rng.unit() < 0.08 {
                    0
                } else if style == 2 {
                    3
                } else if s % beat == 0 {
                    2
                } else if rng.unit() < 0.2 {
                    3
                } else {
                    1
                };
            }
        }
        Role::Tom => {
            // A Euclidean rhythm: k hits spread as evenly as the steps allow.
            let k = 2 + below(rng, (len / 3).max(2) - 1);
            let rotate = below(rng, len.max(1));
            for (s, v) in g.iter_mut().enumerate().take(len) {
                let i = (s + rotate) % len;
                if (i * k) % len < k {
                    *v = if rng.unit() < 0.25 { 1 } else { 2 };
                }
            }
            // A fill on the last beat, now and then.
            if rng.unit() < 0.3 && len >= beat {
                for (i, v) in g.iter_mut().take(len).skip(len - beat).enumerate() {
                    *v = 1 + (i * 3 / beat).min(2);
                }
            }
        }
    }
    g
}

/// A throw for the steps: each lane's playing steps take a new groove's
/// with a chance of `amount`, the groove made for what its channel sounds
/// like.
pub fn throw_steps(p: &ModuleParams, seed: u32, amount: f32) {
    let mut rng = Rng::new(seed ^ 0x51E9_5EED);
    let amount = amount.clamp(0.0, 1.0);
    let len = length(p);
    let rate = (p.data[data::RATE].get().round().max(0.0) as usize).min(DIVISIONS.len() - 1);
    let per_beat = DIVISIONS[rate].1.round() as usize;
    for c in 0..CHANNELS {
        let new = groove(&mut rng, role(&sound(p, c)), len, per_beat);
        for (s, &v) in new.iter().enumerate().take(len) {
            if rng.unit() < amount {
                set_step(p, c, s, v);
            }
        }
    }
}

/// What a throw changes, kept to take it back.
#[derive(Clone)]
pub struct Before {
    sounds: Vec<f32>,
    steps: Vec<f32>,
}

impl Before {
    pub fn take(p: &ModuleParams) -> Self {
        Before {
            sounds: (0..CHANNELS * knob::COUNT).map(|i| p.get(i)).collect(),
            steps: (0..CHANNELS * MAX_LEN)
                .map(|i| p.data[data::STEPS + i].get())
                .collect(),
        }
    }

    pub fn restore(&self, p: &ModuleParams) {
        for c in 0..CHANNELS {
            for k in 0..knob::COUNT {
                p.set(ch(c, k), self.sounds[ch(c, k)]);
            }
        }
        for (i, v) in self.steps.iter().enumerate() {
            p.data[data::STEPS + i].set(*v);
        }
    }
}
