//! Parameters: how a knob is described ([`ParamSpec`]: range, taper, unit),
//! how its value reads ([`ParamSpec::format`]), and the lock-free
//! [`AtomicF32`] the UI thread and the audio thread share it through.
//!
//! The UI writes parameter values into atomics; the audio thread reads them
//! once per block. Meters flow the other way: the audio thread accumulates
//! peaks and the UI drains them every frame.

use std::sync::atomic::{AtomicU32, Ordering};

/// An `f32` stored in an `AtomicU32`. Relaxed ordering is enough: every
/// value is independent and a one-block delay is inaudible.
///
/// `repr(transparent)`: it has the layout of a `u32`, so it can be shared
/// with a node plugin through a pointer (see [`crate::abi::RawParams`]).
#[derive(Debug)]
#[repr(transparent)]
pub struct AtomicF32(AtomicU32);

impl AtomicF32 {
    pub const fn new(v: f32) -> Self {
        Self(AtomicU32::new(v.to_bits()))
    }

    #[inline]
    pub fn get(&self) -> f32 {
        f32::from_bits(self.0.load(Ordering::Relaxed))
    }

    #[inline]
    pub fn set(&self, v: f32) {
        self.0.store(v.to_bits(), Ordering::Relaxed);
    }

    /// Stores the max of the current and given value. For non-negative
    /// floats the bit pattern is monotonic, so an integer max is correct.
    #[inline]
    pub fn fetch_max(&self, v: f32) {
        self.0.fetch_max(v.max(0.0).to_bits(), Ordering::Relaxed);
    }

