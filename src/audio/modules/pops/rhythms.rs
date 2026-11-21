//! The twenty rhythms, in the Mini Pops-7's two rows. Each is two bars of
//! sixteenths, or of eighth-note triplets for the swung and 6/8 ones (the
//! original slowed its clock by 3/4 for those). The originals live in a
//! diode matrix; these are new patterns in each style.

use super::voices::{
    BASE, BONGO_L, BONGO_S, CLAVES, CONGA, COUNT, COWBELL, CYMBAL1, CYMBAL2, GUIRO, HIHAT, QUIJADA,
    RIMSHOT, SNARE1, SNARE2, TAMBOURINE,
};

/// Most steps a pattern can have.
pub const MAX_LEN: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rhythm {
    pub name: &'static str,
    /// Steps per beat: 4 (sixteenths) or 3 (eighth-note triplets).
    pub per_beat: usize,
    /// Steps in the pattern.
    pub len: usize,
    /// One bit per step, per instrument.
    pub rows: [u32; COUNT],
}

/// `x` hits and `.` rests (spaces ignored) over `len` steps, the written
/// steps repeating if there are fewer.
const fn mask(s: &str, len: usize) -> u32 {
    let b = s.as_bytes();
    let mut steps = [false; MAX_LEN];
    let mut n = 0;
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'x' || b[i] == b'.' {
            steps[n] = b[i] == b'x';
            n += 1;
        }
        i += 1;
    }
    let mut m = 0u32;
    let mut k = 0;
    while n > 0 && k < len {
        if steps[k % n] {
            m |= 1 << k;
        }
        k += 1;
    }
    m
}

/// A two-bar rhythm of `bar` steps per bar.
const fn rhythm(name: &'static str, per_beat: usize, bar: usize, hits: &[(usize, &str)]) -> Rhythm {
    let len = bar * 2;
    let mut rows = [0u32; COUNT];
    let mut i = 0;
    while i < hits.len() {
        rows[hits[i].0] = mask(hits[i].1, len);
        i += 1;
    }
    Rhythm {
        name,
        per_beat,
        len,
        rows,
    }
}

