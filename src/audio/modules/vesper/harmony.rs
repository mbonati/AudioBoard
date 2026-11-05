//! The generator's harmony: sixteen moods (each a seven-degree scale), which
//! degrees the engine may stand on (VARIATION), how many notes a chord has
//! (COMPLEXITY), how one chord leads to the next, and how a chord is laid out
//! on the keyboard so that its voices move as little as they can.
//!
//! Everything here is pure and allocation-free (only the labels allocate, and
//! only the panel asks for those), so the audio thread can work a progression
//! out again whenever a rule changes.

use crate::audio::modules::Rng;
use crate::audio::params::NOTE_NAMES;

/// A mood: its name, the scale under it, the scale's seven degrees.
pub struct Mood {
    pub name: &'static str,
    pub scale: &'static str,
    pub semis: [u8; 7],
}

pub const MOODS: [Mood; 16] = [
    Mood { name: "Radiant", scale: "Major (Ionian)", semis: [0, 2, 4, 5, 7, 9, 11] },
    Mood { name: "Dreaming", scale: "Lydian", semis: [0, 2, 4, 6, 7, 9, 11] },
    Mood { name: "Roaming", scale: "Mixolydian", semis: [0, 2, 4, 5, 7, 9, 10] },
    Mood { name: "Dawn", scale: "Harmonic major", semis: [0, 2, 4, 5, 7, 8, 11] },
    Mood { name: "Elegy", scale: "Natural minor (Aeolian)", semis: [0, 2, 3, 5, 7, 8, 10] },
    Mood { name: "Reverie", scale: "Dorian", semis: [0, 2, 3, 5, 7, 9, 10] },
    Mood { name: "Tragic", scale: "Harmonic minor", semis: [0, 2, 3, 5, 7, 8, 11] },
    Mood { name: "Mystery", scale: "Melodic minor", semis: [0, 2, 3, 5, 7, 9, 11] },
    Mood { name: "Descent", scale: "Phrygian", semis: [0, 1, 3, 5, 7, 8, 10] },
    Mood { name: "Void", scale: "Locrian", semis: [0, 1, 3, 5, 6, 8, 10] },
    Mood { name: "Noir", scale: "Hungarian minor", semis: [0, 2, 3, 6, 7, 8, 11] },
    Mood { name: "Ritual", scale: "Neapolitan minor", semis: [0, 1, 3, 5, 7, 8, 11] },
    Mood { name: "Sands", scale: "Phrygian dominant", semis: [0, 1, 4, 5, 7, 8, 10] },
    Mood { name: "Temple", scale: "Double harmonic", semis: [0, 1, 4, 5, 7, 8, 11] },
    Mood { name: "Cosmos", scale: "Lydian augmented", semis: [0, 2, 4, 6, 8, 9, 11] },
    Mood { name: "Orbit", scale: "Lydian dominant", semis: [0, 2, 4, 6, 7, 9, 10] },
];

/// Chords a loop holds at most; also how many chords EVOLVE shows.
pub const MAX_CHORDS: usize = 8;
/// Notes a chord holds at most.
pub const MAX_TONES: usize = 4;

/// What colours a chord beyond its thirds. A colour the scale can't give
/// plays as a plain chord.
pub mod flavor {
    pub const PLAIN: u8 = 0;
    pub const SUS2: u8 = 1;
    pub const SUS4: u8 = 2;
    pub const ADD9: u8 = 3;
}

/// One chord of a progression: where it stands in the scale, and its colour.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Step {
    pub degree: u8,
    pub flavor: u8,
}

impl Step {
    /// As one number, for the panel.
    pub fn pack(self) -> f32 {
        (self.degree as u32 + 8 * self.flavor as u32) as f32
    }

    pub fn unpack(v: f32) -> Self {
        let v = v.max(0.0) as u32;
        Self { degree: (v % 8).min(6) as u8, flavor: (v / 8).min(3) as u8 }
    }
}

