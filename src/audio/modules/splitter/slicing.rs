//! Cutting the sample into slices in one go (UI thread): an equal GRID, at
//! the TRANSIENTS, or at RANDOM into short fragments; and what a slice
//! sounds like, for the dice.
//!
//! The transient slicer follows the sample's level in 5 ms hops and cuts
//! where it rises sharply above the hops just before (by 18 dB at the
//! lowest SENS, 3 dB at the highest), at least 40 ms apart, ignoring what is
//! 50 dB under the loudest hop; each cut moves back to the zero crossing
//! just before it, so slices start without a click.

use super::atoms::{self, ATOMS};
use super::{
    FIELDS, MAX_LEN, MAX_SLICES, count, data, flag, get, get_int, note, set, set_step, slice,
};
use crate::audio::modules::{ModuleParams, Rng};
use crate::audio::sample::Sample;

/// `n` equal regions of the whole source.
pub fn grid(n: usize) -> Vec<(f32, f32)> {
    let n = n.clamp(1, MAX_SLICES);
    (0..n)
        .map(|i| (i as f32 / n as f32, (i + 1) as f32 / n as f32))
        .collect()
}

/// Regions between the sample's onsets (see the module's documentation);
/// `sens` 0…1. At most `max`, the strongest kept.
pub fn transients(sample: &Sample, sens: f32, max: usize) -> Vec<(f32, f32)> {
    let n = sample.frames();
    let hop = ((sample.rate * 0.005) as usize).max(32);
    let hops = n / hop;
    if hops < 4 {
        return vec![(0.0, 1.0)];
    }
    let (l, r) = (&sample.left, sample.right());
    let mono = |i: usize| (l[i] + r[i]) * 0.5;
    let db: Vec<f32> = (0..hops)
        .map(|k| {
            let e: f32 = (k * hop..(k + 1) * hop)
                .map(|i| mono(i) * mono(i))
                .sum::<f32>()
                / hop as f32;
            10.0 * (e + 1e-10).log10()
        })
        .collect();
    let loudest = db.iter().copied().fold(f32::MIN, f32::max);
    let floor = loudest - 50.0;
    let rise: Vec<f32> = (0..hops)
        .map(|k| {
            let from = k.saturating_sub(3);
            if k == 0 {
                return db[0] - floor.min(db[0]);
            }
            let before = db[from..k].iter().sum::<f32>() / (k - from) as f32;
            (db[k] - before).max(0.0)
        })
        .collect();
    let needed = 18.0 - 15.0 * sens.clamp(0.0, 1.0);
    let gap = ((0.04 * sample.rate) as usize / hop).max(1);
    let mut onsets: Vec<(usize, f32)> = Vec::new();
    for k in 0..hops {
        let peak = rise[k] >= needed && db[k] > floor && (k + 1 >= hops || rise[k] >= rise[k + 1]);
        if !peak {
            continue;
        }
        match onsets.last_mut() {
            Some(last) if k - last.0 < gap => {
                if rise[k] > last.1 {
                    *last = (k, rise[k]);
                }
            }
            _ => onsets.push((k, rise[k])),
        }
    }
    // The strongest, back in order.
    if onsets.len() > max {
        onsets.sort_by(|a, b| b.1.total_cmp(&a.1));
        onsets.truncate(max);
        onsets.sort_by_key(|o| o.0);
    }
    let mut cuts: Vec<usize> = onsets
        .iter()
        .map(|&(k, _)| {
            let at = k * hop;
            // Back to the zero crossing before it, within a hop.
            (at.saturating_sub(hop)..=at)
                .rev()
                .find(|&i| i > 0 && (mono(i - 1) <= 0.0) != (mono(i) <= 0.0))
                .unwrap_or(at)
        })
        .collect();
    cuts.dedup();
    if cuts.is_empty() {
        return vec![(0.0, 1.0)];
    }
    // What comes before the first onset is a slice too, if it is heard.
    if cuts[0] > hop * 4
        && cuts.len() < max
        && db[..cuts[0] / hop].iter().any(|&d| d > floor + 10.0)
    {
        cuts.insert(0, 0);
    }
    let total = n as f32;
    cuts.iter()
        .enumerate()
        .map(|(i, &c)| {
            (
                c as f32 / total,
                cuts.get(i + 1).map_or(1.0, |&e| e as f32 / total),
            )
        })
        .collect()
}

