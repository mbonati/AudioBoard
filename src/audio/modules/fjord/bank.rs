//! The factory programs. Like the Nord Drum's, each is four channels in a
//! style (Real: acoustic, Retro: machines and old school, Ethno: world
//! music, FX: weird tweaks) and a category: Drums (kick, snare and two toms,
//! to play with acoustic cymbals), Perc (four pads for a percussionist) or
//! Kit (kick, snare, tom and hihat, for a sequencer). The sounds are our
//! own; the last two use the tone's ADSRs, which the Nord Drum doesn't
//! have. The later locations start empty (Sound Init on every channel) for
//! programs of your own.

use super::knob::*;
use super::voice::Sound;

pub struct Program {
    pub name: &'static str,
    pub style: &'static str,
    pub category: &'static str,
    pub channels: [Sound; 4],
}

/// Program locations, P1 to P99.
pub const PROGRAMS: usize = 99;

// Waveforms (see `voice::WAVES`).
const AN1: f32 = 0.0;
const AN2: f32 = 1.0;
const AN3: f32 = 2.0;
const B1: f32 = 3.0;
const B2: f32 = 4.0;
const B3: f32 = 5.0;
const B4: f32 = 6.0;
const B5: f32 = 7.0;
const B6: f32 = 8.0;
const B7: f32 = 9.0;
const B8: f32 = 10.0;
const B9: f32 = 11.0;
const C1: f32 = 12.0;
const C2: f32 = 13.0;
const C3: f32 = 14.0;
const C4: f32 = 15.0;
const C5: f32 = 16.0;

/// Click types: n1…n9, c1…c9, t1…t9.
const fn n(i: u8) -> f32 {
    (i - 1) as f32
}
const fn c(i: u8) -> f32 {
    (i + 8) as f32
}
const fn t(i: u8) -> f32 {
    (i + 17) as f32
}

// Gates.
const S: f32 = 1.0;
const G: f32 = 2.0;

/// Sound Init: a plain sine drum, the noise and the click mixed out.
pub const INIT: Sound = {
    let mut s = [0.0; COUNT];
    s[N_COLOR] = 0.0;
    s[N_FILTER] = 70.0;
    s[N_TYPE] = 1.0;
    s[N_TIME] = 5.0;
    s[N_DECAY] = 20.0;
    s[T_WAVE] = AN1;
    s[T_FILTER] = 99.0;
    s[T_TIME] = 5.0;
    s[T_DECAY] = 25.0;
    s[PITCH] = 48.0;
    s[BEND_TIME] = 5.0;
    s[C_TYPE] = 4.0;
    s[C_DECAY] = 5.0;
    s[TONE] = 10.0;
    s[LEVEL] = 80.0;
    // The envelopes beyond the Nord Drum: ready, but standing aside (the
    // amp follows DECAY, the filter's AMOUNT is 0).
    s[AMP_D] = 25.0;
    s[AMP_S] = 70.0;
    s[AMP_R] = 25.0;
    s[FLT_D] = 20.0;
    s[FLT_R] = 20.0;
    s
};

/// `base` with some settings changed.
const fn with(base: Sound, set: &[(usize, f32)]) -> Sound {
    let mut s = base;
    let mut i = 0;
    while i < set.len() {
        s[set[i].0] = set[i].1;
        i += 1;
    }
    s
}

// ---------------------------------------------------------------------------
// Starting points
// ---------------------------------------------------------------------------

const KICK_TIGHT: Sound = with(
    INIT,
    &[
        (PITCH, 31.0),
        (T_DECAY, 31.0),
        (BEND, 18.0),
        (BEND_TIME, 3.0),
        (T_PUNCH, 5.0),
        (C_TYPE, t(4)),
        (C_DECAY, 3.0),
        (CLICK, 5.0),
        (NOISE, 2.0),
        (N_COLOR, -2.0),
        (N_FILTER, 60.0),
        (N_DECAY, 8.0),
        (LEVEL, 88.0),
    ],
);
const KICK_BOOM: Sound = with(
    INIT,
    &[
        (PITCH, 29.0),
        (T_DECAY, 37.0),
        (BEND, 5.0),
        (BEND_TIME, 4.0),
        (T_PUNCH, 2.0),
        (C_TYPE, t(2)),
        (C_DECAY, 2.0),
        (CLICK, 2.0),
        (LEVEL, 90.0),
    ],
);
const KICK_REAL: Sound = with(
    INIT,
    &[
        (T_WAVE, B1),
        (PITCH, 33.0),
        (T_DECAY, 28.0),
        (BEND, 7.0),
        (BEND_TIME, 3.0),
        (T_FILTER, 70.0),
        (T_PUNCH, 4.0),
        (NOISE, 4.0),
        (N_COLOR, -5.0),
        (N_FILTER, 55.0),
        (N_DECAY, 14.0),
        (N_SWEEP, 3.0),
        (N_TIME, 3.0),
        (CLICK, 6.0),
        (C_TYPE, c(4)),
        (C_DECAY, 3.0),
        (LEVEL, 88.0),
    ],
);
const KICK_FM: Sound = with(
    INIT,
    &[
        (T_WAVE, B7),
        (PITCH, 34.0),
        (T_DECAY, 30.0),
        (BEND, 14.0),
        (BEND_TIME, 2.0),
        (T_PUNCH, 3.0),
        (CLICK, 3.0),
        (C_TYPE, t(5)),
        (C_DECAY, 2.0),
        (LEVEL, 88.0),
    ],
);

