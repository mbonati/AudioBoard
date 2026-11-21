//! Sibyl's arpeggiator: the keys held (or latched) played one after another
//! on the board's clock — UP, DOWN, UP/DN (the ends once), UP+DN (the ends
//! twice), ORDER (as played), RANDOM, or CHORD (all of them on every step) —
//! over one to four octaves, at a note value from 1/4 to 1/32, each note
//! held for GATE of a step, every second step pushed late by SWING.
//!
//! While the transport plays it steps on the board's grid; stopped, it runs
//! from the first key at the board's tempo. LATCH keeps the chord after the
//! keys are let go; a new chord replaces it.

use super::Event;
use super::voice::Patch;
use audioboard_node::dsp::Rng;

const MAX_KEYS: usize = 32;
/// Notes in a pattern (keys × octaves, both ways).
const MAX_PATTERN: usize = 2 * MAX_KEYS * 4;

pub struct Arp {
    /// The chord, in press order, with velocities.
    keys: [(u8, f32); MAX_KEYS],
    len: usize,
    /// Keys physically down (with LATCH the chord outlives them).
    down: usize,
    /// The beat of the next step, once running.
    next: Option<f64>,
    step: u64,
    /// Notes sounding, and the beat they end.
    sounding: [u8; MAX_KEYS],
    sounding_len: usize,
    off_at: f64,
    rng: Rng,
    realign: bool,
}

impl Arp {
    pub fn new() -> Self {
        Self {
            keys: [(0, 0.0); MAX_KEYS],
            len: 0,
            down: 0,
            next: None,
            step: 0,
            sounding: [0; MAX_KEYS],
            sounding_len: 0,
            off_at: f64::INFINITY,
            rng: Rng::new(0xA77E_6610),
            realign: false,
        }
    }

    pub fn clear(&mut self) {
        self.len = 0;
        self.down = 0;
        self.next = None;
        self.sounding_len = 0;
        self.off_at = f64::INFINITY;
    }

    pub fn running(&self) -> bool {
        self.next.is_some() || self.sounding_len > 0
    }

    pub fn held_count(&self) -> usize {
        self.len
    }

    /// The note sounding now (the lowest, in CHORD).
    pub fn sounding(&self) -> Option<u8> {
        self.sounding[..self.sounding_len].iter().copied().min()
    }

    /// The transport started over: step on the grid again.
    pub fn restart(&mut self) {
        self.realign = true;
    }

    /// Lets go of a latched chord.
    pub fn unlatch(&mut self) {
        if self.down == 0 {
            self.len = 0;
        }
    }

    /// Stops: the notes that were sounding, to let go of.
    pub fn stop(&mut self) -> Option<u8> {
        self.next = None;
        self.len = 0;
        self.down = 0;
        let n = (self.sounding_len > 0).then(|| self.sounding[0]);
        if self.sounding_len > 0 {
            self.sounding.copy_within(1..self.sounding_len, 0);
            self.sounding_len -= 1;
        }
        n
    }

    pub fn press(&mut self, note: u8, vel: f32) {
        self.press_with(note, vel, false);
    }

    fn press_with(&mut self, note: u8, vel: f32, latch: bool) {
        // With LATCH, a key after all were let go starts a new chord.
        if latch && self.down == 0 {
            self.len = 0;
        }
        self.down += 1;
        if let Some(i) = self.keys[..self.len].iter().position(|k| k.0 == note) {
            self.keys[i].1 = vel;
            return;
        }
        if self.len < MAX_KEYS {
            self.keys[self.len] = (note, vel);
            self.len += 1;
        }
    }

    pub fn release(&mut self, note: u8) {
        self.release_with(note, false);
    }

    fn release_with(&mut self, note: u8, latch: bool) {
        self.down = self.down.saturating_sub(1);
        if latch {
            return;
        }
        if let Some(i) = self.keys[..self.len].iter().position(|k| k.0 == note) {
            self.keys.copy_within(i + 1..self.len, i);
            self.len -= 1;
        }
    }