/// The harmonic rules the player sets.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Rules {
    /// The root's pitch class (0 = C).
    pub root: i32,
    pub mood: usize,
    /// How many of the seven degrees chords may stand on (1…7).
    pub variation: usize,
    /// Notes a chord (2…4).
    pub complexity: usize,
}

impl Rules {
    /// Semitones above the root of scale step `d` (any integer: 7 is the
    /// octave).
    pub fn semis(&self, d: i32) -> i32 {
        12 * d.div_euclid(7) + MOODS[self.mood].semis[d.rem_euclid(7) as usize] as i32
    }

    /// Is this pitch class in the scale?
    pub fn in_scale(&self, note: i32) -> bool {
        let pc = (note - self.root).rem_euclid(12);
        MOODS[self.mood].semis.iter().any(|&s| s as i32 == pc)
    }

    /// The chord on `step`, in semitones above the root (stacked, not yet
    /// laid out), and how many notes it has.
    pub fn tones(&self, step: Step) -> ([i32; MAX_TONES], usize) {
        let d = step.degree as i32;
        let above = |k: i32| self.semis(d + k) - self.semis(d);
        let perfect = above(4) == 7;
        let shape: &[i32] = match (self.complexity.clamp(2, MAX_TONES), step.flavor) {
            // Two notes: open if the fifth is pure, else root and third.
            (2, _) => {
                if perfect {
                    &[0, 4]
                } else {
                    &[0, 2]
                }
            }
            (3, flavor::SUS2) if perfect && above(1) == 2 => &[0, 1, 4],
            (3, flavor::SUS4) if perfect && above(3) == 5 => &[0, 3, 4],
            (3, _) => &[0, 2, 4],
            (_, flavor::SUS2) if perfect && above(1) == 2 => &[0, 1, 4, 6],
            (_, flavor::SUS4) if perfect && above(3) == 5 => &[0, 3, 4, 6],
            (_, flavor::ADD9) if above(1) == 2 => &[0, 2, 4, 8],
            _ => &[0, 2, 4, 6],
        };
        let mut out = [0; MAX_TONES];
        for (o, &k) in out.iter_mut().zip(shape) {
            *o = self.semis(d + k);
        }
        (out, shape.len())
    }

    /// The pitch class of the chord's root.
    pub fn chord_root(&self, step: Step) -> i32 {
        (self.root + self.semis(step.degree as i32)).rem_euclid(12)
    }

    /// How welcome a degree is: strong degrees more, those whose fifth isn't
    /// pure (a diminished or augmented chord) much less.
    fn weight(&self, d: usize) -> f32 {
        const STRENGTH: [f32; 7] = [1.0, 0.45, 0.5, 0.85, 0.7, 0.9, 0.3];
        let d = d % 7;
        let fifth = self.semis(d as i32 + 4) - self.semis(d as i32);
        STRENGTH[d] * if fifth == 7 { 1.0 } else { 0.2 }
    }
}

/// A seed that spreads: xorshift's first draws follow its seed closely.
fn spread(seed: u32, salt: u32) -> u32 {
    let x = seed.wrapping_add(salt.wrapping_mul(0x9E37_79B9));
    (x ^ (x >> 15)).wrapping_mul(0x85EB_CA6B) | 1
}

/// The degrees chords may stand on: the tonic first, then in an order the
/// seed picks, the stronger ones likelier. A larger VARIATION keeps the
/// degrees a smaller one had and adds to them.
pub fn allowed(seed: u32, rules: &Rules) -> ([u8; 7], usize) {
    let mut rng = Rng::new(spread(seed, 7));
    let mut weights: [f32; 7] = std::array::from_fn(|d| rules.weight(d));
    weights[0] = 0.0;
    let mut out = [0u8; 7];
    let len = rules.variation.clamp(1, 7);
    for slot in out.iter_mut().take(len).skip(1) {
        let d = pick(&mut rng, &weights);
        *slot = d as u8;
        weights[d] = 0.0;
    }
    (out, len)
}

