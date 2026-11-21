//! Following a MIDI clock: 24 ticks a beat, Start, Continue, Stop and the
//! song position. Between ticks the position runs on at the tempo the
//! ticks have been keeping, so notes land between them too.

/// Ticks a beat.
const PPQ: f64 = 24.0;

pub struct Clock {
    /// Ticks since Start (−1: started, waiting for the first tick, which
    /// is beat 0).
    ticks: i64,
    running: bool,
    /// Samples since the last tick (negative: it comes that far into this
    /// block), and between ticks (smoothed).
    since: f64,
    period: f64,
    /// A Start arrived this block.
    started: bool,
    /// Ticks have come lately.
    alive: bool,
}

/// What the clock says for a block.
#[derive(Clone, Copy, Debug)]
pub struct Timeline {
    pub beat: f64,
    pub beats_per_sample: f64,
    pub running: bool,
    /// Started from the top this block.
    pub restart: bool,
}

impl Clock {
    pub fn new(sr: f32) -> Self {
        // 120 BPM until ticks say otherwise.
        Self {
            ticks: -1,
            running: false,
            since: 0.0,
            period: sr as f64 / (2.0 * PPQ),
            started: false,
            alive: false,
        }
    }

    /// A clock or transport message `frame` samples into the coming block
    /// (anything else is ignored).
    pub fn handle(&mut self, [status, d1, d2]: [u8; 3], frame: usize) {
        match status {
            0xF8 => {
                let interval = self.since + frame as f64;
                if self.ticks >= 0 && interval > 0.0 {
                    // Smoothed, but quick to follow a new tempo.
                    let k = if (interval / self.period - 1.0).abs() > 0.2 {
                        1.0
                    } else {
                        0.2
                    };
                    self.period += (interval - self.period) * k;
                }
                if self.running {
                    self.ticks += 1;
                }
                self.since = -(frame as f64);
                self.alive = true;
            }
            0xFA => {
                self.ticks = -1;
                self.running = true;
                self.started = true;
            }
            0xFB => self.running = true,
            0xFC => self.running = false,
            // Song position: sixteenths, six ticks each.
            0xF2 => self.ticks = (((d2 as i64) << 7) | d1 as i64) * 6 - 1,
            _ => {}
        }
    }

    /// The timeline for a block of `n` samples, after this block's
    /// messages.
    pub fn block(&mut self, n: usize) -> Timeline {
        // Silent for two beats' worth of ticks: the clock has gone.
        if self.since > self.period * PPQ * 2.0 {
            self.alive = false;
        }
        // A tick later in this block: from the one before it.
        let (base, since) = if self.since < 0.0 {
            (self.ticks - 1, self.since + self.period)
        } else {
            (self.ticks, self.since)
        };
        let beat = if base >= 0 {
            (base as f64 + (since / self.period).clamp(0.0, 1.0)) / PPQ
        } else {
            0.0
        };
        let t = Timeline {
            beat,
            beats_per_sample: 1.0 / (PPQ * self.period),
            running: self.running && self.alive,
            restart: self.started,
        };
        self.started = false;
        self.since += n as f64;
        t
    }

    /// The tempo the ticks keep (0 when none come).
    pub fn bpm(&self, sr: f32) -> f32 {
        if self.alive {
            (60.0 * sr as f64 / (PPQ * self.period)) as f32
        } else {
            0.0
        }
    }
}