    /// The pattern for the chord: notes and velocities.
    fn pattern(&self, patch: &Patch, out: &mut [(u8, f32); MAX_PATTERN]) -> usize {
        if self.len == 0 {
            return 0;
        }
        let mut sorted = [(0u8, 0.0f32); MAX_KEYS];
        sorted[..self.len].copy_from_slice(&self.keys[..self.len]);
        // ORDER keeps the order they were played in; the rest go low to high.
        if patch.arp_mode != 4 {
            sorted[..self.len].sort_by_key(|k| k.0);
        }
        let keys = &sorted[..self.len];
        let total = keys.len() * patch.arp_oct;
        // The i-th note going up through the octaves.
        let at = |i: usize| {
            let k = keys[i % keys.len()];
            (k.0 as i32 + 12 * (i / keys.len()) as i32, k.1)
        };
        let mut n = 0;
        let mut put = |(note, vel): (i32, f32)| {
            if n < MAX_PATTERN && (0..128).contains(&note) {
                out[n] = (note as u8, vel);
                n += 1;
            }
        };
        match patch.arp_mode {
            1 => (0..total).rev().for_each(|i| put(at(i))),
            // Up, then down without the ends (UP/DN) or with them (UP+DN).
            2 => {
                (0..total).for_each(|i| put(at(i)));
                (1..total.saturating_sub(1)).rev().for_each(|i| put(at(i)));
            }
            3 => {
                (0..total).for_each(|i| put(at(i)));
                (0..total).rev().for_each(|i| put(at(i)));
            }
            _ => (0..total).for_each(|i| put(at(i))),
        }
        n
    }

