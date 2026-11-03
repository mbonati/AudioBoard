//! The echo after the loop: a tape-ish delay a side, band-passed and soft
//! clipped on every repeat, so REPEATS can pass 100% and swell without
//! running away. PONG crosses the two lines, so the repeats bounce from
//! left to right, and DRIVE, TONE, WIDTH and GAIN shape them.

use super::parts::{Biquad, Kind, soft_clip};
use crate::audio::modules::DelayLine;

/// The band the repeats keep, unless PONG moves it.
pub const CENTER: f32 = 800.0;
/// The longest echo, in seconds.
pub const LONGEST: f32 = 5.0;
/// PONG fades in and out over this long.
const SWITCH: f32 = 0.02;
/// The delay time follows the knob like tape (a time constant).
const LAG: f32 = 0.5;

pub struct Echo {
    lines: [DelayLine; 2],
    sr: f32,
    /// The time now and where it is heading, in seconds.
    time: f32,
    target: f32,
    lag: f32,
    band: [Biquad; 2],
    center: f32,
    feedback: f32,
    /// How far PONG is in (0…1).
    pong: f32,
    pong_on: bool,
    pong_step: f32,
    drive: f32,
    tone: f32,
    width: f32,
    level: f32,
    drive_gain: f32,
    trim: f32,
}

/// What shapes the repeats with PONG on.
#[derive(Clone, Copy)]
pub struct Shape {
    /// 0…1: up to +12 dB into the soft clip.
    pub drive: f32,
    /// The band-pass's centre, in Hz.
    pub tone: f32,
    /// 0: every repeat in the middle; 1: fully left and right.
    pub width: f32,
    /// Linear gain on the repeats (not on what feeds back).
    pub level: f32,
}

impl Echo {
    pub fn new(sr: f32) -> Self {
        let mut band = [Biquad::default(); 2];
        for b in &mut band {
            b.set(Kind::Band, CENTER, 1.0, sr);
        }
        let len = (LONGEST * sr) as usize + 16;
        Self {
            lines: [DelayLine::new(len), DelayLine::new(len)],
            sr,
            time: 0.5,
            target: 0.5,
            lag: 1.0 / (LAG * sr),
            band,
            center: CENTER,
            feedback: 0.0,
            pong: 0.0,
            pong_on: false,
            pong_step: 1.0 / (SWITCH * sr),
            drive: 0.0,
            tone: CENTER,
            width: 1.0,
            level: 1.0,
            drive_gain: 1.0,
            trim: 1.0,
        }
    }

    /// The settings for the next block. `jump`: go straight to the time
    /// (and PONG) rather than glide there.
    pub fn set(&mut self, time: f32, feedback: f32, pong: bool, shape: Shape, jump: bool) {
        self.target = time.clamp(0.001, LONGEST);
        self.feedback = feedback;
        self.pong_on = pong;
        if jump {
            self.time = self.target;
            self.pong = if pong { 1.0 } else { 0.0 };
        }
        (self.drive, self.tone, self.width, self.level) = (shape.drive, shape.tone, shape.width, shape.level);
        // From the plain band to TONE as PONG comes in.
        let x = self.pong;
        let center = if x == 0.0 { CENTER } else { CENTER * (self.tone / CENTER).powf(x) };
        if center != self.center {
            self.center = center;
            for b in &mut self.band {
                b.set(Kind::Band, center, 1.0, self.sr);
            }
        }
        let drive = x * self.drive;
        self.drive_gain = 4f32.powf(drive);
        self.trim = 2f32.powf(-drive);
    }

    pub fn clear(&mut self) {
        for line in &mut self.lines {
            for _ in 0..line.len() {
                line.push(0.0);
            }
        }
        for b in &mut self.band {
            b.clear();
        }
    }

    /// The time between repeats now, in seconds.
    pub fn time(&self) -> f32 {
        self.time
    }

    /// How far PONG is in.
    pub fn pong(&self) -> f32 {
        self.pong
    }

    #[inline]
    pub fn process(&mut self, l: f32, r: f32) -> (f32, f32) {
        if self.pong != if self.pong_on { 1.0 } else { 0.0 } {
            let step = if self.pong_on { self.pong_step } else { -self.pong_step };
            self.pong = (self.pong + step).clamp(0.0, 1.0);
        }
        self.time += (self.target - self.time) * self.lag;
        let d = self.time * self.sr - 1.0;
        let ol = soft_clip(self.drive_gain * self.band[0].process(self.lines[0].tap(d), 0));
        let or = soft_clip(self.drive_gain * self.band[1].process(self.lines[1].tap(d), 1));
        let x = self.pong;
        if x == 0.0 {
            self.lines[0].push(ol * self.feedback + l);
            self.lines[1].push(or * self.feedback + r);
            return (ol, or);
        }
        // Crossed: each line repeats the other's, and the sound goes in on
        // the left alone, so the first repeat is on the left.
        let fb = self.feedback * self.trim;
        let mono = 0.5 * (l + r);
        self.lines[0].push((ol + x * (or - ol)) * fb + l + x * (mono - l));
        self.lines[1].push((or + x * (ol - or)) * fb + r * (1.0 - x));
        let width = 1.0 + x * (self.width - 1.0);
        let level = 1.0 + x * (self.level - 1.0);
        let mid = 0.5 * (ol + or);
        let side = width * 0.5 * (ol - or);
        (level * (mid + side), level * (mid - side))
    }
}
