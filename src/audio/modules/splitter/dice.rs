//! DICE: throws for Splitter, after Fjord's ALEA. One throw makes new slice
//! settings, new rhythms or both; CHANCE sets how far it goes (a little
//! nudges what is there, all the way starts afresh), and BACK takes the
//! last throw back.
//!
//! Throws follow what each slice sounds like (its [`Role`]): pitches move
//! by pure intervals (octaves, fifths, fourths), short hits get ratchets
//! and crushing, long sounds become grain clouds; rhythms keep to a strict
//! grid, lows on the strong beats, hits on busy Euclidean patterns, tones
//! sparse and prime-length so they drift, noise on the offbeats.

use super::slicing::{Role, role};
use super::{MAX_LEN, REPEAT_RATES, count, data, get, length, set, set_step, slice};
use crate::audio::modules::{ModuleParams, Rng};
use crate::audio::sample::Sample;

fn pick<T: Copy>(rng: &mut Rng, list: &[T]) -> T {
    list[((rng.unit() * list.len() as f32) as usize).min(list.len() - 1)]
}

fn between(rng: &mut Rng, lo: f32, hi: f32) -> f32 {
    lo + (hi - lo) * rng.unit()
}

/// Logarithmically between `lo` and `hi` (both above 0).
fn log_between(rng: &mut Rng, lo: f32, hi: f32) -> f32 {
    lo * (hi / lo).powf(rng.unit())
}