    /// Plans this block: takes the block's key events (MIDI, in frame order)
    /// and answers with the notes to play at their frames.
    #[allow(clippy::too_many_arguments)]
    pub fn plan(
        &mut self,
        patch: &Patch,
        beat0: f64,
        bps: f64,
        n: usize,
        events: &[Event],
        push: &mut impl FnMut(usize, [u8; 3]),
    ) {
        let rate = patch.arp_beats;
        let end = beat0 + bps * n as f64;
        let frame_of =
            |b: f64| (((b - beat0) / bps).round().max(0.0) as usize).min(n.saturating_sub(1));
        let latch = patch.arp_latch;
        // A transport jump (start, loop, locate): back on the grid.
        if let Some(next) = self.next
            && (self.realign || next < beat0 - rate || next > beat0 + 4.0 * rate)
        {
            self.next = Some((beat0 / rate).ceil() * rate);
            self.step = (beat0 / rate).ceil() as u64;
        }
        self.realign = false;
        let mut pattern = [(0u8, 0.0f32); MAX_PATTERN];
        let mut ei = 0;
        loop {
            let ev_beat = events
                .get(ei)
                .map_or(f64::INFINITY, |e| beat0 + bps * e.frame as f64);
            let off_beat = if self.sounding_len > 0 {
                self.off_at
            } else {
                f64::INFINITY
            };
            let step_beat = match self.next {
                Some(b) => {
                    b + if self.step % 2 == 1 {
                        patch.arp_swing as f64 * rate * 0.5
                    } else {
                        0.0
                    }
                }
                None => f64::INFINITY,
            };
            let t = ev_beat.min(off_beat).min(step_beat);
            if t >= end {
                break;
            }
            if t == off_beat {
                for &note in &self.sounding[..self.sounding_len] {
                    push(frame_of(t), [0x80, note, 0]);
                }
                self.sounding_len = 0;
                self.off_at = f64::INFINITY;
            } else if t == ev_beat {
                let e = events[ei];
                ei += 1;
                match e.data[0] & 0xF0 {
                    0x90 if e.data[2] > 0 => {
                        let was_empty = self.len == 0 || (latch && self.down == 0);
                        self.press_with(e.data[1] & 0x7F, e.data[2] as f32 / 127.0, latch);
                        if self.next.is_none() || was_empty && self.sounding_len == 0 {
                            // Start: right away when stopped or close after a grid
                            // line, else on the next one.
                            let grid = (t / rate).floor() * rate;
                            let start = if t - grid < rate * 0.2 {
                                t
                            } else {
                                grid + rate
                            };
                            if self.next.is_none() {
                                self.step = (start / rate).round() as u64;
                            }
                            self.next = Some(start.max(t));
                        }
                    }
                    0x80 | 0x90 => self.release_with(e.data[1] & 0x7F, latch),
                    _ => {}
                }
                if self.len == 0 {
                    self.next = None;
                }
            } else {
                // A step: the previous notes end, the next ones start.
                for &note in &self.sounding[..self.sounding_len] {
                    push(frame_of(t), [0x80, note, 0]);
                }
                self.sounding_len = 0;
                let count = self.pattern(patch, &mut pattern);
                if count > 0 {
                    let at = frame_of(t);
                    if patch.arp_mode == 6 {
                        for &(note, vel) in pattern[..count.min(self.len)].iter() {
                            push(
                                at,
                                [0x90, note, (vel * 127.0).round().clamp(1.0, 127.0) as u8],
                            );
                            if self.sounding_len < MAX_KEYS {
                                self.sounding[self.sounding_len] = note;
                                self.sounding_len += 1;
                            }
                        }
                    } else {
                        let i = if patch.arp_mode == 5 {
                            (self.rng.next_u32() as usize) % count
                        } else {
                            (self.step as usize) % count
                        };
                        let (note, vel) = pattern[i];
                        push(
                            at,
                            [0x90, note, (vel * 127.0).round().clamp(1.0, 127.0) as u8],
                        );
                        self.sounding[0] = note;
                        self.sounding_len = 1;
                    }
                    let gate = (patch.arp_gate as f64 * rate).max(bps * 2.0);
                    self.off_at = t + gate.min(rate * 0.999);
                }
                self.step += 1;
                self.next = self.next.map(|b| b + rate);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::audio::modules::sibyl::Event;

    fn patch() -> Patch {
        Patch {
            arp_on: true,
            arp_mode: 0,
            arp_beats: 0.25,
            arp_oct: 1,
            arp_gate: 0.5,
            arp_swing: 0.0,
            ..Patch::default()
        }
    }

    /// Plans `blocks` of 64 frames at 120 BPM from beat 0 with the keys
    /// pressed in the first block; returns (absolute frame, message).
    fn plan(patch: &Patch, blocks: usize, keys: &[u8]) -> Vec<(usize, [u8; 3])> {
        let mut arp = Arp::new();
        let bps = 2.0 / 48_000.0;
        let mut out = Vec::new();
        for b in 0..blocks {
            let events: Vec<Event> = if b == 0 {
                keys.iter()
                    .map(|&k| Event {
                        frame: 0,
                        data: [0x90, k, 100],
                    })
                    .collect()
            } else {
                Vec::new()
            };
            arp.plan(
                patch,
                b as f64 * 64.0 * bps,
                bps,
                64,
                &events,
                &mut |f, d| out.push((b * 64 + f, d)),
            );
        }
        out
    }

    #[test]
    fn steps_every_note_value_up_the_chord() {
        let out = plan(&patch(), 400, &[60, 64, 67]);
        let ons: Vec<(usize, u8)> = out
            .iter()
            .filter(|e| e.1[0] == 0x90)
            .map(|e| (e.0, e.1[1]))
            .collect();
        assert!(ons.len() >= 4, "{ons:?}");
        for (i, &(frame, note)) in ons.iter().enumerate() {
            assert_eq!(frame, i * 6000, "{ons:?}");
            assert_eq!(note, [60, 64, 67][i % 3]);
        }
        let offs: Vec<usize> = out.iter().filter(|e| e.1[0] == 0x80).map(|e| e.0).collect();
        assert_eq!(offs[0], 3000, "half a step: {offs:?}");
    }

    #[test]
    fn patterns_go_both_ways() {
        let mut p = patch();
        p.arp_mode = 2;
        p.arp_oct = 2;
        let mut arp = Arp::new();
        for k in [60, 64, 67] {
            arp.press(k, 1.0);
        }
        let mut out = [(0u8, 0.0f32); MAX_PATTERN];
        let n = arp.pattern(&p, &mut out);
        let notes: Vec<u8> = out[..n].iter().map(|x| x.0).collect();
        assert_eq!(notes, [60, 64, 67, 72, 76, 79, 76, 72, 67, 64]);
        p.arp_mode = 3;
        let n = arp.pattern(&p, &mut out);
        assert_eq!(n, 12);
    }
}