/// An index, likelier the larger its weight (all zero: the first).
fn pick(rng: &mut Rng, weights: &[f32]) -> usize {
    let total: f32 = weights.iter().sum();
    let mut x = rng.unit() * total;
    let mut last = 0;
    for (k, &w) in weights.iter().enumerate() {
        if w > 0.0 {
            last = k;
            x -= w;
            if x < 0.0 {
                return k;
            }
        }
    }
    last
}

/// The chord after `prev`. Roots move by fourths and fifths first, then
/// thirds (which keep two notes), then steps; never onto themselves unless
/// there's nowhere else; `before` (the chord before that) is less welcome, so
/// that two chords don't just swap, and `avoid` not at all.
pub fn next_step(rng: &mut Rng, rules: &Rules, allowed: &[u8], prev: Step, before: Option<u8>, avoid: Option<u8>) -> Step {
    let mut weights = [0.0f32; 7];
    for (w, &c) in weights.iter_mut().zip(allowed) {
        if c == prev.degree || Some(c) == avoid {
            continue;
        }
        let motion = match (c as i32 - prev.degree as i32).rem_euclid(7) {
            3 | 4 => 1.4,
            2 | 5 => 1.25,
            _ => 0.9,
        };
        let again = if Some(c) == before { 0.35 } else { 1.0 };
        *w = rules.weight(c as usize) * motion * again;
    }
    let degree = if weights.iter().any(|&w| w > 0.0) { allowed[pick(rng, &weights)] } else { prev.degree };
    let x = rng.unit();
    let flavor = if x < 0.62 {
        flavor::PLAIN
    } else if x < 0.78 {
        flavor::SUS2
    } else if x < 0.94 {
        flavor::SUS4
    } else {
        flavor::ADD9
    };
    Step { degree, flavor }
}

/// LOOP's progression, from the seed: `count` chords, usually from the tonic,
/// the last leading back round without landing on the first.
pub fn progression(seed: u32, count: usize, rules: &Rules) -> [Step; MAX_CHORDS] {
    let (degrees, len) = allowed(seed, rules);
    let degrees = &degrees[..len];
    let mut rng = Rng::new(spread(seed, 1));
    let start = rng.unit();
    let first = if start < 0.75 || len == 1 { 0 } else { degrees[1 + (rng.unit() * (len - 1) as f32) as usize % (len - 1)] };
    let mut out = [Step { degree: first, flavor: flavor::PLAIN }; MAX_CHORDS];
    let count = count.clamp(1, MAX_CHORDS);
    for k in 1..count {
        let before = (k >= 2).then(|| out[k - 2].degree);
        let avoid = (k == count - 1).then_some(first);
        out[k] = next_step(&mut rng, rules, degrees, out[k - 1], before, avoid);
    }
    out
}

/// A random stream for EVOLVE, from the seed.
pub fn stream(seed: u32) -> Rng {
    Rng::new(spread(seed, 2))
}

/// Lays `step` out around `center`, as near as it can be to `prev` (the
/// notes before it; empty: nearest the centre). Tries each inversion, closed
/// and spread, in three octaves. Returns how many notes `out` got, low to
/// high.
pub fn voice(rules: &Rules, step: Step, prev: &[i32], center: i32, out: &mut [i32; MAX_TONES]) -> usize {
    let (tones, len) = rules.tones(step);
    let pcs: [i32; MAX_TONES] = tones.map(|t| (rules.root + t).rem_euclid(12));
    let mut best = f32::INFINITY;
    let mut consider = |notes: &[i32; MAX_TONES]| {
        let cost = voicing_cost(&notes[..len], prev, center);
        if cost < best {
            best = cost;
            *out = *notes;
        }
    };
    for inversion in 0..len {
        for octave in [-12, 0, 12] {
            let mut notes = [0; MAX_TONES];
            // The lowest note, the others stacked on it.
            let low = center - 7 + octave;
            notes[0] = low + (pcs[inversion] - low).rem_euclid(12);
            for k in 1..len {
                let pc = pcs[(inversion + k) % len];
                notes[k] = notes[k - 1] + 1 + (pc - notes[k - 1] - 1).rem_euclid(12);
            }
            consider(&notes);
            if len >= 3 {
                // Spread: the second from the top an octave down.
                let mut open = notes;
                open[len - 2] -= 12;
                open[..len].sort_unstable();
                consider(&open);
            }
        }
    }
    len
}

