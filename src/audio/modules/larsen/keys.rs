//! The keys on the MIDI input: which note each string plays and whether
//! its key is down.
//!
//! With one string it plays the latest key held (or, once they are all
//! up, the latest played). With three, each new note takes a string that
//! has never played, else the one let go longest ago, else the one held
//! longest; a string keeps its note after its key is up, and takes a held
//! note that has none when its own key is let go.

use crate::audio::modules::steps::{Held, note_event};

pub const STRINGS: usize = 3;

pub struct Keys {
    held: Held,
    /// Pitch bend, in semitones (±2).
    pub bend: f32,
    /// Each string's note (kept once let go), and whether its key is down.
    note: [Option<u8>; STRINGS],
    down: [bool; STRINGS],
    /// When each string took its note, and when it was let go.
    taken: [u64; STRINGS],
    released: [u64; STRINGS],
    clock: u64,
    last: Option<u8>,
    /// Keys have set the pitch since the FREQ knob last had it.
    pub played: bool,
    poly: bool,
}

impl Keys {
    pub fn new() -> Self {
        Self {
            held: Held::new(),
            bend: 0.0,
            note: [None; STRINGS],
            down: [false; STRINGS],
            taken: [0; STRINGS],
            released: [0; STRINGS],
            clock: 0,
            last: None,
            played: false,
            poly: false,
        }
    }

    pub fn any_held(&self) -> bool {
        !self.held.is_empty()
    }

    /// The one string's note: the latest key held, else the latest played.
    pub fn latest(&self) -> Option<u8> {
        self.held.notes().last().copied().or(self.last)
    }

    /// String `s`'s note, and whether its key is down (three strings).
    pub fn string(&self, s: usize) -> (Option<u8>, bool) {
        (self.note[s], self.down[s])
    }

    /// Three strings or one. Going to three with keys down gives the
    /// latest three their strings.
    pub fn set_poly(&mut self, poly: bool) {
        if poly == self.poly {
            return;
        }
        self.poly = poly;
        self.note = [None; STRINGS];
        self.down = [false; STRINGS];
        if !poly {
            return;
        }
        let held = self.held.notes();
        let latest = &held[held.len().saturating_sub(STRINGS)..];
        for (s, &n) in latest.iter().enumerate() {
            self.clock += 1;
            (self.note[s], self.down[s], self.taken[s]) = (Some(n), true, self.clock);
        }
        if latest.is_empty() {
            self.note[0] = self.last;
        }
    }

    pub fn handle(&mut self, msg: [u8; 3]) {
        self.clock += 1;
        match note_event(msg) {
            Some(true) => self.press(msg[1]),
            Some(false) => self.release(msg[1]),
            None if msg[0] & 0xF0 == 0xE0 => {
                let value = ((msg[2] as i32) << 7 | msg[1] as i32) - 8192;
                self.bend = value as f32 / 8192.0 * 2.0;
            }
            // All notes off, all sound off.
            None if msg[0] & 0xF0 == 0xB0 && matches!(msg[1], 120 | 123) => {
                self.held.clear();
                for s in 0..STRINGS {
                    if self.down[s] {
                        (self.down[s], self.released[s]) = (false, self.clock);
                    }
                }
            }
            None => {}
        }
    }

    fn press(&mut self, note: u8) {
        self.held.press(note);
        self.last = Some(note);
        self.played = true;
        if !self.poly || (0..STRINGS).any(|s| self.down[s] && self.note[s] == Some(note)) {
            return;
        }
        let fresh = (0..STRINGS).find(|&s| self.note[s].is_none());
        let let_go = (0..STRINGS)
            .filter(|&s| !self.down[s])
            .min_by_key(|&s| self.released[s]);
        let longest = (0..STRINGS).min_by_key(|&s| self.taken[s]);
        if let Some(s) = fresh.or(let_go).or(longest) {
            (self.note[s], self.down[s], self.taken[s]) = (Some(note), true, self.clock);
        }
    }

    fn release(&mut self, note: u8) {
        self.held.release(note);
        if !self.poly {
            return;
        }
        for s in 0..STRINGS {
            if !(self.down[s] && self.note[s] == Some(note)) {
                continue;
            }
            (self.down[s], self.released[s]) = (false, self.clock);
            // A held note with no string takes this one, the latest first.
            let waiting = self
                .held
                .notes()
                .iter()
                .rev()
                .find(|&&n| !(0..STRINGS).any(|t| self.down[t] && self.note[t] == Some(n)));
            if let Some(&n) = waiting {
                (self.note[s], self.down[s], self.taken[s]) = (Some(n), true, self.clock);
            }
        }
    }
}