const SNARE_TIGHT: Sound = with(
    INIT,
    &[
        (T_WAVE, B3),
        (PITCH, 56.0),
        (T_DECAY, 21.0),
        (BEND, 6.0),
        (BEND_TIME, 2.0),
        (T_FILTER, 85.0),
        (T_PUNCH, 3.0),
        (TONE, 7.0),
        (NOISE, 8.0),
        (N_COLOR, 3.0),
        (N_FILTER, 84.0),
        (N_TYPE, 2.0),
        (N_SWEEP, 3.0),
        (N_TIME, 3.0),
        (N_DECAY, 28.0),
        (CLICK, 5.0),
        (C_TYPE, c(6)),
        (C_DECAY, 3.0),
    ],
);
const SNARE_BOX: Sound = with(
    INIT,
    &[
        (T_WAVE, B3),
        (PITCH, 54.0),
        (T_DECAY, 19.0),
        (BEND, 2.0),
        (BEND_TIME, 2.0),
        (T_FILTER, 70.0),
        (TONE, 8.0),
        (NOISE, 7.0),
        (N_COLOR, 5.0),
        (N_FILTER, 88.0),
        (N_DECAY, 24.0),
        (CLICK, 3.0),
        (C_TYPE, n(7)),
        (C_DECAY, 2.0),
    ],
);
const SNARE_REAL: Sound = with(
    INIT,
    &[
        (T_WAVE, B3),
        (PITCH, 57.0),
        (T_DECAY, 24.0),
        (BEND, 4.0),
        (BEND_TIME, 2.0),
        (T_FILTER, 80.0),
        (T_PUNCH, 4.0),
        (TONE, 6.0),
        (NOISE, 9.0),
        (N_COLOR, 2.0),
        (N_FILTER, 90.0),
        (N_TYPE, 2.0),
        (N_SWEEP, 5.0),
        (N_TIME, 4.0),
        (N_DECAY, 31.0),
        (CLICK, 6.0),
        (C_TYPE, c(7)),
        (C_DECAY, 4.0),
    ],
);
const SNARE_BRUSH: Sound = with(
    INIT,
    &[
        (T_WAVE, B3),
        (PITCH, 55.0),
        (T_DECAY, 18.0),
        (T_FILTER, 60.0),
        (TONE, 3.0),
        (NOISE, 9.0),
        (N_COLOR, 1.0),
        (N_FILTER, 78.0),
        (N_DECAY, 32.0),
        (N_GATE, S),
        (CLICK, 2.0),
        (C_TYPE, n(8)),
        (C_DECAY, 6.0),
    ],
);
const SNARE_GATED: Sound = with(
    SNARE_TIGHT,
    &[
        (PITCH, 55.0),
        (T_DECAY, 27.0),
        (T_GATE, G),
        (TONE, 6.0),
        (NOISE, 9.0),
        (N_COLOR, 2.0),
        (N_FILTER, 86.0),
        (N_DECAY, 31.0),
        (N_GATE, G),
    ],
);

const TOM: Sound = with(
    INIT,
    &[
        (T_WAVE, B2),
        (PITCH, 47.0),
        (T_DECAY, 31.0),
        (BEND, 4.0),
        (BEND_TIME, 5.0),
        (T_FILTER, 78.0),
        (T_PUNCH, 3.0),
        (NOISE, 2.0),
        (N_COLOR, -3.0),
        (N_FILTER, 55.0),
        (N_DECAY, 16.0),
        (CLICK, 3.0),
        (C_TYPE, n(3)),
        (C_DECAY, 3.0),
    ],
);
const TOM_HEX: Sound = with(
    INIT,
    &[
        (T_WAVE, B7),
        (PITCH, 45.0),
        (T_DECAY, 33.0),
        (BEND, 12.0),
        (BEND_TIME, 5.0),
        (T_FILTER, 70.0),
        (T_SWEEP, 4.0),
        (T_TIME, 4.0),
        (NOISE, 5.0),
        (N_COLOR, -1.0),
        (N_FILTER, 58.0),
        (N_SWEEP, 6.0),
        (N_TIME, 3.0),
        (N_DECAY, 22.0),
        (CLICK, 4.0),
        (C_TYPE, c(4)),
        (C_DECAY, 3.0),
    ],
);
const TOM_SAW: Sound = with(
    INIT,
    &[
        (T_WAVE, AN3),
        (PITCH, 50.0),
        (T_DECAY, 30.0),
        (BEND, 10.0),
        (BEND_TIME, 6.0),
        (T_FILTER, 45.0),
        (T_SWEEP, 7.0),
        (T_TIME, 4.0),
        (T_PUNCH, 2.0),
    ],
);