    /// Reads the value and resets it to zero (used to drain peak meters).
    #[inline]
    pub fn take(&self) -> f32 {
        f32::from_bits(self.0.swap(0, Ordering::Relaxed))
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Taper {
    Linear,
    Log,
}

impl Taper {
    /// Stable number used by the plugin ABI.
    pub const fn code(self) -> u32 {
        match self {
            Taper::Linear => 0,
            Taper::Log => 1,
        }
    }

    pub const fn from_code(code: u32) -> Option<Self> {
        match code {
            0 => Some(Taper::Linear),
            1 => Some(Taper::Log),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Unit {
    Hz,
    Cents,
    Percent,
    Decibel,
    Seconds,
    Semitones,
    Octaves,
    PerSecond,
    Integer,
    /// MIDI channel; 0 is "omni".
    Channel,
    /// Index into [`DIVISIONS`].
    Division,
    /// Pitch class, 0 = C.
    NoteName,
    /// MIDI note number ("C4" = 60).
    Note,
    /// Index into [`NOTE_VALUES`].
    NoteValue,
    /// Index into [`SYNC_DIVISIONS`].
    Sync,
    /// A level or gain already in decibels.
    Db,
    /// A filter's Q.
    Q,
    /// A plain number, two decimals.
    Plain,
}

impl Unit {
    /// Every unit, in the order of their ABI codes.
    pub const ALL: [Unit; 18] = [
        Unit::Hz,
        Unit::Cents,
        Unit::Percent,
        Unit::Decibel,
        Unit::Seconds,
        Unit::Semitones,
        Unit::Octaves,
        Unit::PerSecond,
        Unit::Integer,
        Unit::Channel,
        Unit::Division,
        Unit::NoteName,
        Unit::Note,
        Unit::NoteValue,
        Unit::Sync,
        Unit::Db,
        Unit::Q,
        Unit::Plain,
    ];

    /// Stable number used by the plugin ABI. New units get new codes at
    /// the end; codes are never reused.
    pub const fn code(self) -> u32 {
        match self {
            Unit::Hz => 0,
            Unit::Cents => 1,
            Unit::Percent => 2,
            Unit::Decibel => 3,
            Unit::Seconds => 4,
            Unit::Semitones => 5,
            Unit::Octaves => 6,
            Unit::PerSecond => 7,
            Unit::Integer => 8,
            Unit::Channel => 9,
            Unit::Division => 10,
            Unit::NoteName => 11,
            Unit::Note => 12,
            Unit::NoteValue => 13,
            Unit::Sync => 14,
            Unit::Db => 15,
            Unit::Q => 16,
            Unit::Plain => 17,
        }
    }

    /// The unit for an ABI code; a code from a newer SDK reads as
    /// [`Unit::Plain`].
    pub fn from_code(code: u32) -> Self {
        Self::ALL.get(code as usize).copied().unwrap_or(Unit::Plain)
    }
}

/// Note values a tempo-synced time can take, shortest first: (label,
/// beats). "D" is dotted, "T" triplet.
pub const SYNC_DIVISIONS: [(&str, f64); 14] = [
    ("1/32", 0.125),
    ("1/16T", 1.0 / 6.0),
    ("1/16", 0.25),
    ("1/8T", 1.0 / 3.0),
    ("1/16D", 0.375),
    ("1/8", 0.5),
    ("1/4T", 2.0 / 3.0),
    ("1/8D", 0.75),
    ("1/4", 1.0),
    ("1/2T", 4.0 / 3.0),
    ("1/4D", 1.5),
    ("1/2", 2.0),
    ("1/2D", 3.0),
    ("1/1", 4.0),
];

/// The note value a DIV knob points at.
pub fn sync_division(v: f32) -> (&'static str, f64) {
    SYNC_DIVISIONS[(v.round().max(0.0) as usize).min(SYNC_DIVISIONS.len() - 1)]
}

/// Note values for rhythm generators, longest first: (label, beats,
/// triplet, thirty-second).
pub const NOTE_VALUES: [(&str, f64, bool, bool); 9] = [
    ("1/1", 4.0, false, false),
    ("1/2", 2.0, false, false),
    ("1/4", 1.0, false, false),
    ("1/4T", 2.0 / 3.0, true, false),
    ("1/8", 0.5, false, false),
    ("1/8T", 1.0 / 3.0, true, false),
    ("1/16", 0.25, false, false),
    ("1/16T", 1.0 / 6.0, true, false),
    ("1/32", 0.125, false, true),
];

/// "C4" for MIDI note 60.
pub fn note_label(note: i32) -> String {
    format!("{}{}", NOTE_NAMES[note.rem_euclid(12) as usize], note.div_euclid(12) - 1)
}

/// Note lengths for sequencers: (label, steps per beat).
pub const DIVISIONS: [(&str, f64); 6] = [("1/4", 1.0), ("1/8", 2.0), ("1/8T", 3.0), ("1/16", 4.0), ("1/16T", 6.0), ("1/32", 8.0)];

pub const NOTE_NAMES: [&str; 12] = ["C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "B"];

/// Static description of a knob-controlled parameter.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ParamSpec {
    pub name: &'static str,
    pub min: f32,
    pub max: f32,
    pub default: f32,
    pub taper: Taper,
    pub unit: Unit,
    pub bipolar: bool,
}

impl ParamSpec {
    pub const fn lin(name: &'static str, min: f32, max: f32, default: f32, unit: Unit) -> Self {
        Self {
            name,
            min,
            max,
            default,
            taper: Taper::Linear,
            unit,
            bipolar: false,
        }
    }

    pub const fn log(name: &'static str, min: f32, max: f32, default: f32, unit: Unit) -> Self {
        Self {
            taper: Taper::Log,
            ..Self::lin(name, min, max, default, unit)
        }
    }

    /// Knob arc grows from the centre.
    pub const fn bipolar(self) -> Self {
        Self { bipolar: true, ..self }
    }

    pub fn to_norm(self, v: f32) -> f32 {
        let n = match self.taper {
            Taper::Linear => (v - self.min) / (self.max - self.min),
            Taper::Log => (v / self.min).ln() / (self.max / self.min).ln(),
        };
        n.clamp(0.0, 1.0)
    }

    pub fn denorm(&self, n: f32) -> f32 {
        let n = n.clamp(0.0, 1.0);
        match self.taper {
            Taper::Linear => self.min + n * (self.max - self.min),
            Taper::Log => self.min * (self.max / self.min).powf(n),
        }
    }

    /// Why this spec can't work, if it can't: an empty name, a range that
    /// is empty or not finite, a log taper reaching zero, a default out of
    /// range. Plugin descriptions are checked with it before use.
    pub fn problem(&self) -> Option<String> {
        let finite = self.min.is_finite() && self.max.is_finite() && self.default.is_finite();
        if self.name.trim().is_empty() {
            Some("a parameter has no name".to_owned())
        } else if !finite || self.min >= self.max {
            Some(format!("{}: the range {}…{} is empty or not finite", self.name, self.min, self.max))
        } else if self.taper == Taper::Log && self.min <= 0.0 {
            Some(format!("{}: a log taper needs a range above zero", self.name))
        } else if self.default < self.min || self.default > self.max {
            Some(format!("{}: the default {} is out of range", self.name, self.default))
        } else {
            None
        }
    }

    pub fn format(&self, v: f32) -> String {
        match (self.unit, self.bipolar) {
            (Unit::Hz, _) => format_hz(v),
            (Unit::Cents, true) => format!("{v:+.0} ct"),
            (Unit::Cents, false) => format!("{v:.0} ct"),
            (Unit::Percent, _) => format!("{:.0}%", v * 100.0),
            (Unit::Decibel, _) => format_db(gain_to_db(v)),
            (Unit::Seconds, _) => format_seconds(v),
            (Unit::Semitones, true) => format!("{:+.0} st", v.round()),
            (Unit::Semitones, false) => format!("±{v:.1} st"),
            (Unit::Octaves, true) => format!("{v:+.1} oct"),
            (Unit::Octaves, false) => format!("{v:.1} oct"),
            (Unit::PerSecond, _) if v < 10.0 => format!("{v:.1}/s"),
            (Unit::PerSecond, _) => format!("{v:.0}/s"),
            (Unit::Integer, _) => format!("{:.0}", v.round()),
            (Unit::Channel, _) if v.round() < 1.0 => "Omni".to_owned(),
            (Unit::Channel, _) => format!("Ch {:.0}", v.round()),
            (Unit::Division, _) => DIVISIONS[(v.round() as usize).min(DIVISIONS.len() - 1)].0.to_owned(),
            (Unit::NoteName, _) => NOTE_NAMES[(v.round() as usize) % 12].to_owned(),
            (Unit::Note, _) => note_label(v.round() as i32),
            (Unit::Sync, _) => sync_division(v).0.to_owned(),
            (Unit::Db, true) => format!("{v:+.1} dB"),
            (Unit::Db, false) => format!("{v:.1} dB"),
            (Unit::Q, _) => format!("{v:.2}"),
            (Unit::Plain, true) => format!("{v:+.2}"),
            (Unit::Plain, false) => format!("{v:.2}"),
            (Unit::NoteValue, _) => NOTE_VALUES[(v.round().max(0.0) as usize).min(NOTE_VALUES.len() - 1)].0.to_owned(),
        }
    }
}

pub fn format_hz(v: f32) -> String {
    if v >= 1000.0 {
        format!("{:.2} kHz", v / 1000.0)
    } else if v >= 100.0 {
        format!("{v:.0} Hz")
    } else if v >= 10.0 {
        format!("{v:.1} Hz")
    } else {
        format!("{v:.2} Hz")
    }
}

pub fn format_seconds(v: f32) -> String {
    if v < 1.0 {
        format!("{:.0} ms", v * 1000.0)
    } else if v < 10.0 {
        format!("{v:.2} s")
    } else {
        format!("{v:.1} s")
    }
}

pub fn format_db(db: f32) -> String {
    if db <= -90.0 {
        "-inf dB".to_owned()
    } else {
        format!("{db:+.1} dB")
    }
}

pub fn gain_to_db(g: f32) -> f32 {
    if g <= 1e-5 { -100.0 } else { 20.0 * g.log10() }
}

/// Fader position (0..1) that corresponds to unity gain.
pub const FADER_UNITY: f32 = 0.793_700_5; // cbrt(0.5)

/// Cubic fader law: 0 → -inf, unity at ~79% travel, +6 dB at the top.
#[inline]
pub fn fader_gain(pos: f32) -> f32 {
    let p = pos.clamp(0.0, 1.0);
    2.0 * p * p * p
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unit_codes_round_trip() {
        for (i, u) in Unit::ALL.iter().enumerate() {
            assert_eq!(u.code() as usize, i);
            assert_eq!(Unit::from_code(u.code()), *u);
        }
        assert_eq!(Unit::from_code(9999), Unit::Plain);
        assert_eq!(Taper::from_code(Taper::Log.code()), Some(Taper::Log));
        assert_eq!(Taper::from_code(7), None);
    }

    #[test]
    fn specs_are_checked() {
        assert!(ParamSpec::lin("MIX", 0.0, 1.0, 0.5, Unit::Percent).problem().is_none());
        assert!(ParamSpec::lin("", 0.0, 1.0, 0.5, Unit::Percent).problem().is_some());
        assert!(ParamSpec::lin("X", 1.0, 1.0, 1.0, Unit::Plain).problem().is_some());
        assert!(ParamSpec::log("F", 0.0, 10.0, 1.0, Unit::Hz).problem().is_some());
        assert!(ParamSpec::lin("X", 0.0, 1.0, 2.0, Unit::Plain).problem().is_some());
        assert!(ParamSpec::lin("X", 0.0, f32::NAN, 0.0, Unit::Plain).problem().is_some());
    }

    #[test]
    fn values_read_as_their_unit() {
        let spec = ParamSpec::log("FREQ", 20.0, 20_000.0, 440.0, Unit::Hz);
        assert_eq!(spec.format(440.0), "440 Hz");
        assert!((spec.denorm(spec.to_norm(1234.0)) - 1234.0).abs() < 0.1);
        assert_eq!(ParamSpec::lin("N", 0.0, 127.0, 60.0, Unit::Note).format(60.0), "C4");
        assert_eq!(ParamSpec::lin("X", -1.0, 1.0, 0.0, Unit::Plain).bipolar().format(0.25), "+0.25");
    }
}
