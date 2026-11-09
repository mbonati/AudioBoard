//! The notes for Colloid: handpan and Hang layouts (a ding and its tone
//! fields, as the instruments are tuned), chords with the colour tones each
//! one takes, and how either becomes a set of notes, one per molecule.

use super::physics::MAX_MOLECULES;
use crate::audio::params::NOTE_NAMES;

/// Handpan and Hang layouts: (label, long name, the tone fields above the
/// ding, in semitones, low to high). Every note sits well with every other,
/// so whatever the atoms strike, the line is a melody.
pub const HANDPANS: [(&str, &str, &[u8]); 9] = [
    ("KURD", "Kurd: the classic minor handpan (on D: D | A Bb C D E F G A C)", &[7, 8, 10, 12, 14, 15, 17, 19, 22]),
    ("CELTIC", "Celtic minor: minor without the sixth (on D: D | A C D E F G A C)", &[7, 10, 12, 14, 15, 17, 19, 22]),
    ("AMARA", "Amara: Celtic minor without the fourth, open and airy (on D: D | A C D E F A C D)", &[7, 10, 12, 14, 15, 19, 22, 24]),
    ("INTEGRAL", "Integral: the first Hang's sound, minor with the sixth (on D: D | A Bb C D E F A)", &[7, 8, 10, 12, 14, 15, 19]),
    ("HIJAZ", "Hijaz: Middle Eastern, with the raised seventh (on D: D | A Bb C# D E F G A)", &[7, 8, 11, 12, 14, 15, 17, 19]),
    ("PYGMY", "Pygmy: a bright minor pentatonic (on D: D | E F A C D E F A)", &[2, 3, 7, 10, 12, 14, 15, 19]),
    ("AKEBONO", "Akebono: Japanese pentatonic (on D: D | A Bb D E F A Bb)", &[7, 8, 12, 14, 15, 19, 20]),
    ("SABYE", "Sabye: sunny major without the fourth (on D: D | A B C# D E F# A B)", &[7, 9, 11, 12, 14, 16, 19, 21]),
    ("AEGEAN", "Aegean: dreamy major with the raised fourth (on D: D | A C# D F# G# A C# D)", &[7, 11, 12, 16, 18, 19, 23, 24]),
];

/// The shapes on the selector: the handpans, then the chords.
pub const SHAPES: usize = HANDPANS.len() + CHORDS.len();
pub const FIRST_CHORD: usize = HANDPANS.len();

/// A shape's label and long name.
pub fn shape_name(shape: usize) -> (&'static str, &'static str) {
    match shape.min(SHAPES - 1) {
        s if s < FIRST_CHORD => (HANDPANS[s].0, HANDPANS[s].1),
        s => (CHORDS[s - FIRST_CHORD].0, CHORDS[s - FIRST_CHORD].1),
    }
}

/// (label, long name, chord tones and colour tones, as semitones above the
/// root).
pub const CHORDS: [(&str, &str, &[u8], &[u8]); 12] = [
    ("MAJ", "Major triad", &[0, 4, 7], &[2, 9, 11]),
    ("MIN", "Minor triad", &[0, 3, 7], &[2, 5, 10]),
    ("SUS2", "Suspended second", &[0, 2, 7], &[5, 9, 10]),
    ("SUS4", "Suspended fourth", &[0, 5, 7], &[2, 9, 10]),
    ("MAJ7", "Major seventh", &[0, 4, 7, 11], &[2, 6, 9]),
    ("MIN7", "Minor seventh", &[0, 3, 7, 10], &[2, 5, 9]),
    ("DOM7", "Dominant seventh", &[0, 4, 7, 10], &[2, 9, 6]),
    ("MAJ9", "Major ninth", &[0, 4, 7, 11, 2], &[6, 9]),
    ("MIN9", "Minor ninth", &[0, 3, 7, 10, 2], &[5, 9]),
    ("6/9", "Six-nine", &[0, 4, 7, 9, 2], &[6, 11]),
    ("QUART", "Quartal: fourths stacked on the root", &[0, 5, 10, 3], &[8, 2]),
    ("PENTA", "Pentatonic cloud", &[0, 2, 4, 7, 9], &[11, 6]),
];

/// What a note is in the chord: the molecule's shape and colour.
pub const ROOT: u8 = 0;
pub const THIRD: u8 = 1;
pub const FIFTH: u8 = 2;
pub const SEVENTH: u8 = 3;
/// A colour tone (9th, 11th, 13th, or anything else).
pub const COLOUR: u8 = 4;

pub const ROLES: [&str; 5] = ["root", "third", "fifth", "seventh", "colour"];

