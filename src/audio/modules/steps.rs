//! Timing and note bookkeeping shared by the MIDI generators (sequencer,
//! arpeggiator, MIDI In). Steps sit on the global beat grid, so every
//! generator stays in time with the others and with the tempo.

use super::{ModuleParams, Rng};
use crate::audio::engine::{MidiBuf, ProcessCtx};
use crate::audio::params::DIVISIONS;

/// Steps starting within one block, at most.
pub const MAX_STEPS: usize = 8;

/// A step starting in this block.
#[derive(Clone, Copy, Debug)]
pub struct Step {
    pub frame: usize,
    /// Its position on the beat grid.
    pub beat: f64,
    /// Steps since beat 0: the pattern position is this modulo its length.
    pub index: i64,
}

/// Steps per beat for a RATE knob value.
pub fn steps_per_beat(rate: f32) -> f64 {
    DIVISIONS[(rate.round().max(0.0) as usize).min(DIVISIONS.len() - 1)].1
}

/// Finds the steps on the grid as blocks go by.
pub struct StepClock {
    /// Beat of the last step reported, so a step never plays twice when
    /// the rate changes.
    last: f64,
}

impl StepClock {
    pub fn new() -> Self {
        Self { last: f64::NEG_INFINITY }
    }

    /// The steps starting in this block, in order. Odd steps are pushed
    /// late by `swing` (0…1, up to half a step). A step belongs to the
    /// block holding its nearest frame.
    pub fn steps(&mut self, ctx: &ProcessCtx, n: usize, per_beat: f64, swing: f32, out: &mut [Step; MAX_STEPS]) -> usize {
        let len = 1.0 / per_beat;
        let at = |k: i64| (k as f64 + if k & 1 == 1 { swing as f64 * 0.5 } else { 0.0 }) * len;
        let mut count = 0;
        let mut k = (ctx.beat / len).floor() as i64 - 1;
        while frame_at(ctx, at(k)) < n as f64 - 0.5 && count < MAX_STEPS {
            let beat = at(k);
            if frame_at(ctx, beat) >= -0.5 && beat > self.last {
                self.last = beat;
                out[count] = Step { frame: frame_of(ctx, n, beat), beat, index: k };
                count += 1;
            }
            k += 1;
        }
        count
    }
}

/// The note a generator is sounding and the beat it ends on.
pub struct Voice {
    note: Option<(u8, f64)>,
}

impl Voice {
    pub fn new() -> Self {
        Self { note: None }
    }

    pub fn note(&self) -> Option<u8> {
        self.note.map(|(n, _)| n)
    }

    /// Ends the note if it ends before `beat` (see [`block_end`]).
    pub fn release_before(&mut self, out: &mut MidiBuf, ctx: &ProcessCtx, n: usize, beat: f64) {
        if let Some((note, end)) = self.note
            && end < beat
        {
            out.push(frame_of(ctx, n, end), [0x80, note, 0]);
            self.note = None;
        }
    }

    /// Ends the current note and starts `note`, held for `length` beats.
    pub fn play(&mut self, out: &mut MidiBuf, step: &Step, note: u8, velocity: u8, length: f64) {
        self.stop(out, step.frame);
        out.push(step.frame, [0x90, note, velocity.max(1)]);
        self.note = Some((note, step.beat + length));
    }

    /// Ends the current note now.
    pub fn stop(&mut self, out: &mut MidiBuf, frame: usize) {
        if let Some((note, _)) = self.note.take() {
            out.push(frame, [0x80, note, 0]);
        }
    }
}

/// Frames from the start of this block to a beat.
fn frame_at(ctx: &ProcessCtx, beat: f64) -> f64 {
    (beat - ctx.beat) / ctx.beats_per_sample
}

/// The nearest frame within this block for a beat.
pub fn frame_of(ctx: &ProcessCtx, n: usize, beat: f64) -> usize {
    (frame_at(ctx, beat).round().max(0.0) as usize).min(n.saturating_sub(1))
}

/// The beat where this block's last frame stops being nearest: notes ending
/// before it end in this block.
pub fn block_end(ctx: &ProcessCtx, n: usize) -> f64 {
    ctx.beat + ctx.beats_per_sample * (n as f64 - 0.5)
}

/// Keys being held, in the order they were pressed (at most [`Held::CAP`]).
#[derive(Clone)]
pub struct Held {
    notes: [u8; Held::CAP],
    len: usize,
}

impl Held {
    pub const CAP: usize = 16;

    pub fn new() -> Self {
        Self { notes: [0; Self::CAP], len: 0 }
    }

    pub fn notes(&self) -> &[u8] {
        &self.notes[..self.len]
    }

    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn press(&mut self, note: u8) {
        self.release(note);
        if self.len < Self::CAP {
            self.notes[self.len] = note;
            self.len += 1;
        }
    }

    pub fn release(&mut self, note: u8) {
        if let Some(i) = self.notes().iter().position(|&n| n == note) {
            self.notes.copy_within(i + 1..self.len, i);
            self.len -= 1;
        }
    }
}

/// A note-on (`Some(true)`), note-off (`Some(false)`) or anything else.
pub fn note_event([status, _, velocity]: [u8; 3]) -> Option<bool> {
    match status & 0xF0 {
        0x90 if velocity > 0 => Some(true),
        0x80 | 0x90 => Some(false),
        _ => None,
    }
}

/// `true` with probability `chance`.
pub fn roll(rng: &mut Rng, chance: f32) -> bool {
    chance >= 1.0 || rng.unit() < chance
}

/// Layout of `viz` for nodes that show notes on a keyboard.
pub mod viz {
    /// The note sounding now, or −1.
    pub const CURRENT: usize = 0;
    /// How many entries of `NOTES` are in use.
    pub const COUNT: usize = 1;
    pub const NOTES: usize = 2;
    pub const SHOWN: usize = 12;
    pub const LEN: usize = NOTES + SHOWN;
}

/// Publishes the sounding note and the held ones for the keyboard display.
pub fn publish_notes(p: &ModuleParams, current: Option<u8>, held: &[u8]) {
    p.viz[viz::CURRENT].set(current.map_or(-1.0, f32::from));
    let shown = held.len().min(viz::SHOWN);
    p.viz[viz::COUNT].set(shown as f32);
    for (slot, &note) in p.viz[viz::NOTES..viz::NOTES + shown].iter().zip(held) {
        slot.set(note as f32);
    }
}