const HAT_CLOSED: Sound = with(
    INIT,
    &[
        (T_WAVE, C3),
        (PITCH, 92.0),
        (T_DECAY, 12.0),
        (TONE, 5.0),
        (NOISE, 9.0),
        (N_COLOR, 8.0),
        (N_FILTER, 96.0),
        (N_DECAY, 14.0),
        (CLICK, 3.0),
        (C_TYPE, n(9)),
        (C_DECAY, 2.0),
        (LEVEL, 80.0),
    ],
);
const HAT_OPEN: Sound = with(HAT_CLOSED, &[(T_DECAY, 30.0), (N_DECAY, 34.0)]);
const HAT_METAL: Sound = with(
    HAT_CLOSED,
    &[
        (PITCH, 87.0),
        (T_DECAY, 14.0),
        (T_FILTER, 92.0),
        (TONE, 7.0),
        (N_COLOR, 9.0),
        (N_FILTER, 95.0),
        (N_DECAY, 12.0),
    ],
);

const CLAP: Sound = with(
    INIT,
    &[
        (TONE, 0.0),
        (NOISE, 10.0),
        (N_COLOR, 4.0),
        (N_FILTER, 74.0),
        (N_TYPE, 3.0),
        (N_SWEEP, 2.0),
        (N_TIME, 2.0),
        (N_DECAY, 25.0),
        (N_GATE, S),
        (CLICK, 6.0),
        (C_TYPE, n(6)),
        (C_DECAY, 7.0),
        (C_GATE, S),
    ],
);
const RIM: Sound = with(
    INIT,
    &[
        (T_WAVE, B9),
        (PITCH, 76.0),
        (T_DECAY, 10.0),
        (T_FILTER, 90.0),
        (T_PUNCH, 4.0),
        (TONE, 8.0),
        (NOISE, 3.0),
        (N_COLOR, 6.0),
        (N_FILTER, 85.0),
        (N_DECAY, 7.0),
        (CLICK, 7.0),
        (C_TYPE, c(7)),
        (C_DECAY, 2.0),
        (LEVEL, 72.0),
    ],
);
const COWBELL: Sound = with(
    INIT,
    &[
        (T_WAVE, C2),
        (PITCH, 73.0),
        (T_DECAY, 26.0),
        (T_FILTER, 78.0),
        (T_PUNCH, 2.0),
        (CLICK, 3.0),
        (C_TYPE, n(7)),
        (C_DECAY, 2.0),
        (LEVEL, 70.0),
    ],
);
const CONGA: Sound = with(
    INIT,
    &[
        (T_WAVE, B6),
        (PITCH, 60.0),
        (T_DECAY, 26.0),
        (BEND, 2.0),
        (BEND_TIME, 3.0),
        (T_FILTER, 85.0),
        (T_PUNCH, 2.0),
        (CLICK, 3.0),
        (C_TYPE, n(5)),
        (C_DECAY, 2.0),
    ],
);
const SLAP: Sound = with(
    CONGA,
    &[
        (PITCH, 64.0),
        (T_DECAY, 16.0),
        (BEND, 3.0),
        (BEND_TIME, 2.0),
        (T_FILTER, 95.0),
        (T_PUNCH, 6.0),
        (NOISE, 4.0),
        (N_COLOR, 4.0),
        (N_FILTER, 80.0),
        (N_DECAY, 12.0),
        (CLICK, 6.0),
        (C_TYPE, c(6)),
        (C_DECAY, 3.0),
    ],
);
const TABLA: Sound = with(
    INIT,
    &[
        (T_WAVE, B5),
        (PITCH, 64.0),
        (T_DECAY, 26.0),
        (T_FILTER, 90.0),
        (T_PUNCH, 3.0),
        (CLICK, 4.0),
        (C_TYPE, c(7)),
        (C_DECAY, 2.0),
    ],
);
/// The tabla's bass drum: its pitch slides up as the palm presses.
const BAYAN: Sound = with(
    INIT,
    &[
        (T_WAVE, B4),
        (PITCH, 41.0),
        (T_DECAY, 34.0),
        (BEND, -7.0),
        (BEND_TIME, 7.0),
        (T_FILTER, 60.0),
        (CLICK, 2.0),
        (C_TYPE, t(1)),
        (C_DECAY, 3.0),
    ],
);
const TIMPANI: Sound = with(
    INIT,
    &[
        (T_WAVE, B4),
        (PITCH, 40.0),
        (T_DECAY, 42.0),
        (BEND, 1.0),
        (BEND_TIME, 3.0),
        (T_FILTER, 70.0),
        (T_PUNCH, 1.0),
        (NOISE, 2.0),
        (N_COLOR, -6.0),
        (N_FILTER, 40.0),
        (N_DECAY, 28.0),
        (CLICK, 2.0),
        (C_TYPE, n(2)),
        (C_DECAY, 4.0),
    ],
);
const BELL: Sound = with(
    INIT,
    &[
        (T_WAVE, C1),
        (PITCH, 72.0),
        (T_DECAY, 46.0),
        (T_FILTER, 92.0),
        (CLICK, 2.0),
        (C_TYPE, t(9)),
        (C_DECAY, 2.0),
        (LEVEL, 70.0),
    ],
);
const GONG: Sound = with(
    INIT,
    &[
        (T_WAVE, C4),
        (PITCH, 38.0),
        (T_DECAY, 48.0),
        (T_GATE, S),
        (T_FILTER, 72.0),
        (T_SWEEP, 3.0),
        (T_TIME, 7.0),
        (NOISE, 2.0),
        (N_COLOR, -2.0),
        (N_FILTER, 60.0),
        (N_DECAY, 40.0),
        (LEVEL, 75.0),
    ],
);
const GLOCK: Sound = with(
    INIT,
    &[
        (T_WAVE, C5),
        (PITCH, 84.0),
        (T_DECAY, 38.0),
        (T_PUNCH, 1.0),
        (CLICK, 3.0),
        (C_TYPE, t(9)),
        (C_DECAY, 1.0),
        (LEVEL, 66.0),
    ],
);
const MARIMBA: Sound = with(
    INIT,
    &[
        (T_WAVE, C5),
        (PITCH, 60.0),
        (T_DECAY, 30.0),
        (T_FILTER, 80.0),
        (T_PUNCH, 2.0),
        (CLICK, 2.0),
        (C_TYPE, n(4)),
        (C_DECAY, 2.0),
    ],
);
const STEEL: Sound = with(
    INIT,
    &[
        (T_WAVE, B8),
        (PITCH, 64.0),
        (T_DECAY, 34.0),
        (T_FILTER, 82.0),
        (T_PUNCH, 2.0),
        (T_SWEEP, 2.0),
        (T_TIME, 3.0),
        (CLICK, 2.0),
        (C_TYPE, t(7)),
        (C_DECAY, 1.0),
        (LEVEL, 72.0),
    ],
);
const LOG: Sound = with(
    INIT,
    &[
        (T_WAVE, B9),
        (PITCH, 55.0),
        (T_DECAY, 24.0),
        (T_FILTER, 85.0),
        (T_PUNCH, 3.0),
        (CLICK, 3.0),
        (C_TYPE, c(4)),
        (C_DECAY, 2.0),
    ],
);
const CLAVES: Sound = with(
    INIT,
    &[
        (PITCH, 98.0),
        (T_DECAY, 12.0),
        (CLICK, 4.0),
        (C_TYPE, c(8)),
        (C_DECAY, 1.0),
        (LEVEL, 66.0),
    ],
);
/// Sounds for the tone's ADSRs.
const SWELL: Sound = with(
    INIT,
    &[
        (T_WAVE, AN3),
        (PITCH, 48.0),
        (AMP_ENV, 1.0),
        (AMP_A, 32.0),
        (AMP_D, 30.0),
        (AMP_S, 70.0),
        (AMP_R, 34.0),
        (T_FILTER, 40.0),
        (FLT_AMT, 6.0),
        (FLT_A, 30.0),
        (FLT_D, 34.0),
        (FLT_S, 30.0),
        (FLT_R, 30.0),
        (LEVEL, 75.0),
    ],
);
const ACID: Sound = with(
    INIT,
    &[
        (T_WAVE, AN3),
        (PITCH, 36.0),
        (AMP_ENV, 1.0),
        (AMP_D, 22.0),
        (AMP_S, 50.0),
        (AMP_R, 12.0),
        (T_FILTER, 28.0),
        (T_PUNCH, 3.0),
        (FLT_AMT, 8.0),
        (FLT_D, 20.0),
        (FLT_S, 10.0),
        (FLT_R, 12.0),
    ],
);
const ZAP: Sound = with(
    INIT,
    &[
        (PITCH, 52.0),
        (T_DECAY, 30.0),
        (BEND, 30.0),
        (BEND_TIME, 8.0),
    ],
);
const SWEEP: Sound = with(
    INIT,
    &[
        (TONE, 0.0),
        (NOISE, 10.0),
        (N_FILTER, 25.0),
        (N_TYPE, 7.0),
        (N_SWEEP, 10.0),
        (N_TIME, 7.0),
        (N_DECAY, 36.0),
        (LEVEL, 70.0),
    ],
);