/// A throw for the settings of `slices`: each amount moves `amount` (0…1)
/// of the way to a new one, each choice changes with that chance.
pub fn throw_slices(
    p: &ModuleParams,
    sample: Option<&Sample>,
    seed: u32,
    amount: f32,
    slices: &[usize],
) {
    use slice::*;
    let a = amount.clamp(0.0, 1.0);
    let n = count(p);
    for &s in slices.iter().filter(|&&s| s < n) {
        let mut rng = Rng::new(seed ^ (s as u32 + 1).wrapping_mul(0x9E37_79B9) | 1);
        let r = role(p, s, sample);
        let from_sample = get(p, s, SOURCE) < 0.5;
        let choose = |rng: &mut Rng, k: usize, v: f32| {
            if rng.unit() < a {
                set(p, s, k, v);
            }
        };
        let nudge = |k: usize, v: f32| set(p, s, k, get(p, s, k) + (v - get(p, s, k)) * a);

        let intervals: &[f32] = match (r, from_sample) {
            (_, true) => &[0.0, 0.0, 12.0, -12.0, 7.0, -5.0, -24.0],
            (Role::Tone, _) => &[0.0, 12.0, 24.0, -12.0, 7.0, 19.0, 5.0, 36.0],
            (Role::Low, _) => &[0.0, -12.0, 5.0, -5.0, 7.0],
            (Role::Hit, _) => &[0.0, 12.0, -12.0, 24.0],
            (Role::Noise, _) => &[0.0, 12.0, -12.0, -24.0, 24.0],
            (Role::Texture, _) => &[0.0, -12.0, 12.0, 7.0, -5.0],
        };
        let v = pick(&mut rng, intervals);
        choose(&mut rng, PITCH, v);
        let v = if rng.unit() < 0.25 { 1.0 } else { 0.0 };
        choose(&mut rng, REVERSE, v);
        let short = matches!(r, Role::Hit | Role::Tone | Role::Noise);
        let play = if short && rng.unit() < 0.12 {
            2.0
        } else if rng.unit() < 0.1 {
            1.0
        } else {
            0.0
        };
        choose(&mut rng, PLAY, play);
        let attack = if r == Role::Tone || r == Role::Texture {
            log_between(&mut rng, 0.5, 30.0)
        } else {
            between(&mut rng, 0.0, 2.0)
        };
        nudge(ATTACK, attack);
        let (lo, hi) = match r {
            Role::Low => (80.0, 320.0),
            Role::Hit => (2.0, 25.0),
            Role::Tone => (10.0, 400.0),
            Role::Noise => (5.0, 90.0),
            Role::Texture => (200.0, 1500.0),
        };
        let v = log_between(&mut rng, lo, hi);
        nudge(RELEASE, v);
        let v = get(p, s, GAIN) + between(&mut rng, -4.0, 4.0);
        nudge(GAIN, v.clamp(-30.0, 3.0));
        let v = between(&mut rng, -0.7, 0.7);
        nudge(PAN, v);

        // The ratchet.
        let repeats: &[f32] = if matches!(r, Role::Hit | Role::Noise) {
            &[1.0, 1.0, 1.0, 1.0, 2.0, 3.0, 4.0, 8.0, 16.0]
        } else {
            &[1.0, 1.0, 1.0, 2.0, 3.0]
        };
        let v = pick(&mut rng, repeats);
        choose(&mut rng, REPEAT, v);
        let v = (1 + (rng.unit() * (REPEAT_RATES.len() - 1) as f32) as usize)
            .min(REPEAT_RATES.len() - 1) as f32;
        choose(&mut rng, REP_RATE, v);
        let v = pick(
            &mut rng,
            &[0.0, 0.0, 0.0, 0.0, 12.0, -12.0, 7.0, -5.0, 1.0, -1.0],
        );
        choose(&mut rng, RAMP, v);
        let v = between(&mut rng, 0.0, 60.0);
        nudge(FALL, v);

        // The crusher.
        let v = if rng.unit() < 0.75 {
            16.0
        } else {
            pick(&mut rng, &[12.0, 8.0, 6.0, 4.0, 3.0])
        };
        choose(&mut rng, BITS, v);
        let v = if rng.unit() < 0.8 {
            1.0
        } else {
            pick(&mut rng, &[2.0, 3.0, 4.0, 8.0, 16.0])
        };
        choose(&mut rng, DOWN, v);

        // Grains.
        let chance = match r {
            Role::Texture => 0.7,
            Role::Tone => 0.15,
            _ => 0.08,
        };
        let v = if rng.unit() < chance { 1.0 } else { 0.0 };
        choose(&mut rng, GRAIN, v);
        let v = log_between(&mut rng, 8.0, 250.0);
        nudge(SIZE, v);
        let v = log_between(&mut rng, 8.0, 150.0);
        nudge(DENSITY, v);
        let v = rng.unit();
        nudge(SPRAY, v);
        let v = rng.unit();
        nudge(POS, v);
        let v = if rng.unit() < 0.3 { 1.0 } else { 0.0 };
        choose(&mut rng, FREEZE, v);
        let v = if rng.unit() < 0.4 { 1.0 } else { 0.0 };
        choose(&mut rng, SYNC, v);
        let v = pick(&mut rng, &[0.0, 0.0, 0.0, 0.15, 12.0, 7.0, 5.0]);
        choose(&mut rng, JITTER, v);

        // A sample slice's region shifts and stretches now and then: a cut
        // in a new place.
        if from_sample && sample.is_some() && rng.unit() < a * 0.5 {
            let (s0, s1) = (
                get(p, s, START).min(get(p, s, END)),
                get(p, s, START).max(get(p, s, END)),
            );
            let len =
                ((s1 - s0) * pick(&mut rng, &[0.25, 0.5, 0.5, 1.0, 1.0, 2.0])).clamp(0.0005, 1.0);
            let start = (s0 + rng.bipolar() * len * 0.5).clamp(0.0, 1.0 - len);
            set(p, s, START, start);
            set(p, s, END, start + len);
        }
    }
}

