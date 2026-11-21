//! Harmony for Automa: the scales (the church modes, harmonic and melodic
//! minor), chords stacked in thirds on each degree, the progressions, and
//! the rules that keep every generated note consonant with the chord under
//! it: chord tones anywhere, tensions (a whole step above a chord tone) on
//! weaker steps as TENSION allows, avoid notes (a half step above one) only
//! as quick passing tones.

use super::hash01;
use crate::audio::params::NOTE_NAMES;

/// (short name, long name, semitones of each degree).
pub const SCALES: [(&str, &str, [u8; 7]); 8] = [
    ("MAJOR", "Major (Ionian)", [0, 2, 4, 5, 7, 9, 11]),
    ("MINOR", "Natural minor (Aeolian)", [0, 2, 3, 5, 7, 8, 10]),
    ("DORIAN", "Dorian", [0, 2, 3, 5, 7, 9, 10]),
    ("PHRYG", "Phrygian", [0, 1, 3, 5, 7, 8, 10]),
    ("LYDIAN", "Lydian", [0, 2, 4, 6, 7, 9, 11]),
    ("MIXO", "Mixolydian", [0, 2, 4, 5, 7, 9, 10]),
    ("HARM", "Harmonic minor", [0, 2, 3, 5, 7, 8, 11]),
    ("MELOD", "Melodic minor", [0, 2, 3, 5, 7, 9, 11]),
];

/// Where each scale's tonic sits in its parent major scale, in semitones
/// (for spelling with sharps or flats).
const PARENT: [u8; 8] = [0, 3, 10, 8, 7, 5, 3, 3];

/// Chord progressions, as scale degrees (0 = the tonic's chord).
pub const PRESETS: [(&str, &[u8]); 12] = [
    ("Pop", &[0, 4, 5, 3]),
    ("Axis", &[5, 3, 0, 4]),
    ("Doo-wop", &[0, 5, 3, 4]),
    ("Jazz turnaround", &[1, 4, 0, 5]),
    ("Circle", &[5, 1, 4, 0]),
    ("Canon", &[0, 4, 5, 2, 3, 0, 3, 4]),
    ("Royal road", &[3, 4, 2, 5]),
    ("Epic (minor)", &[0, 5, 2, 6]),
    ("Andalusian (minor)", &[0, 6, 5, 4]),
    ("Modal vamp", &[0, 3]),
    ("Floating", &[0, 1]),
    ("Drone", &[0]),
];

/// How long each chord lasts: (label, beats).
pub const CHANGES: [(&str, f64); 4] = [
    ("1/2 BAR", 2.0),
    ("1 BAR", 4.0),
    ("2 BARS", 8.0),
    ("4 BARS", 16.0),
];

/// Chords a progression holds at most.
pub const SLOTS: usize = 8;

const NUMERALS: [&str; 7] = ["I", "II", "III", "IV", "V", "VI", "VII"];
const FLAT_NAMES: [&str; 12] = [
    "C", "Db", "D", "Eb", "E", "F", "Gb", "G", "Ab", "A", "Bb", "B",
];

/// What a scale tone is over a chord.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Role {
    Chord,
    /// A whole step above a chord tone (9, 11, 13) or a seventh: colour.
    Tension,
    /// A half step above a chord tone: it clashes unless it moves on.
    Avoid,
}

/// The key, its scale and whether chords take their seventh.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Harmony {
    pub key: u8,
    pub scale: usize,
    pub sevenths: bool,
}