const fn program(
    name: &'static str,
    style: &'static str,
    category: &'static str,
    channels: [Sound; 4],
) -> Program {
    Program {
        name,
        style,
        category,
        channels,
    }
}

const fn pitched(s: Sound, pitch: f32) -> Sound {
    with(s, &[(PITCH, pitch)])
}

/// The factory programs, for the code that runs.
pub static FACTORY: [Program; 34] = BANK;

/// The same for what is worked out while compiling (the knobs' defaults,
/// the packed bank): a static can't be read there.
#[allow(clippy::large_const_arrays)]
const BANK: [Program; 34] = [
    program(
        "First Light",
        "Retro",
        "Kit",
        [KICK_TIGHT, SNARE_TIGHT, pitched(TOM, 50.0), HAT_CLOSED],
    ),
    program(
        "Maple Studio",
        "Real",
        "Drums",
        [
            KICK_REAL,
            SNARE_REAL,
            pitched(TOM, 52.0),
            with(TOM, &[(PITCH, 45.0), (T_DECAY, 33.0)]),
        ],
    ),
    program(
        "Sub Boom",
        "Retro",
        "Kit",
        [
            KICK_BOOM,
            SNARE_BOX,
            with(
                INIT,
                &[
                    (PITCH, 52.0),
                    (T_DECAY, 30.0),
                    (BEND, 3.0),
                    (BEND_TIME, 4.0),
                ],
            ),
            HAT_METAL,
        ],
    ),
    program(
        "Brush Room",
        "Real",
        "Drums",
        [
            with(KICK_REAL, &[(LEVEL, 80.0), (T_FILTER, 55.0), (CLICK, 3.0)]),
            SNARE_BRUSH,
            with(TOM, &[(PITCH, 53.0), (T_FILTER, 60.0), (NOISE, 4.0)]),
            with(TOM, &[(PITCH, 46.0), (T_FILTER, 60.0), (NOISE, 4.0)]),
        ],
    ),
    program(
        "Hex Pads",
        "Retro",
        "Drums",
        [
            KICK_FM,
            with(SNARE_TIGHT, &[(T_WAVE, B7), (BEND, 9.0)]),
            pitched(TOM_HEX, 50.0),
            pitched(TOM_HEX, 43.0),
        ],
    ),
    program(
        "Dusty Crates",
        "Retro",
        "Kit",
        [
            with(
                INIT,
                &[
                    (T_WAVE, B1),
                    (PITCH, 30.0),
                    (T_DECAY, 34.0),
                    (T_FILTER, 55.0),
                    (T_PUNCH, 6.0),
                    (BEND, 6.0),
                    (BEND_TIME, 3.0),
                    (LEVEL, 90.0),
                ],
            ),
            with(
                SNARE_TIGHT,
                &[
                    (T_FILTER, 70.0),
                    (T_PUNCH, 7.0),
                    (N_COLOR, -1.0),
                    (N_FILTER, 70.0),
                    (N_TYPE, 3.0),
                ],
            ),
            RIM,
            with(HAT_CLOSED, &[(N_FILTER, 85.0), (T_FILTER, 80.0)]),
        ],
    ),
    program(
        "Kettle Hall",
        "Real",
        "Perc",
        [
            pitched(TIMPANI, 38.0),
            pitched(TIMPANI, 45.0),
            pitched(TIMPANI, 41.0),
            pitched(TIMPANI, 48.0),
        ],
    ),
    program(
        "Tabla Night",
        "Ethno",
        "Perc",
        [
            TABLA,
            BAYAN,
            with(
                TABLA,
                &[
                    (PITCH, 67.0),
                    (T_DECAY, 14.0),
                    (T_PUNCH, 5.0),
                    (C_TYPE, c(8)),
                ],
            ),
            with(
                BAYAN,
                &[
                    (PITCH, 43.0),
                    (T_DECAY, 14.0),
                    (BEND, 0.0),
                    (NOISE, 3.0),
                    (N_COLOR, -4.0),
                    (N_FILTER, 50.0),
                    (N_DECAY, 10.0),
                ],
            ),
        ],
    ),
    program(
        "Conga Line",
        "Ethno",
        "Perc",
        [
            CONGA,
            SLAP,
            with(CONGA, &[(PITCH, 53.0), (T_DECAY, 28.0)]),
            with(
                CONGA,
                &[
                    (PITCH, 69.0),
                    (T_DECAY, 18.0),
                    (BEND_TIME, 2.0),
                    (T_PUNCH, 3.0),
                    (C_TYPE, n(6)),
                ],
            ),
        ],
    ),
    program(
        "Gated Eighties",
        "Retro",
        "Drums",
        [
            with(KICK_TIGHT, &[(T_DECAY, 30.0), (T_GATE, G)]),
            SNARE_GATED,
            with(
                TOM,
                &[
                    (PITCH, 50.0),
                    (T_DECAY, 30.0),
                    (T_GATE, G),
                    (NOISE, 4.0),
                    (N_GATE, G),
                    (N_DECAY, 26.0),
                ],
            ),
            with(
                TOM,
                &[
                    (PITCH, 44.0),
                    (T_DECAY, 31.0),
                    (T_GATE, G),
                    (NOISE, 4.0),
                    (N_GATE, G),
                    (N_DECAY, 27.0),
                ],
            ),
        ],
    ),
    program(
        "Steel Garden",
        "Ethno",
        "Perc",
        [
            pitched(STEEL, 62.0),
            pitched(STEEL, 65.0),
            pitched(STEEL, 69.0),
            pitched(STEEL, 72.0),
        ],
    ),
    program(
        "Temple Bells",
        "Ethno",
        "Perc",
        [
            pitched(BELL, 60.0),
            pitched(BELL, 67.0),
            with(BELL, &[(PITCH, 72.0), (T_DECAY, 44.0)]),
            with(BELL, &[(PITCH, 79.0), (T_DECAY, 42.0)]),
        ],
    ),
    program(
        "Reso Sweeps",
        "FX",
        "Perc",
        [
            SWEEP,
            with(
                SWEEP,
                &[
                    (N_FILTER, 35.0),
                    (N_TYPE, 6.0),
                    (N_TIME, 5.0),
                    (N_DECAY, 32.0),
                ],
            ),
            with(
                SWEEP,
                &[
                    (N_FILTER, 45.0),
                    (N_COLOR, 4.0),
                    (N_TIME, 3.0),
                    (N_DECAY, 28.0),
                ],
            ),
            with(
                SWEEP,
                &[
                    (N_FILTER, 15.0),
                    (N_COLOR, -4.0),
                    (N_TIME, 9.0),
                    (N_DECAY, 40.0),
                    (N_GATE, S),
                ],
            ),
        ],
    ),
    program(
        "Laser Duel",
        "FX",
        "Kit",
        [
            with(ZAP, &[(PITCH, 34.0), (BEND_TIME, 4.0), (LEVEL, 88.0)]),
            with(
                ZAP,
                &[
                    (PITCH, 58.0),
                    (BEND, 20.0),
                    (BEND_TIME, 3.0),
                    (T_DECAY, 22.0),
                    (NOISE, 7.0),
                    (N_COLOR, 3.0),
                    (N_FILTER, 82.0),
                    (N_DECAY, 24.0),
                ],
            ),
            with(ZAP, &[(PITCH, 55.0), (BEND, -9.0), (BEND_TIME, 6.0)]),
            HAT_METAL,
        ],
    ),
    program(
        "Wood Shop",
        "Ethno",
        "Perc",
        [
            pitched(LOG, 52.0),
            pitched(LOG, 59.0),
            CLAVES,
            with(LOG, &[(PITCH, 79.0), (T_DECAY, 12.0), (C_TYPE, c(7))]),
        ],
    ),
    program(
        "Syndromes",
        "Retro",
        "Drums",
        [
            with(KICK_TIGHT, &[(BEND, 12.0), (BEND_TIME, 4.0)]),
            with(
                SNARE_BOX,
                &[
                    (T_WAVE, AN3),
                    (T_FILTER, 60.0),
                    (T_SWEEP, 5.0),
                    (T_TIME, 3.0),
                ],
            ),
            pitched(TOM_SAW, 52.0),
            pitched(TOM_SAW, 45.0),
        ],
    ),
    program(
        "Garage",
        "Real",
        "Kit",
        [
            KICK_REAL,
            with(SNARE_REAL, &[(PITCH, 58.0), (T_DECAY, 26.0)]),
            pitched(TOM, 49.0),
            with(HAT_CLOSED, &[(T_FILTER, 85.0)]),
        ],
    ),
    program(
        "Clap Trap",
        "Retro",
        "Kit",
        [with(KICK_BOOM, &[(T_DECAY, 33.0)]), CLAP, RIM, HAT_CLOSED],
    ),
    program(
        "Glockenspiel",
        "Real",
        "Perc",
        [
            pitched(GLOCK, 84.0),
            pitched(GLOCK, 88.0),
            pitched(GLOCK, 91.0),
            pitched(GLOCK, 96.0),
        ],
    ),
    program(
        "Gong Bath",
        "FX",
        "Perc",
        [
            pitched(GONG, 36.0),
            with(GONG, &[(PITCH, 40.0), (T_DECAY, 47.0)]),
            with(GONG, &[(PITCH, 43.0), (T_DECAY, 49.0)]),
            with(GONG, &[(PITCH, 47.0), (T_DECAY, 50.0)]),
        ],
    ),
    program(
        "Djembe Circle",
        "Ethno",
        "Perc",
        [
            with(
                CONGA,
                &[
                    (PITCH, 45.0),
                    (T_DECAY, 26.0),
                    (BEND, 3.0),
                    (T_FILTER, 60.0),
                ],
            ),
            with(CONGA, &[(PITCH, 60.0), (T_DECAY, 20.0)]),
            with(SLAP, &[(PITCH, 66.0), (NOISE, 6.0)]),
            with(TOM, &[(PITCH, 40.0), (T_DECAY, 30.0), (T_FILTER, 65.0)]),
        ],
    ),
    program(
        "Cowbell Fever",
        "Retro",
        "Perc",
        [
            COWBELL,
            pitched(COWBELL, 68.0),
            with(COWBELL, &[(PITCH, 81.0), (T_DECAY, 22.0)]),
            with(COWBELL, &[(PITCH, 76.0), (T_DECAY, 22.0)]),
        ],
    ),
    program(
        "Dub Plate",
        "Retro",
        "Kit",
        [
            with(KICK_BOOM, &[(PITCH, 28.0), (T_DECAY, 39.0)]),
            with(
                SNARE_TIGHT,
                &[(T_FILTER, 65.0), (N_DECAY, 35.0), (N_GATE, S)],
            ),
            with(TOM, &[(PITCH, 43.0), (T_DECAY, 36.0)]),
            with(HAT_OPEN, &[(N_DECAY, 28.0), (T_DECAY, 26.0)]),
        ],
    ),
    program(
        "Iron Works",
        "FX",
        "Drums",
        [
            with(
                KICK_REAL,
                &[
                    (NOISE, 8.0),
                    (N_COLOR, -8.0),
                    (N_FILTER, 50.0),
                    (N_TYPE, 4.0),
                    (N_DECAY, 26.0),
                ],
            ),
            with(SNARE_TIGHT, &[(T_WAVE, C3), (PITCH, 70.0), (T_DECAY, 22.0)]),
            with(
                INIT,
                &[
                    (T_WAVE, C4),
                    (PITCH, 50.0),
                    (T_DECAY, 32.0),
                    (T_FILTER, 80.0),
                    (T_PUNCH, 4.0),
                ],
            ),
            with(
                INIT,
                &[
                    (T_WAVE, C4),
                    (PITCH, 57.0),
                    (T_DECAY, 26.0),
                    (T_FILTER, 85.0),
                    (T_PUNCH, 4.0),
                ],
            ),
        ],
    ),
    program(
        "Jazz Club",
        "Real",
        "Drums",
        [
            with(
                KICK_REAL,
                &[
                    (PITCH, 38.0),
                    (T_DECAY, 30.0),
                    (BEND, 3.0),
                    (T_FILTER, 60.0),
                ],
            ),
            with(
                SNARE_REAL,
                &[
                    (PITCH, 60.0),
                    (T_DECAY, 27.0),
                    (N_DECAY, 29.0),
                    (N_SWEEP, 3.0),
                ],
            ),
            pitched(TOM, 55.0),
            pitched(TOM, 50.0),
        ],
    ),
    program(
        "Taiko",
        "Ethno",
        "Drums",
        [
            with(
                TOM,
                &[
                    (PITCH, 35.0),
                    (T_DECAY, 40.0),
                    (BEND, 5.0),
                    (BEND_TIME, 4.0),
                    (T_FILTER, 55.0),
                    (T_PUNCH, 6.0),
                    (NOISE, 4.0),
                    (N_COLOR, -6.0),
                    (N_FILTER, 45.0),
                    (N_DECAY, 30.0),
                ],
            ),
            with(
                INIT,
                &[
                    (T_WAVE, B3),
                    (PITCH, 64.0),
                    (T_DECAY, 16.0),
                    (T_PUNCH, 6.0),
                    (CLICK, 5.0),
                    (C_TYPE, c(7)),
                    (C_DECAY, 2.0),
                ],
            ),
            with(TOM, &[(PITCH, 43.0), (T_DECAY, 36.0), (BEND, 4.0)]),
            with(
                BELL,
                &[(PITCH, 79.0), (T_DECAY, 28.0), (C_TYPE, c(9)), (CLICK, 4.0)],
            ),
        ],
    ),
    program(
        "Micro Ticks",
        "Retro",
        "Kit",
        [
            with(
                INIT,
                &[
                    (PITCH, 36.0),
                    (T_DECAY, 22.0),
                    (BEND, 20.0),
                    (BEND_TIME, 1.0),
                    (CLICK, 4.0),
                    (C_TYPE, t(5)),
                    (C_DECAY, 1.0),
                ],
            ),
            with(
                INIT,
                &[
                    (TONE, 2.0),
                    (NOISE, 6.0),
                    (N_COLOR, 7.0),
                    (N_FILTER, 92.0),
                    (N_DECAY, 16.0),
                    (CLICK, 6.0),
                    (C_TYPE, c(8)),
                    (C_DECAY, 3.0),
                ],
            ),
            with(
                INIT,
                &[
                    (PITCH, 64.0),
                    (T_DECAY, 15.0),
                    (BEND, 8.0),
                    (BEND_TIME, 2.0),
                ],
            ),
            with(
                INIT,
                &[
                    (TONE, 0.0),
                    (NOISE, 7.0),
                    (N_COLOR, 9.0),
                    (N_FILTER, 97.0),
                    (N_DECAY, 10.0),
                    (CLICK, 5.0),
                    (C_TYPE, n(9)),
                    (C_DECAY, 2.0),
                    (LEVEL, 86.0),
                ],
            ),
        ],
    ),
    program(
        "Marimba Box",
        "Real",
        "Perc",
        [
            pitched(MARIMBA, 48.0),
            pitched(MARIMBA, 55.0),
            pitched(MARIMBA, 60.0),
            pitched(MARIMBA, 64.0),
        ],
    ),
    program(
        "Pitch Drop",
        "FX",
        "Perc",
        [
            with(ZAP, &[(PITCH, 40.0), (BEND_TIME, 9.0), (T_DECAY, 38.0)]),
            with(
                ZAP,
                &[(T_WAVE, B7), (PITCH, 50.0), (BEND, 24.0), (BEND_TIME, 7.0)],
            ),
            with(
                ZAP,
                &[(T_WAVE, AN2), (PITCH, 60.0), (BEND, -9.0), (BEND_TIME, 8.0)],
            ),
            with(SWEEP, &[(N_FILTER, 20.0), (N_TYPE, 5.0), (N_TIME, 8.0)]),
        ],
    ),
    program(
        "Noise Orchestra",
        "FX",
        "Kit",
        [
            with(
                INIT,
                &[
                    (T_WAVE, B1),
                    (PITCH, 30.0),
                    (TONE, 3.0),
                    (NOISE, 10.0),
                    (N_COLOR, -10.0),
                    (N_FILTER, 40.0),
                    (N_TYPE, 5.0),
                    (N_SWEEP, 8.0),
                    (N_TIME, 2.0),
                    (N_DECAY, 28.0),
                    (LEVEL, 88.0),
                ],
            ),
            with(
                INIT,
                &[
                    (TONE, 0.0),
                    (NOISE, 10.0),
                    (N_FILTER, 80.0),
                    (N_TYPE, 4.0),
                    (N_SWEEP, 6.0),
                    (N_TIME, 3.0),
                    (N_DECAY, 28.0),
                ],
            ),
            with(
                INIT,
                &[
                    (TONE, 0.0),
                    (NOISE, 10.0),
                    (N_COLOR, -6.0),
                    (N_FILTER, 45.0),
                    (N_TYPE, 6.0),
                    (N_SWEEP, 7.0),
                    (N_TIME, 4.0),
                    (N_DECAY, 30.0),
                ],
            ),
            with(
                INIT,
                &[
                    (TONE, 0.0),
                    (NOISE, 10.0),
                    (N_COLOR, 9.0),
                    (N_FILTER, 99.0),
                    (N_DECAY, 13.0),
                    (LEVEL, 84.0),
                ],
            ),
        ],
    ),
    program(
        "Frame Drums",
        "Ethno",
        "Perc",
        [
            with(
                INIT,
                &[
                    (T_WAVE, B3),
                    (PITCH, 43.0),
                    (T_DECAY, 28.0),
                    (T_FILTER, 50.0),
                    (NOISE, 4.0),
                    (N_COLOR, -4.0),
                    (N_FILTER, 60.0),
                    (N_DECAY, 24.0),
                ],
            ),
            with(
                INIT,
                &[
                    (T_WAVE, B3),
                    (PITCH, 52.0),
                    (T_DECAY, 22.0),
                    (T_FILTER, 60.0),
                    (NOISE, 3.0),
                    (N_COLOR, -2.0),
                    (N_FILTER, 66.0),
                    (N_DECAY, 20.0),
                ],
            ),
            with(
                INIT,
                &[
                    (T_WAVE, C3),
                    (PITCH, 94.0),
                    (T_DECAY, 24.0),
                    (TONE, 4.0),
                    (NOISE, 8.0),
                    (N_COLOR, 8.0),
                    (N_FILTER, 96.0),
                    (N_TYPE, 2.0),
                    (N_DECAY, 26.0),
                    (N_GATE, S),
                    (LEVEL, 66.0),
                ],
            ),
            with(
                INIT,
                &[
                    (T_WAVE, B3),
                    (PITCH, 38.0),
                    (T_DECAY, 32.0),
                    (T_FILTER, 45.0),
                    (T_PUNCH, 3.0),
                ],
            ),
        ],
    ),
    program(
        "Electro Funk",
        "Retro",
        "Kit",
        [
            KICK_FM,
            with(
                INIT,
                &[
                    (T_WAVE, AN2),
                    (PITCH, 58.0),
                    (BEND, 8.0),
                    (BEND_TIME, 2.0),
                    (T_DECAY, 20.0),
                    (TONE, 6.0),
                    (NOISE, 8.0),
                    (N_COLOR, 4.0),
                    (N_FILTER, 86.0),
                    (N_DECAY, 26.0),
                ],
            ),
            with(ZAP, &[(PITCH, 50.0), (BEND, 16.0), (BEND_TIME, 4.0)]),
            HAT_METAL,
        ],
    ),
    program(
        "Slow Bloom",
        "FX",
        "Perc",
        [
            SWELL,
            with(
                SWELL,
                &[
                    (T_WAVE, AN2),
                    (PITCH, 55.0),
                    (AMP_A, 26.0),
                    (AMP_D, 28.0),
                    (AMP_S, 60.0),
                    (AMP_R, 32.0),
                    (T_FILTER, 99.0),
                    (FLT_AMT, 0.0),
                ],
            ),
            with(
                BELL,
                &[
                    (PITCH, 67.0),
                    (AMP_ENV, 1.0),
                    (AMP_A, 36.0),
                    (AMP_D, 40.0),
                    (AMP_S, 0.0),
                    (AMP_R, 36.0),
                    (CLICK, 0.0),
                ],
            ),
            with(
                STEEL,
                &[
                    (PITCH, 62.0),
                    (AMP_ENV, 1.0),
                    (AMP_A, 20.0),
                    (AMP_D, 32.0),
                    (AMP_S, 40.0),
                    (AMP_R, 30.0),
                    (T_FILTER, 90.0),
                    (FLT_AMT, -4.0),
                    (FLT_D, 30.0),
                    (FLT_S, 0.0),
                ],
            ),
        ],
    ),
    program(
        "Acid Kit",
        "Retro",
        "Kit",
        [KICK_TIGHT, SNARE_TIGHT, ACID, HAT_CLOSED],
    ),
];

/// Program `k`'s factory sounds (an empty location: Sound Init).
pub const fn factory(k: usize) -> [Sound; 4] {
    if k < BANK.len() {
        BANK[k].channels
    } else {
        [INIT; 4]
    }
}