/// A throw for the rhythms: each slice's Euclidean rhythm and probability,
/// and the pattern's steps, made for what each slice sounds like.
pub fn throw_rhythm(p: &ModuleParams, sample: Option<&Sample>, seed: u32, amount: f32) {
    use slice::*;
    let a = amount.clamp(0.0, 1.0);
    let n = count(p);
    let mut rng = Rng::new(seed ^ 0x51E9_5EED);
    let roles: Vec<Role> = (0..n).map(|s| role(p, s, sample)).collect();
    for (s, &r) in roles.iter().enumerate() {
        let (k, steps, rot) = match r {
            Role::Low => (pick(&mut rng, &[2.0, 3.0, 4.0]), 16.0, 0.0),
            Role::Hit => (
                pick(&mut rng, &[5.0, 6.0, 7.0, 9.0, 11.0]),
                16.0,
                (rng.unit() * 16.0).floor(),
            ),
            Role::Tone => (
                pick(&mut rng, &[1.0, 2.0, 3.0]),
                pick(&mut rng, &[7.0, 9.0, 11.0, 13.0, 16.0]),
                (rng.unit() * 7.0).floor(),
            ),
            Role::Noise => (
                pick(&mut rng, &[2.0, 3.0, 4.0, 5.0, 6.0]),
                pick(&mut rng, &[12.0, 16.0]),
                pick(&mut rng, &[1.0, 3.0, 5.0, 7.0]),
            ),
            Role::Texture => (1.0, pick(&mut rng, &[16.0, 32.0]), 0.0),
        };
        for (field, v) in [(HITS, k), (STEPS, steps), (ROTATE, rot)] {
            if rng.unit() < a {
                set(p, s, field, v);
            }
        }
        let (lo, hi) = match r {
            Role::Low => (40.0, 80.0),
            Role::Hit => (30.0, 80.0),
            Role::Tone => (8.0, 30.0),
            Role::Noise => (15.0, 45.0),
            Role::Texture => (4.0, 12.0),
        };
        let v = between(&mut rng, lo, hi);
        set(p, s, PROB, get(p, s, PROB) + (v - get(p, s, PROB)) * a);
    }
    // The pattern: lows on the beats, hits anywhere, the rest sparse.
    let len = length(p);
    for i in 0..len.min(MAX_LEN) {
        if rng.unit() >= a || n == 0 {
            continue;
        }
        let mut candidates: Vec<(usize, f32)> = Vec::new();
        for (s, &r) in roles.iter().enumerate() {
            let w = match r {
                Role::Low if i % 8 == 0 => 3.0,
                Role::Low if i % 4 == 0 => 1.0,
                Role::Low => 0.0,
                Role::Hit => 0.9,
                Role::Tone => 0.25,
                Role::Noise if i % 2 == 1 => 0.6,
                Role::Noise => 0.15,
                Role::Texture if i == 0 => 0.6,
                Role::Texture => 0.05,
            };
            if w > 0.0 {
                candidates.push((s, w));
            }
        }
        let total: f32 = candidates.iter().map(|c| c.1).sum();
        // Rests take as much room as everything else together.
        let mut x = rng.unit() * total * 2.0;
        let mut chosen = None;
        for (s, w) in candidates {
            if x < w {
                chosen = Some(s);
                break;
            }
            x -= w;
        }
        set_step(p, i, chosen);
    }
    p.data[data::SEED].set((rng.next_u32() >> 8) as f32);
}

/// What a throw changes, kept to take it back: the slices and the pattern.
#[derive(Clone)]
pub struct Before {
    values: Vec<f32>,
    count: f32,
}

impl Before {
    pub fn take(p: &ModuleParams) -> Self {
        Before {
            values: (data::STEPS..data::LEN).map(|i| p.data[i].get()).collect(),
            count: p.data[data::COUNT].get(),
        }
    }

    pub fn restore(&self, p: &ModuleParams) {
        for (i, v) in self.values.iter().enumerate() {
            p.data[data::STEPS + i].set(*v);
        }
        p.data[data::COUNT].set(self.count);
        let sel = p.data[data::SELECTED].get();
        p.data[data::SELECTED].set(sel.min((self.count - 1.0).max(0.0)));
    }
}