/// A chord as pitch classes: bit `pc` of `mask` (C = bit 0), its root, and
/// the colour tones added to it (`colours`, bits too, not in `mask`); or a
/// handpan's layout on `root` (its ding), with its notes in `mask`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ChordSet {
    pub root: u8,
    pub mask: u16,
    pub colours: u16,
    pub layout: Option<u8>,
}

/// The pitch classes of `intervals` above `root`, as bits.
pub fn mask_of(root: u8, intervals: &[u8]) -> u16 {
    intervals.iter().fold(0, |m, &i| m | 1 << ((root as u16 + i as u16) % 12))
}

impl ChordSet {
    /// Shape `shape` on `root`: a handpan's layout, or a chord with each of
    /// its colour tones in when `pick(k)` (k-th colour) says so.
    pub fn new(root: u8, shape: usize, pick: impl Fn(usize) -> bool) -> Self {
        let shape = shape.min(SHAPES - 1);
        if shape < FIRST_CHORD {
            return Self::handpan(root, shape);
        }
        let (_, _, tones, colours) = CHORDS[shape - FIRST_CHORD];
        let mask = mask_of(root, tones);
        let colours = colours.iter().enumerate().filter(|&(k, _)| pick(k)).fold(0, |m, (_, &i)| m | 1 << ((root as u16 + i as u16) % 12));
        Self { root: root % 12, mask, colours: colours & !mask, layout: None }
    }

    /// Handpan `layout` with its ding on `root`.
    pub fn handpan(root: u8, layout: usize) -> Self {
        let layout = layout.min(HANDPANS.len() - 1);
        let mask = mask_of(root, HANDPANS[layout].2) | 1 << (root % 12);
        Self { root: root % 12, mask, colours: 0, layout: Some(layout as u8) }
    }

    /// Every pitch class that sounds.
    pub fn all(&self) -> u16 {
        self.mask | self.colours
    }

    pub fn contains(&self, pc: u8) -> bool {
        self.all() >> (pc % 12) & 1 == 1
    }

    /// What pitch class `pc` is in the chord.
    pub fn role(&self, pc: u8) -> u8 {
        let pc = pc % 12;
        if self.colours >> pc & 1 == 1 {
            return COLOUR;
        }
        match (pc + 12 - self.root) % 12 {
            0 => ROOT,
            3 | 4 => THIRD,
            6..=8 => FIFTH,
            10 | 11 => SEVENTH,
            _ => COLOUR,
        }
    }

    /// The chord's name: a shape of the selector on its root ("D MIN9"), or
    /// else its notes ("C E F# B").
    pub fn name(&self) -> String {
        let root = NOTE_NAMES[self.root as usize];
        if let Some(l) = self.layout {
            return format!("{root} {}", HANDPANS[l as usize % HANDPANS.len()].0);
        }
        if let Some((label, ..)) = CHORDS.iter().find(|(_, _, tones, _)| mask_of(self.root, tones) == self.mask) {
            return format!("{root} {label}");
        }
        (0..12).map(|i| (self.root + i) % 12).filter(|&pc| self.mask >> pc & 1 == 1).map(|pc| NOTE_NAMES[pc as usize]).collect::<Vec<_>>().join(" ")
    }
}