fn voicing_cost(notes: &[i32], prev: &[i32], center: i32) -> f32 {
    let mean = notes.iter().sum::<i32>() as f32 / notes.len() as f32;
    let mut cost = (mean - center as f32).abs() * if prev.is_empty() { 2.0 } else { 0.5 };
    for &n in notes {
        if let Some(moved) = prev.iter().map(|&p| (n - p).abs()).min() {
            cost += moved as f32;
        }
    }
    // Close intervals turn to mud down low.
    for pair in notes.windows(2) {
        if pair[0] < 55 && pair[1] - pair[0] < 5 {
            cost += 8.0;
        }
    }
    let (low, high) = (notes[0], notes[notes.len() - 1]);
    if low < center - 17 || high > center + 19 {
        cost += 24.0;
    }
    cost
}

/// Moves a note into the octave starting at `low`.
pub fn fold(note: i32, low: i32) -> i32 {
    low + (note - low).rem_euclid(12)
}

/// How this key spells a pitch class.
pub fn note_name(rules: &Rules, pc: i32) -> &'static str {
    const FLATS: [&str; 12] = ["C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B"];
    // A minor scale reads from its relative major.
    let parent = (rules.root + if rules.semis(2) == 3 { 3 } else { 0 }).rem_euclid(12);
    let names = if matches!(parent, 1 | 3 | 5 | 8 | 10) { &FLATS } else { &NOTE_NAMES };
    names[pc.rem_euclid(12) as usize]
}

/// "Am7", "Csus2", "F#°"…
pub fn label(rules: &Rules, step: Step) -> String {
    let (tones, len) = rules.tones(step);
    let name = note_name(rules, rules.chord_root(step));
    let iv = |k: usize| (tones[k] - tones[0]).rem_euclid(12);
    if len == 2 {
        return match iv(1) {
            7 => format!("{name}5"),
            3 => format!("{name}m"),
            _ => name.to_owned(),
        };
    }
    let triad = match (iv(1), iv(2)) {
        (3, 7) => "m",
        (4, 7) => "",
        (3, 6) => "°",
        (4, 8) => "+",
        (2, 7) => "sus2",
        (5, 7) => "sus4",
        (4, 6) => "b5",
        (3, 8) => "m#5",
        _ => "",
    };
    if len == 3 {
        return format!("{name}{triad}");
    }
    match (triad, iv(3)) {
        ("", 2) => format!("{name}add9"),
        ("m", 2) => format!("{name}madd9"),
        (_, 2) => format!("{name}{triad}add9"),
        ("", 11) => format!("{name}maj7"),
        ("", _) => format!("{name}7"),
        ("m", 11) => format!("{name}mMaj7"),
        ("m", _) => format!("{name}m7"),
        ("°", 9) => format!("{name}°7"),
        ("°", _) => format!("{name}ø7"),
        ("+", 11) => format!("{name}+maj7"),
        ("+", _) => format!("{name}+7"),
        ("sus2" | "sus4", 11) => format!("{name}maj7{triad}"),
        ("sus2" | "sus4", _) => format!("{name}7{triad}"),
        (_, _) => format!("{name}{triad}7"),
    }
}

/// "vi", "IV", "vii°"…: upper case when the third is major or absent.
pub fn numeral(rules: &Rules, step: Step) -> String {
    const NUMERALS: [&str; 7] = ["I", "II", "III", "IV", "V", "VI", "VII"];
    let n = NUMERALS[step.degree as usize % 7];
    let d = step.degree as i32;
    let third = rules.semis(d + 2) - rules.semis(d);
    let fifth = rules.semis(d + 4) - rules.semis(d);
    match (third, fifth) {
        (3, 6) => format!("{}°", n.to_lowercase()),
        (3, _) => n.to_lowercase(),
        (4, 8) => format!("{n}+"),
        _ => n.to_owned(),
    }
}