/// Top row (black labels), then bottom row (red), as on the panel.
pub const RHYTHMS: [Rhythm; 20] = [
    rhythm(
        "WALTZ",
        4,
        12,
        &[
            (BASE, "x... .... ...."),
            (SNARE2, ".... x... x..."),
            (HIHAT, "x.x. x.x. x.x."),
            (TAMBOURINE, ".... x... x..."),
        ],
    ),
    rhythm(
        "SAMBA",
        4,
        16,
        &[
            (BASE, "x..x x..x x..x x..x"),
            (SNARE2, "..x. .x.x ..x. .x.x"),
            (HIHAT, "xxxx xxxx xxxx xxxx"),
            (CLAVES, "x..x ..x. ..x. x..."),
            (CONGA, ".... ..x. .... ..x."),
            (TAMBOURINE, "x.x. x.x. x.x. x.x."),
        ],
    ),
    rhythm(
        "MAMBO",
        4,
        16,
        &[
            (BASE, "x... ..x. x... ..x."),
            (COWBELL, "x.x. x.x. x.x. x.x."),
            (BONGO_S, "..xx ..xx ..xx ..xx"),
            (BONGO_L, "x... x... x... x..."),
            (CLAVES, "..x. x... x..x ..x."),
            (CONGA, ".... ..xx .... ..xx"),
            (GUIRO, "x... x... x... x..."),
        ],
    ),
    rhythm(
        "RUMBA",
        4,
        16,
        &[
            (BASE, "x... ..x. x... ...."),
            (CLAVES, "x..x ..x. ..x. x..."),
            (BONGO_S, ".x.x .x.x .x.x .x.x"),
            (CONGA, ".... ..x. ..x. ..xx"),
            (HIHAT, "x.x. x.x. x.x. x.x."),
            (QUIJADA, "x... .... .... .... .... .... .... ...."),
        ],
    ),
    rhythm(
        "TANGO",
        4,
        16,
        &[
            (BASE, "x... x... x... x.x."),
            (SNARE2, ".... .... x.x. x..."),
            (HIHAT, "x.x. x.x. x.x. x.x."),
            (TAMBOURINE, ".... x... .... x..."),
        ],
    ),
    rhythm(
        "BOSSANOVA",
        4,
        16,
        &[
            (BASE, "x..x x..x x..x x..x"),
            (RIMSHOT, "x..x ..x. ..x. .x.."),
            (HIHAT, "xxxx xxxx xxxx xxxx"),
        ],
    ),
    rhythm(
        "ROCK 1",
        4,
        16,
        &[
            (BASE, "x... .... x... ...."),
            (SNARE1, ".... x... .... x..."),
            (HIHAT, "x.x. x.x. x.x. x.x."),
        ],
    ),
    rhythm(
        "ROCK 3",
        4,
        16,
        &[
            (BASE, "x... ..x. x.x. .... x... ..x. x... ...."),
            (SNARE1, ".... x... .... x... .... x... .... x.x."),
            (HIHAT, "x.x. x.x. x.x. x.x."),
            (CYMBAL1, "x... .... .... .... .... .... .... ...."),
        ],
    ),
    rhythm(
        "MARCH 2/4",
        4,
        16,
        &[
            (BASE, "x... .... x... ...."),
            (SNARE1, "x.xx x.x. x.xx x.x."),
            (CYMBAL1, ".... x... .... x..."),
        ],
    ),
    rhythm(
        "SWING",
        3,
        12,
        &[
            (BASE, "x.. ... x.. ..."),
            (CYMBAL2, "x.. x.x x.. x.x"),
            (HIHAT, "... x.. ... x.."),
            (SNARE2, "... ... ... ..x"),
        ],
    ),
    rhythm(
        "JAZZ WALTZ",
        3,
        9,
        &[
            (BASE, "x.. ... ..."),
            (CYMBAL2, "x.. x.x x.."),
            (HIHAT, "... x.. x.."),
            (SNARE2, "... ... ..x"),
        ],
    ),
    rhythm(
        "MERINGUE",
        4,
        16,
        &[
            (BASE, "x... x... x... x..."),
            (SNARE2, "x..x .x.. x..x .xx."),
            (GUIRO, "x.x. x.x. x.x. x.x."),
            (COWBELL, "x... x... x... x..."),
            (CONGA, "..x. ..x. ..x. ..x."),
        ],
    ),
    rhythm(
        "CHA-CHA",
        4,
        16,
        &[
            (BASE, "x... .... x... ...."),
            (COWBELL, "x... x... x... x..."),
            (GUIRO, "x... x.x. x... x.x."),
            (CONGA, ".... ..x. x... x.x."),
            (SNARE2, ".... .... .... x.x."),
            (CLAVES, "..x. x... x..x ..x."),
        ],
    ),
    rhythm(
        "BEGUINE",
        4,
        16,
        &[
            (BASE, "x... ..x. x... ...."),
            (CONGA, "..x. x... ..x. x..."),
            (BONGO_S, "x..x ..x. x..x ..x."),
            (HIHAT, "x.x. x.x. x.x. x.x."),
            (TAMBOURINE, ".... x... .... x..."),
        ],
    ),
    rhythm(
        "HABANERA",
        4,
        16,
        &[
            (BASE, "x..x x... x..x x..."),
            (SNARE2, ".... x... .... x..."),
            (TAMBOURINE, "x.x. x.x. x.x. x.x."),
            (BONGO_L, "x... .... x... ...."),
        ],
    ),
    rhythm(
        "SLOW ROCK",
        3,
        12,
        &[
            (BASE, "x.. ... x.x ..."),
            (SNARE1, "... x.. ... x.."),
            (HIHAT, "xxx xxx xxx xxx"),
        ],
    ),
    rhythm(
        "ROCK 2",
        4,
        16,
        &[
            (BASE, "x... ..x. x... ...."),
            (SNARE1, ".... x... .... x..."),
            (HIHAT, "xxxx xxxx xxxx xxxx"),
        ],
    ),
    rhythm(
        "ROCK 4",
        4,
        16,
        &[
            (BASE, "x..x ..x. x..x ..x."),
            (SNARE1, ".... x..x .x.. x..."),
            (HIHAT, "x.x. x.x. x.x. x.x."),
            (CYMBAL1, "x... .... .... .... .... .... .... ...."),
        ],
    ),
    rhythm(
        "MARCH 6/8",
        3,
        6,
        &[(BASE, "x.. x.."), (SNARE1, "x.x xx."), (CYMBAL1, "... x..")],
    ),
    rhythm(
        "LATIN SWING",
        3,
        12,
        &[
            (BASE, "x.. ... x.. ..."),
            (COWBELL, "x.. x.x x.. x.x"),
            (CONGA, "... x.. ... x.x"),
            (CLAVES, "x.. .x. ..x x.."),
            (QUIJADA, "x.. ... ... ... ... ... ... ..."),
        ],
    ),
];

/// Rhythms per row.
pub const ROW: usize = 10;

/// The rhythms selected in a row played together, as the Mini Pops did:
/// on the first one's grid, each adding the hits that fall on its steps.
pub fn combine(selected: &[usize]) -> Option<Rhythm> {
    let first = *selected.first()?;
    let mut out = RHYTHMS[first];
    for &r in &selected[1..] {
        let other = &RHYTHMS[r];
        for s in 0..out.len {
            // Where step `s` falls on the other rhythm's grid, if on a step.
            let at = s * other.per_beat;
            if !at.is_multiple_of(out.per_beat) {
                continue;
            }
            let j = (at / out.per_beat) % other.len;
            for (row, &bits) in out.rows.iter_mut().zip(&other.rows) {
                if bits >> j & 1 == 1 {
                    *row |= 1 << s;
                }
            }
        }
    }
    Some(out)
}