impl Harmony {
    pub fn intervals(&self) -> &'static [u8; 7] {
        &SCALES[self.scale.min(SCALES.len() - 1)].2
    }

    /// Semitones from the key's tonic up to degree `d` (any integer: 7 is
    /// the octave).
    pub fn semis(&self, d: i32) -> i32 {
        12 * d.div_euclid(7) + self.intervals()[d.rem_euclid(7) as usize] as i32
    }

    /// Pitch class of degree `d`.
    pub fn pc(&self, d: i32) -> u8 {
        ((self.key as i32 + self.semis(d)).rem_euclid(12)) as u8
    }

    /// The chord on degree `root`: its tones, third over third.
    pub fn chord(&self, root: u8) -> Chord {
        let root = root % 7;
        let len = if self.sevenths { 4 } else { 3 };
        let mut pcs = [0; 4];
        for (k, pc) in pcs.iter_mut().enumerate().take(len) {
            *pc = self.pc(root as i32 + 2 * k as i32);
        }
        Chord { root, pcs, len }
    }

    /// Names a pitch class the way this key spells it.
    pub fn name(&self, pc: u8) -> &'static str {
        let parent = (self.key + PARENT[self.scale.min(PARENT.len() - 1)]) % 12;
        if matches!(parent, 1 | 3 | 5 | 8 | 10) {
            FLAT_NAMES[pc as usize % 12]
        } else {
            NOTE_NAMES[pc as usize % 12]
        }
    }

    /// "C major", "F# dorian", "A harmonic minor"…
    pub fn label(&self) -> String {
        let scale = SCALES[self.scale.min(SCALES.len() - 1)].1;
        format!(
            "{} {}",
            self.name(self.key),
            scale.split(" (").next().unwrap_or(scale).to_lowercase()
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Chord {
    /// The degree it stands on.
    pub root: u8,
    pub pcs: [u8; 4],
    pub len: usize,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Quality {
    Major,
    Minor,
    Diminished,
    Augmented,
}

impl Chord {
    pub fn tones(&self) -> &[u8] {
        &self.pcs[..self.len]
    }

    pub fn contains(&self, pc: u8) -> bool {
        self.tones().contains(&(pc % 12))
    }

    pub fn role(&self, pc: u8) -> Role {
        let pc = pc % 12;
        if self.contains(pc) {
            Role::Chord
        } else if self.tones().iter().any(|&t| (pc + 12 - t) % 12 == 1) {
            Role::Avoid
        } else {
            Role::Tension
        }
    }

    fn interval(&self, k: usize) -> u8 {
        (self.pcs[k] + 12 - self.pcs[0]) % 12
    }

    fn quality(&self) -> Quality {
        match (self.interval(1), self.interval(2)) {
            (3, 6) => Quality::Diminished,
            (4, 8) => Quality::Augmented,
            (3, _) => Quality::Minor,
            _ => Quality::Major,
        }
    }

    /// "Am7", "G", "Bø7"…
    pub fn name(&self, h: &Harmony) -> String {
        let quality = self.quality();
        let suffix = if self.len < 4 {
            match quality {
                Quality::Major => "",
                Quality::Minor => "m",
                Quality::Diminished => "°",
                Quality::Augmented => "+",
            }
        } else {
            match (quality, self.interval(3)) {
                (Quality::Major, 11) => "maj7",
                (Quality::Major, _) => "7",
                (Quality::Minor, 11) => "mMaj7",
                (Quality::Minor, _) => "m7",
                (Quality::Diminished, 9) => "°7",
                (Quality::Diminished, _) => "ø7",
                (Quality::Augmented, 11) => "+maj7",
                (Quality::Augmented, _) => "+7",
            }
        };
        format!("{}{suffix}", h.name(self.pcs[0]))
    }

    /// "vi", "V", "vii°"…: upper case for major chords.
    pub fn numeral(&self) -> String {
        let n = NUMERALS[self.root as usize % 7];
        match self.quality() {
            Quality::Major => n.to_owned(),
            Quality::Minor => n.to_lowercase(),
            Quality::Diminished => format!("{}°", n.to_lowercase()),
            Quality::Augmented => format!("{n}+"),
        }
    }
}

/// How strong a step is: the first of a chord, on a beat, on an eighth,
/// anything else.
pub fn strength(beat: f64, chord_beats: f64) -> usize {
    let near = |x: f64| (x - x.round()).abs() < 1e-6;
    if near(beat / chord_beats) {
        0
    } else if near(beat) {
        1
    } else if near(beat * 2.0) {
        2
    } else {
        3
    }
}

/// May a tone of this role sound on a step of this strength?
pub fn allowed(role: Role, strength: usize, tension: f32) -> bool {
    match role {
        Role::Chord => true,
        Role::Tension => tension >= [f32::INFINITY, 0.7, 0.4, 0.15][strength.min(3)],
        Role::Avoid => strength >= 3 && tension >= 0.85,
    }
}

/// The nearest degree to `d` within `lo..=hi` that may sound on this step;
/// going down first, the way a dissonance resolves.
pub fn snap(
    h: &Harmony,
    chord: &Chord,
    d: i32,
    (lo, hi): (i32, i32),
    strength: usize,
    tension: f32,
) -> i32 {
    let ok = |d: i32| (lo..=hi).contains(&d) && allowed(chord.role(h.pc(d)), strength, tension);
    let d = d.clamp(lo, hi);
    if ok(d) {
        return d;
    }
    for k in 1..=7 {
        for c in [d - k, d + k] {
            if ok(c) {
                return c;
            }
        }
    }
    d
}

/// Chord functions: tonic (I iii vi), subdominant (ii IV), dominant (V vii).
pub fn family(degree: u8) -> &'static [u8] {
    match degree % 7 {
        0 | 2 | 5 => &[0, 2, 5],
        1 | 3 => &[1, 3],
        _ => &[4, 6],
    }
}

/// The chords and their timing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Progression {
    pub degrees: [u8; SLOTS],
    pub len: usize,
    /// Beats per chord.
    pub beats: f64,
    /// After the first time round, chords may give way to another of the
    /// same function.
    pub wander: bool,
    pub seed: u32,
}

impl Progression {
    /// The slot playing at `beat`, and how many times round it is.
    pub fn slot_at(&self, beat: f64) -> (usize, i64) {
        let k = (beat / self.beats + 1e-9).floor() as i64;
        let len = self.len.clamp(1, SLOTS) as i64;
        (k.rem_euclid(len) as usize, k.div_euclid(len))
    }

    /// The degree of `slot` on its `cycle`th time round.
    pub fn degree(&self, slot: usize, cycle: i64) -> u8 {
        let written = self.degrees[slot % SLOTS] % 7;
        // The first time round, and the opening tonic, as written.
        if !self.wander || cycle <= 0 || (slot == 0 && written == 0) {
            return written;
        }
        let at = cycle as u64 * SLOTS as u64 + slot as u64;
        if hash01(self.seed, at, 0x000C_407D) >= 0.35 {
            return written;
        }
        // Another chord of the family (no allocation: the audio thread).
        let family = family(written);
        let others = family.len() - 1;
        let k = (hash01(self.seed, at, 0x5B5) * others as f32) as usize % others;
        family
            .iter()
            .copied()
            .filter(|&d| d != written)
            .nth(k)
            .unwrap_or(written)
    }

    /// The degree playing at `beat`.
    pub fn degree_at(&self, beat: f64) -> u8 {
        let (slot, cycle) = self.slot_at(beat);
        self.degree(slot, cycle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const C_MAJOR: Harmony = Harmony {
        key: 0,
        scale: 0,
        sevenths: false,
    };

    #[test]
    fn chords_are_named_and_numbered() {
        let names: Vec<String> = (0..7).map(|d| C_MAJOR.chord(d).name(&C_MAJOR)).collect();
        assert_eq!(names, ["C", "Dm", "Em", "F", "G", "Am", "B°"]);
        let numerals: Vec<String> = (0..7).map(|d| C_MAJOR.chord(d).numeral()).collect();
        assert_eq!(numerals, ["I", "ii", "iii", "IV", "V", "vi", "vii°"]);
        let sevenths = Harmony {
            sevenths: true,
            ..C_MAJOR
        };
        let names: Vec<String> = (0..7).map(|d| sevenths.chord(d).name(&sevenths)).collect();
        assert_eq!(names, ["Cmaj7", "Dm7", "Em7", "Fmaj7", "G7", "Am7", "Bø7"]);
        // A harmonic minor's dominant is major; flat keys are spelt with flats.
        let a_harm = Harmony {
            key: 9,
            scale: 6,
            sevenths: false,
        };
        assert_eq!(a_harm.chord(4).name(&a_harm), "E");
        let f_major = Harmony {
            key: 5,
            scale: 0,
            sevenths: false,
        };
        assert_eq!(f_major.chord(3).name(&f_major), "Bb");
        let d_minor = Harmony {
            key: 2,
            scale: 1,
            sevenths: false,
        };
        assert_eq!(d_minor.chord(5).name(&d_minor), "Bb");
        assert_eq!(d_minor.label(), "D natural minor");
    }

    #[test]
    fn tones_have_roles_over_the_chord() {
        let c = C_MAJOR.chord(0);
        let roles: Vec<Role> = (0..7).map(|d| c.role(C_MAJOR.pc(d))).collect();
        use Role::*;
        // C D E F G A B over C: F clashes with E; D, A and B are colour.
        assert_eq!(
            roles,
            [Chord, Tension, Chord, Avoid, Chord, Tension, Tension]
        );
        // Over G, C is the avoid note.
        assert_eq!(C_MAJOR.chord(4).role(0), Avoid);
    }

    #[test]
    fn snapping_keeps_strong_steps_on_the_chord() {
        let c = C_MAJOR.chord(0);
        // F on the downbeat falls to E; D falls to C.
        assert_eq!(snap(&C_MAJOR, &c, 3, (0, 15), 0, 1.0), 2);
        assert_eq!(snap(&C_MAJOR, &c, 1, (0, 15), 0, 1.0), 0);
        // On a weak step with TENSION, D stays.
        assert_eq!(snap(&C_MAJOR, &c, 1, (0, 15), 3, 0.5), 1);
        // At the bottom of the range it goes up instead.
        assert_eq!(snap(&C_MAJOR, &C_MAJOR.chord(4), 0, (0, 15), 0, 0.0), 1);
    }

    #[test]
    fn progressions_wander_within_their_function() {
        let p = Progression {
            degrees: [0, 4, 5, 3, 0, 0, 0, 0],
            len: 4,
            beats: 4.0,
            wander: true,
            seed: 7,
        };
        assert_eq!(p.slot_at(0.0), (0, 0));
        assert_eq!(p.slot_at(5.0), (1, 0));
        assert_eq!(p.slot_at(17.0), (0, 1));
        // The first time round as written.
        assert_eq!(
            (0..4).map(|s| p.degree(s, 0)).collect::<Vec<_>>(),
            [0, 4, 5, 3]
        );
        let mut changed = false;
        for cycle in 1..40 {
            for slot in 0..4 {
                let d = p.degree(slot, cycle);
                assert!(family(p.degrees[slot]).contains(&d));
                changed |= d != p.degrees[slot];
            }
            assert_eq!(p.degree(0, cycle), 0, "home stays home");
        }
        assert!(changed);
    }
}