/// `n` short fragments at random places: 8 ms to 400 ms (or half the
/// source), in order of where they start.
pub fn random(n: usize, seed: u32, seconds: f32) -> Vec<(f32, f32)> {
    let mut rng = Rng::new(seed | 1);
    let seconds = seconds.max(0.01);
    let longest = (0.4f32).min(seconds * 0.5).max(0.008);
    let mut out: Vec<(f32, f32)> = (0..n.clamp(1, MAX_SLICES))
        .map(|_| {
            let len = 0.008 * (longest / 0.008).powf(rng.unit());
            let share = (len / seconds).min(1.0);
            let start = rng.unit() * (1.0 - share);
            (start, start + share)
        })
        .collect();
    out.sort_by(|a, b| a.0.total_cmp(&b.0));
    out
}

/// Euclidean hits a fresh slice starts with, by its place (so a freshly cut
/// sample plays something in EUCLID mode).
const FRESH_HITS: [f32; 8] = [4.0, 3.0, 2.0, 5.0, 1.0, 3.0, 2.0, 1.0];

/// Makes the regions the slices of `source`, from the first on: their
/// settings start plain (a slice keeps its MIDI note, a new one gets C2 up
/// by slice), and the pattern plays them in order.
pub fn apply(p: &ModuleParams, regions: &[(f32, f32)], source: usize) {
    let n = regions.len().min(MAX_SLICES);
    for (s, &(a, b)) in regions.iter().take(n).enumerate() {
        let kept = if s < count(p) { note(p, s) } else { None };
        for (k, fl) in FIELDS.iter().enumerate() {
            set(p, s, k, fl.default);
        }
        set(p, s, slice::START, a.min(b));
        set(p, s, slice::END, a.max(b));
        set(p, s, slice::SOURCE, source as f32);
        set(
            p,
            s,
            slice::NOTE,
            kept.map_or((36 + s).min(127) as f32, f32::from),
        );
        set(p, s, slice::HITS, FRESH_HITS[s % FRESH_HITS.len()]);
        set(p, s, slice::ROTATE, (s % 4) as f32);
    }
    p.data[data::COUNT].set(n as f32);
    p.data[data::SELECTED].set(0.0);
    p.data[data::LEARN].set(0.0);
    for i in 0..MAX_LEN {
        set_step(p, i, (n > 0 && i < 16).then(|| i % n));
    }
}

/// What a slice sounds like, for the dice.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    /// Sub and bass: on the strong beats.
    Low,
    /// Clicks and short hits: busy grids.
    Hit,
    /// Sines, pings, pitched sounds: sparse.
    Tone,
    /// Noise and data: offbeats.
    Noise,
    /// Long sounds and grain clouds: rare.
    Texture,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Role::Low => "low",
            Role::Hit => "hit",
            Role::Tone => "tone",
            Role::Noise => "noise",
            Role::Texture => "texture",
        }
    }
}

/// Slice `s`'s role: an atom's from its kind, a sample slice's from its
/// length and how often it crosses zero.
pub fn role(p: &ModuleParams, s: usize, sample: Option<&Sample>) -> Role {
    let source = get_int(p, s, slice::SOURCE).max(0) as usize;
    let span = (get(p, s, slice::END) - get(p, s, slice::START)).abs();
    if source > 0 {
        let seconds = span * (atoms::ATOM_LEN as f32 / atoms::ATOM_RATE);
        if flag(p, s, slice::GRAIN) && seconds > 0.3 {
            return Role::Texture;
        }
        return match source - 1 {
            atoms::SUB => Role::Low,
            atoms::CLICK => Role::Hit,
            atoms::NOISE | atoms::DATA | atoms::CRACKLE => Role::Noise,
            k if k < ATOMS.len() => Role::Tone,
            _ => Role::Hit,
        };
    }
    let Some(sample) = sample else {
        return Role::Hit;
    };
    let seconds = span * sample.seconds();
    if seconds > 0.6 {
        return Role::Texture;
    }
    let n = sample.frames();
    let a = ((get(p, s, slice::START).min(get(p, s, slice::END))) * n as f32) as usize;
    let b = (a + (0.25 * sample.rate) as usize)
        .min(((get(p, s, slice::START).max(get(p, s, slice::END))) * n as f32) as usize)
        .min(n);
    if b <= a + 2 {
        return Role::Hit;
    }
    let crossings = sample.left[a..b]
        .windows(2)
        .filter(|w| (w[0] < 0.0) != (w[1] < 0.0))
        .count();
    let zcr = crossings as f32 / ((b - a) as f32 / sample.rate);
    if zcr < 500.0 {
        Role::Low
    } else if seconds < 0.06 {
        Role::Hit
    } else if zcr > 4000.0 {
        Role::Noise
    } else {
        Role::Tone
    }
}