/// The notes of `set` from `low` up `octaves` octaves (the root on top
/// too), `count` of them spread evenly from the bottom; when the register
/// holds too few, it reaches higher. A handpan plays its own layout from its
/// ding on `low` (`octaves` aside), and an octave up if it needs more notes.
/// Returns how many it wrote (sorted, all different).
pub fn voicing(set: &ChordSet, low: i32, octaves: i32, count: usize, out: &mut [u8; MAX_MOLECULES]) -> usize {
    let count = count.min(MAX_MOLECULES);
    if count == 0 || set.all() == 0 {
        return 0;
    }
    let mut candidates = [0u8; 128];
    let mut high = low + 12 * octaves.max(1);
    let low = low.clamp(0, 127);
    let len = if let Some(l) = set.layout {
        let fields = HANDPANS[l as usize % HANDPANS.len()].2;
        let mut len = 0;
        'octaves: for octave in 0..4 {
            for &n in std::iter::once(&0).chain(fields) {
                let pitch = low + n as i32 + 12 * octave;
                if pitch > 127 || len >= count.max(fields.len() + 1) {
                    break 'octaves;
                }
                if len == 0 || pitch > candidates[len - 1] as i32 {
                    candidates[len] = pitch as u8;
                    len += 1;
                }
            }
        }
        len
    } else {
        loop {
        let mut len = 0;
        for pitch in low..=high.min(127) {
            if set.contains(pitch as u8) {
                candidates[len] = pitch as u8;
                len += 1;
            }
        }
        if len >= count || high >= 127 {
            break len;
        }
        high += 12;
        }
    };
    if len == 0 {
        return 0;
    }
    // Evenly spaced in pitch from the bottom; near each place, the chord
    // tone heard least so far, so every tone gets its molecules (an even
    // pick by index would land on the same tones over and over).
    let count = count.min(len);
    let lowest = candidates[0] as f32;
    let span = (candidates[len - 1] - candidates[0]) as f32;
    let mut used = [0u8; 12];
    let mut start = 0;
    for (k, slot) in out.iter_mut().enumerate().take(count) {
        let target = if count == 1 { lowest } else { lowest + span * k as f32 / (count - 1) as f32 };
        let dist = |i: usize| (candidates[i] as f32 - target).abs();
        let score = |i: usize| used[candidates[i] as usize % 12] as f32 * 8.0 + dist(i);
        // Room is left above for the molecules still to place.
        let last = len - (count - k);
        let nearest = (start..=last).min_by(|&a, &b| dist(a).total_cmp(&dist(b))).unwrap_or(start);
        let pick = (start..=last).filter(|&i| dist(i) <= 4.5).min_by(|&a, &b| score(a).total_cmp(&score(b))).unwrap_or(nearest);
        *slot = candidates[pick];
        used[candidates[pick] as usize % 12] += 1;
        start = pick + 1;
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spreads_the_chord() {
        let set = ChordSet::new(0, FIRST_CHORD, |_| false);
        let mut out = [0; MAX_MOLECULES];
        // C major from C3 over two octaves: C E G C E G C, four of them.
        let n = voicing(&set, 48, 2, 4, &mut out);
        assert_eq!(&out[..n], &[48, 55, 64, 72]);
        // More molecules than the register holds: it reaches up.
        let n = voicing(&set, 48, 1, 9, &mut out);
        assert_eq!(n, 9);
        assert!(out[..n].windows(2).all(|w| w[0] < w[1]));
        assert!(out[..n].iter().all(|&p| set.contains(p)));
        // Quartal D (D G C F) over three octaves: every tone, spread out.
        let quartal = ChordSet::new(2, FIRST_CHORD + 10, |_| false);
        let n = voicing(&quartal, 50, 3, 7, &mut out);
        let pcs: std::collections::BTreeSet<u8> = out[..n].iter().map(|p| p % 12).collect();
        assert_eq!(pcs.len(), 4, "{:?}", &out[..n]);
        assert_eq!(out[0], 50);
    }

    #[test]
    fn roles_and_names() {
        let set = ChordSet::new(2, FIRST_CHORD + 8, |_| false); // D MIN9
        assert_eq!(set.name(), "D MIN9");
        assert_eq!(set.role(2), ROOT);
        assert_eq!(set.role(5), THIRD);
        assert_eq!(set.role(9), FIFTH);
        assert_eq!(set.role(0), SEVENTH);
        assert_eq!(set.role(4), COLOUR);
        let coloured = ChordSet::new(0, FIRST_CHORD, |k| k == 0);
        assert!(coloured.contains(2));
        assert_eq!(coloured.role(2), COLOUR);
        let odd = ChordSet { root: 0, mask: mask_of(0, &[0, 1, 6]), colours: 0, layout: None };
        assert_eq!(odd.name(), "C C# F#");
    }

    #[test]
    fn handpans_play_their_layout() {
        let kurd = ChordSet::new(2, 0, |_| true);
        assert_eq!(kurd.name(), "D KURD");
        assert_eq!(kurd.colours, 0, "a handpan takes no colour tones");
        let mut out = [0; MAX_MOLECULES];
        // D3 | A3 Bb3 C4 D4 E4 F4 G4 A4 C5: all ten.
        let n = voicing(&kurd, 50, 3, 10, &mut out);
        assert_eq!(&out[..n], &[50, 57, 58, 60, 62, 64, 65, 67, 69, 72]);
        // Fewer molecules: the ding, then spread over the fields.
        let n = voicing(&kurd, 50, 3, 5, &mut out);
        assert_eq!(out[0], 50);
        assert_eq!(n, 5);
        assert!(out[..n].iter().all(|p| [50, 57, 58, 60, 62, 64, 65, 67, 69, 72].contains(p)));
        // More: the layout again an octave up.
        let n = voicing(&kurd, 50, 3, 14, &mut out);
        assert_eq!(n, 14);
        assert!(out[..n].windows(2).all(|w| w[0] < w[1]));
        assert!(out[..n].iter().all(|&p| kurd.contains(p)));
    }
}
