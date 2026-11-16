//! MIDI sync: the transport following an external MIDI clock, and sending
//! one.
//!
//! **Following.** The MIDI thread of the chosen input times each clock tick
//! (24 a beat) and keeps the tempo they set: the average period over the
//! last beat of ticks, which smooths their jitter, restarting quickly when
//! the tempo jumps. Start, Stop, Continue and the song position go along.
//! The engine reads this at every callback ([`SyncShared::clock`]) and
//! steers its own beat to the clock's: at the clock's tempo, nudged ahead
//! or back to stay on it, jumping only when far out (a new song position).
//!
//! **Sending.** A thread sends the board's transport out: 24 ticks a beat at
//! the engine's beat and tempo (also while stopped, so a device can show the
//! tempo), Start when the board starts from the top, Stop, and the song
//! position plus Continue when it starts elsewhere. Ticks leave when their
//! beat is heard, one audio buffer after it is rendered.

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, Ordering::Acquire, Ordering::Relaxed, Ordering::Release};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// Clock ticks a beat.
pub const PPQ: f64 = 24.0;

/// Nanoseconds since the first call: one clock for the MIDI threads, the
/// audio thread and the clock-out thread.
pub fn now_ns() -> u64 {
    static EPOCH: OnceLock<Instant> = OnceLock::new();
    // Never 0, which means "never" below.
    EPOCH.get_or_init(Instant::now).elapsed().as_nanos() as u64 + 1
}

/// The clock as last heard.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ClockState {
    /// Nanoseconds between ticks (smoothed).
    pub period_ns: f64,
    /// When the last tick came (0: none yet).
    pub last_tick_ns: u64,
    /// Ticks since Start, counting the last one (−1: started, waiting for
    /// the first tick, which is beat 0).
    pub ticks: i64,
    /// Started or continued, not stopped.
    pub running: bool,
    /// Bumped by every Start: the engine goes back to beat 0.
    pub starts: u32,
    /// Bumped by every Continue and song position: the engine jumps to
    /// the clock's beat.
    pub moves: u32,
}

impl ClockState {
    /// The clock's tempo.
    pub fn bpm(&self) -> f64 {
        60e9 / (self.period_ns.max(1.0) * PPQ)
    }

    /// Ticks keep coming: none for eight ticks (and at least 0.3 s) means
    /// the clock has gone.
    pub fn alive(&self, now: u64) -> bool {
        self.last_tick_ns != 0 && (now.saturating_sub(self.last_tick_ns) as f64) < (8.0 * self.period_ns).max(3e8)
    }

    /// The transport is moving: running, ticking, and past its first tick.
    pub fn playing(&self, now: u64) -> bool {
        self.running && self.ticks >= 0 && self.alive(now)
    }

    /// The beat at time `at`: the last tick's, carried on at the clock's
    /// tempo for at most two ticks (enough to look a buffer ahead; past
    /// that the clock is late, and time waits for it).
    pub fn beat_at(&self, at: u64) -> f64 {
        let since = at as f64 - self.last_tick_ns as f64;
        (self.ticks.max(0) as f64 + (since / self.period_ns.max(1.0)).clamp(0.0, 2.0)) / PPQ
    }
}

/// The board's transport as the engine last rendered it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Timeline {
    /// The beat at the start of the buffer being rendered.
    pub beat: f64,
    pub bpm: f64,
    pub playing: bool,
    /// When that buffer was rendered (0: no audio yet), and how long until
    /// it is heard.
    pub at_ns: u64,
    pub latency_ns: u64,
}

impl Timeline {
    /// The beat heard at time `t`.
    pub fn heard_beat(&self, t: u64) -> f64 {
        if !self.playing {
            return self.beat;
        }
        let since = t as f64 - (self.at_ns + self.latency_ns) as f64;
        self.beat + since * self.bpm / 60e9
    }

    /// When beat `beat` is heard.
    pub fn time_of(&self, beat: f64) -> f64 {
        (self.at_ns + self.latency_ns) as f64 + (beat - self.beat) * 60e9 / self.bpm.max(1.0)
    }
}

/// Shared by the MIDI threads (the clock heard), the engine (following it,
/// and publishing its own timeline) and the clock-out thread.
pub struct SyncShared {
    /// The transport follows the external clock (set by the UI).
    pub follow: AtomicBool,
    /// The MIDI input that is the clock source (its connection id; 0: none).
    pub source: AtomicU32,
    /// The clock, written by the source's MIDI thread (a seqlock: odd while
    /// writing).
    clock_seq: AtomicU32,
    period_ns: AtomicU64,
    last_tick_ns: AtomicU64,
    ticks: AtomicI64,
    running: AtomicBool,
    starts: AtomicU32,
    moves: AtomicU32,
    /// The engine's timeline (also a seqlock).
    out_seq: AtomicU32,
    out_beat: AtomicU64,
    out_bpm: AtomicU64,
    out_playing: AtomicBool,
    out_at: AtomicU64,
    out_latency: AtomicU64,
    /// Ticks sent by the clock-out thread, for the settings window.
    pub sent: AtomicU64,
}

impl Default for SyncShared {
    fn default() -> Self {
        Self::new()
    }
}

impl SyncShared {
    pub fn new() -> Self {
        Self {
            follow: AtomicBool::new(false),
            source: AtomicU32::new(0),
            clock_seq: AtomicU32::new(0),
            period_ns: AtomicU64::new((60e9 / (120.0 * PPQ)).to_bits()),
            last_tick_ns: AtomicU64::new(0),
            ticks: AtomicI64::new(-1),
            running: AtomicBool::new(false),
            starts: AtomicU32::new(0),
            moves: AtomicU32::new(0),
            out_seq: AtomicU32::new(0),
            out_beat: AtomicU64::new(0),
            out_bpm: AtomicU64::new(120f64.to_bits()),
            out_playing: AtomicBool::new(false),
            out_at: AtomicU64::new(0),
            out_latency: AtomicU64::new(0),
            sent: AtomicU64::new(0),
        }
    }

    /// The clock as last heard. Lock-free (the audio thread reads it).
    pub fn clock(&self) -> ClockState {
        loop {
            let seq = self.clock_seq.load(Acquire);
            if seq & 1 == 1 {
                std::hint::spin_loop();
                continue;
            }
            let state = ClockState {
                period_ns: f64::from_bits(self.period_ns.load(Acquire)),
                last_tick_ns: self.last_tick_ns.load(Acquire),
                ticks: self.ticks.load(Acquire),
                running: self.running.load(Acquire),
                starts: self.starts.load(Acquire),
                moves: self.moves.load(Acquire),
            };
            if self.clock_seq.load(Acquire) == seq {
                return state;
            }
        }
    }

    fn write_clock(&self, s: &ClockState) {
        let seq = self.clock_seq.load(Relaxed);
        self.clock_seq.store(seq.wrapping_add(1), Release);
        self.period_ns.store(s.period_ns.to_bits(), Release);
        self.last_tick_ns.store(s.last_tick_ns, Release);
        self.ticks.store(s.ticks, Release);
        self.running.store(s.running, Release);
        self.starts.store(s.starts, Release);
        self.moves.store(s.moves, Release);
        self.clock_seq.store(seq.wrapping_add(2), Release);
    }

    /// The engine's timeline, for the clock-out thread and the UI.
    pub fn timeline(&self) -> Timeline {
        loop {
            let seq = self.out_seq.load(Acquire);
            if seq & 1 == 1 {
                std::hint::spin_loop();
                continue;
            }
            let t = Timeline {
                beat: f64::from_bits(self.out_beat.load(Acquire)),
                bpm: f64::from_bits(self.out_bpm.load(Acquire)),
                playing: self.out_playing.load(Acquire),
                at_ns: self.out_at.load(Acquire),
                latency_ns: self.out_latency.load(Acquire),
            };
            if self.out_seq.load(Acquire) == seq {
                return t;
            }
        }
    }

    /// Written by the engine once per callback.
    pub fn publish(&self, t: Timeline) {
        let seq = self.out_seq.load(Relaxed);
        self.out_seq.store(seq.wrapping_add(1), Release);
        self.out_beat.store(t.beat.to_bits(), Release);
        self.out_bpm.store(t.bpm.to_bits(), Release);
        self.out_playing.store(t.playing, Release);
        self.out_at.store(t.at_ns, Release);
        self.out_latency.store(t.latency_ns, Release);
        self.out_seq.store(seq.wrapping_add(2), Release);
    }
}

/// Tick times kept for the tempo: one beat's worth.
const WINDOW: usize = 24;

/// Times one input's clock (on its MIDI thread) and writes what it hears
/// into [`SyncShared`] while that input is the source.
pub struct ClockFollower {
    times: [u64; WINDOW],
    /// Tick times kept, the newest at `head - 1`.
    count: usize,
    head: usize,
    /// Intervals in a row far from the tempo, the same way (slower or
    /// faster): two mean a new tempo.
    odd: u32,
    slower: bool,
    state: ClockState,
}

impl Default for ClockFollower {
    fn default() -> Self {
        Self::new()
    }
}

impl ClockFollower {
    pub fn new() -> Self {
        Self {
            times: [0; WINDOW],
            count: 0,
            head: 0,
            odd: 0,
            slower: false,
            state: ClockState {
                period_ns: 60e9 / (120.0 * PPQ),
                last_tick_ns: 0,
                ticks: -1,
                running: false,
                starts: 0,
                moves: 0,
            },
        }
    }

    #[cfg(test)]
    pub fn state(&self) -> ClockState {
        self.state
    }

    fn newest(&self) -> Option<u64> {
        (self.count > 0).then(|| self.times[(self.head + WINDOW - 1) % WINDOW])
    }

    fn oldest(&self) -> u64 {
        self.times[(self.head + WINDOW - self.count) % WINDOW]
    }

    /// The line that best fits the kept tick times (least squares): the
    /// tick period, and where it puts the newest tick (`now` with fewer
    /// than three kept). A late or early tick barely moves either.
    fn fit(&self, now: u64) -> (f64, u64) {
        let n = self.count;
        if n < 3 {
            let period = if n == 2 { now.saturating_sub(self.oldest()) as f64 } else { self.state.period_ns };
            return (if period > 0.0 { period } else { self.state.period_ns }, now);
        }
        let first = self.oldest();
        let at = |k: usize| self.times[(self.head + WINDOW - n + k) % WINDOW].saturating_sub(first) as f64;
        let mean_x = (n - 1) as f64 / 2.0;
        let mean_y = (0..n).map(at).sum::<f64>() / n as f64;
        let (mut num, mut den) = (0.0, 0.0);
        for k in 0..n {
            let dx = k as f64 - mean_x;
            num += dx * (at(k) - mean_y);
            den += dx * dx;
        }
        let period = if den > 0.0 && num > 0.0 { num / den } else { self.state.period_ns };
        let newest = first as f64 + mean_y + period * (n as f64 - 1.0 - mean_x);
        (period, (newest.max(1.0) as u64).min(now))
    }

    fn restart_window(&mut self, keep: &[u64]) {
        self.count = 0;
        self.head = 0;
        for &t in keep {
            self.push(t);
        }
    }

    fn push(&mut self, t: u64) {
        self.times[self.head] = t;
        self.head = (self.head + 1) % WINDOW;
        self.count = (self.count + 1).min(WINDOW);
    }

    /// One message at time `now`; true if it was clock or transport.
    pub fn message(&mut self, msg: &[u8], now: u64) -> bool {
        let Some(&status) = msg.first() else { return false };
        let data = |i: usize| msg.get(i).copied().unwrap_or(0) as i64 & 0x7F;
        let s = &mut self.state;
        match status {
            0xF8 => {
                match self.newest() {
                    // A gap (the clock stopped and came back): start over.
                    Some(last) if now.saturating_sub(last) > 500_000_000 => self.restart_window(&[now]),
                    Some(last) => {
                        // Jitter makes one interval long and the next short;
                        // a new tempo makes several in a row off the same way.
                        let ratio = now.saturating_sub(last) as f64 / self.state.period_ns;
                        let slower = ratio > 1.0;
                        self.odd = match (ratio - 1.0).abs() > 0.15 && self.count >= 3 {
                            true if self.odd > 0 && slower == self.slower => self.odd + 1,
                            true => 1,
                            false => 0,
                        };
                        self.slower = slower;
                        if self.odd >= 3 {
                            // A new tempo: from the last three intervals.
                            let keep = [3, 2].map(|back| self.times[(self.head + WINDOW - back) % WINDOW]);
                            self.restart_window(&[keep[0], keep[1], last]);
                            self.odd = 0;
                        }
                        self.push(now);
                    }
                    None => self.push(now),
                }
                // The tempo, and the last tick's time, from the line that
                // best fits the ticks kept: their jitter averages out.
                let (period, newest) = self.fit(now);
                let s = &mut self.state;
                s.period_ns = period;
                if s.running {
                    s.ticks += 1;
                }
                s.last_tick_ns = newest;
            }
            0xFA => {
                s.ticks = -1;
                s.running = true;
                s.starts = s.starts.wrapping_add(1);
            }
            0xFB => {
                s.running = true;
                s.moves = s.moves.wrapping_add(1);
            }
            0xFC => s.running = false,
            // Song position: sixteenths, six ticks each. The next tick is
            // that position.
            0xF2 => {
                s.ticks = ((data(2) << 7) | data(1)) * 6 - 1;
                s.moves = s.moves.wrapping_add(1);
            }
            _ => return false,
        }
        true
    }

    /// Takes a message and, if it was clock or transport, writes the
    /// clock into `shared`.
    pub fn handle(&mut self, msg: &[u8], now: u64, shared: &SyncShared) {
        if self.message(msg, now) {
            shared.write_clock(&self.state);
        }
    }
}

/// How the engine steers its beat to the clock's (see `Engine::render_at`).
pub struct Follow {
    starts: u32,
    moves: u32,
    was_playing: bool,
}

impl Follow {
    pub fn new(shared: &SyncShared) -> Self {
        let c = shared.clock();
        Self { starts: c.starts, moves: c.moves, was_playing: false }
    }

    /// For a callback of `frames` at `sample_rate`, from the board's beat
    /// `beat`: (beat to start from, beats per sample, playing, rewound).
    /// The beat is the clock's when the buffer will be heard, one buffer
    /// from `now`.
    pub fn step(&mut self, clock: &ClockState, now: u64, beat: f64, frames: usize, sample_rate: f32) -> (f64, f64, bool, bool) {
        let sr = sample_rate as f64;
        let rewound = clock.starts != self.starts;
        self.starts = clock.starts;
        let moved = clock.moves != self.moves;
        self.moves = clock.moves;
        let mut beat = if rewound { 0.0 } else { beat };
        let playing = clock.playing(now);
        let nominal = clock.bpm() / (60.0 * sr);
        if !playing {
            self.was_playing = false;
            return (beat, nominal, false, rewound);
        }
        let heard = now + (frames as f64 / sr * 1e9) as u64;
        let target = clock.beat_at(heard);
        let error = target - beat;
        let per_sample = if !self.was_playing || moved || error.abs() > 0.25 {
            beat = target;
            nominal
        } else {
            // Close the gap over about 0.3 s, never more than ±4% off the
            // clock's tempo (pitch-locked plugins barely notice).
            (nominal + error / (0.3 * sr)).clamp(nominal * 0.96, nominal * 1.04)
        };
        self.was_playing = true;
        (beat, per_sample, true, rewound)
    }
}

/// Sends the board's clock to MIDI outputs, from a thread of its own.
pub struct ClockOut {
    tx: Sender<Vec<String>>,
    /// The outputs connected now (names), and why one could not open.
    pub status: Arc<std::sync::Mutex<(Vec<String>, Option<String>)>>,
}

impl ClockOut {
    pub fn start(shared: Arc<super::params::EngineShared>) -> Self {
        let (tx, rx) = mpsc::channel();
        let status = Arc::new(std::sync::Mutex::new((Vec::new(), None)));
        let thread_status = status.clone();
        let _ = std::thread::Builder::new()
            .name("midi-clock-out".into())
            .spawn(move || clock_out_thread(rx, shared, thread_status));
        Self { tx, status }
    }

    /// Sends to these outputs (by name; none: off).
    pub fn set_outputs(&self, names: Vec<String>) {
        let _ = self.tx.send(names);
    }

    pub fn connected(&self) -> (Vec<String>, Option<String>) {
        self.status.lock().map(|s| s.clone()).unwrap_or_default()
    }
}

/// MIDI outputs on the system.
pub fn output_names() -> Vec<String> {
    let Ok(out) = midir::MidiOutput::new("AudioBoard") else { return Vec::new() };
    out.ports().iter().filter_map(|p| out.port_name(p).ok()).collect()
}

/// Asks the scheduler for prompt wake-ups: ticks are due every few ms.
fn timing_thread() {
    #[cfg(target_os = "macos")]
    {
        unsafe extern "C" {
            fn pthread_set_qos_class_self_np(qos: u32, relative: i32) -> i32;
        }
        // QOS_CLASS_USER_INTERACTIVE.
        // SAFETY: sets the calling thread's own scheduling class.
        unsafe {
            pthread_set_qos_class_self_np(0x21, 0);
        }
    }
}

fn clock_out_thread(rx: Receiver<Vec<String>>, shared: Arc<super::params::EngineShared>, status: Arc<std::sync::Mutex<(Vec<String>, Option<String>)>>) {
    timing_thread();
    let sync = &shared.sync;
    let mut wanted: Vec<String> = Vec::new();
    let mut conns: Vec<(String, midir::MidiOutputConnection)> = Vec::new();
    let mut checked = Instant::now() - Duration::from_secs(10);
    let mut playing = false;
    let mut next_tick: i64 = 0;
    let mut last_free: u64 = 0;
    loop {
        // New outputs, or (while off) wait for some.
        let command = if wanted.is_empty() {
            match rx.recv_timeout(Duration::from_millis(500)) {
                Ok(c) => Some(c),
                Err(RecvTimeoutError::Timeout) => None,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        } else {
            match rx.try_recv() {
                Ok(c) => Some(c),
                Err(mpsc::TryRecvError::Empty) => None,
                Err(mpsc::TryRecvError::Disconnected) => return,
            }
        };
        if let Some(names) = command {
            // A device that stops hearing the clock is told to stop.
            for (name, conn) in conns.iter_mut() {
                if playing && !names.contains(name) {
                    let _ = conn.send(&[0xFC]);
                }
            }
            conns.retain(|(n, _)| names.contains(n));
            wanted = names;
            checked = Instant::now() - Duration::from_secs(10);
        }
        if wanted.is_empty() {
            playing = false;
            if let Ok(mut s) = status.lock() {
                *s = (Vec::new(), None);
            }
            continue;
        }
        // Outputs that appeared since (a device plugged in).
        if checked.elapsed() > Duration::from_secs(2) {
            checked = Instant::now();
            let mut error = None;
            let missing: Vec<String> = wanted.iter().filter(|n| conns.iter().all(|(c, _)| c != *n)).cloned().collect();
            for name in &missing {
                let Ok(out) = midir::MidiOutput::new("AudioBoard") else { continue };
                let Some(port) = out.ports().into_iter().find(|p| out.port_name(p).ok().as_deref() == Some(name.as_str())) else {
                    continue;
                };
                match out.connect(&port, "AudioBoard clock") {
                    Ok(conn) => conns.push((name.clone(), conn)),
                    Err(e) => error = Some(format!("{name}: {e}")),
                }
            }
            if let Ok(mut s) = status.lock() {
                *s = (conns.iter().map(|(n, _)| n.clone()).collect(), error);
            }
        }

        let tl = sync.timeline();
        let now = now_ns();
        if tl.at_ns == 0 || conns.is_empty() {
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        let mut send = |bytes: &[u8]| {
            conns.retain_mut(|(_, c)| c.send(bytes).is_ok());
        };
        let tick_ns = 60e9 / (tl.bpm.max(1.0) * PPQ);
        if tl.playing != playing {
            playing = tl.playing;
            if playing {
                let heard = tl.heard_beat(now);
                // From the top (the board starts at beat 0; a followed
                // clock's Start lands within a tick of it): Start, and the
                // next tick is beat 0.
                if tl.beat < 0.125 {
                    send(&[0xFA]);
                    next_tick = 0;
                } else {
                    let sixteenths = ((heard * 4.0).floor() as i64).clamp(0, 0x3FFF);
                    send(&[0xF2, (sixteenths & 0x7F) as u8, (sixteenths >> 7) as u8]);
                    send(&[0xFB]);
                    next_tick = sixteenths * 6;
                }
            } else {
                send(&[0xFC]);
                last_free = now;
            }
        }
        let due = if playing { tl.time_of(next_tick as f64 / PPQ) } else { last_free as f64 + tick_ns };
        if due <= now as f64 + 200_000.0 {
            send(&[0xF8]);
            sync.sent.fetch_add(1, Relaxed);
            if playing {
                next_tick += 1;
                // Far behind (the machine slept): carry on from now.
                if (now as f64 - tl.time_of(next_tick as f64 / PPQ)) > 4.0 * tick_ns {
                    next_tick = (tl.heard_beat(now) * PPQ).ceil() as i64;
                }
            } else {
                last_free = if now as f64 - due > 4.0 * tick_ns { now } else { due as u64 };
            }
            continue;
        }
        // Wake for the tick, or sooner to see a tempo or transport change.
        let wait = ((due - now as f64) as u64).min(4_000_000);
        std::thread::sleep(Duration::from_nanos(wait));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1_000_000;

    /// 120 BPM: a tick every 20.833 ms.
    fn tick_ns(bpm: f64) -> f64 {
        60e9 / (bpm * PPQ)
    }

    #[test]
    fn follower_tracks_tempo_through_jitter() {
        let mut f = ClockFollower::new();
        let t0 = 1_000 * MS;
        f.message(&[0xFA], t0);
        // 2 beats at 100 BPM with ±1 ms jitter (4% of a tick).
        let period = tick_ns(100.0);
        for k in 0..48u64 {
            let jitter = if k % 2 == 0 { MS as f64 } else { -(MS as f64) };
            f.message(&[0xF8], t0 + (k as f64 * period + jitter) as u64);
        }
        let s = f.state();
        assert!((s.bpm() - 100.0).abs() < 0.3, "{}", s.bpm());
        assert_eq!(s.ticks, 47);
        assert!(s.running);
        // A jump to 140 BPM is followed within a few ticks.
        let last = s.last_tick_ns;
        for k in 1..=4u64 {
            f.message(&[0xF8], last + (k as f64 * tick_ns(140.0)) as u64);
        }
        assert!((f.state().bpm() - 140.0).abs() < 1.0, "{}", f.state().bpm());
    }

    #[test]
    fn transport_messages() {
        let mut f = ClockFollower::new();
        let shared = SyncShared::new();
        let t = 5_000 * MS;
        f.handle(&[0xFA], t, &shared);
        assert_eq!(shared.clock().ticks, -1);
        assert_eq!(shared.clock().starts, 1);
        f.handle(&[0xF8], t + 20 * MS, &shared);
        assert_eq!(shared.clock().ticks, 0);
        f.handle(&[0xFC], t + 30 * MS, &shared);
        assert!(!shared.clock().running);
        // Ticks while stopped keep the tempo, not the position.
        f.handle(&[0xF8], t + 41 * MS, &shared);
        assert_eq!(shared.clock().ticks, 0);
        // Song position 16 sixteenths = beat 4, then Continue.
        f.handle(&[0xF2, 16, 0], t + 50 * MS, &shared);
        f.handle(&[0xFB], t + 51 * MS, &shared);
        let c = shared.clock();
        assert!(c.running && c.moves == 2 && c.ticks == 95);
        f.handle(&[0xF8], t + 62 * MS, &shared);
        assert_eq!(shared.clock().ticks, 96);
        assert!((shared.clock().beat_at(t + 62 * MS) - 4.0).abs() < 1e-9);
        // Notes are not clock.
        assert!(!f.message(&[0x90, 60, 100], t));
    }

    #[test]
    fn engine_locks_onto_the_clock() {
        let sr = 48_000.0f32;
        let frames = 256;
        let shared = SyncShared::new();
        let mut f = ClockFollower::new();
        let mut follow = Follow::new(&shared);
        let period = tick_ns(123.0);
        let t0 = 2_000 * MS;
        f.handle(&[0xFA], t0, &shared);
        let mut next_tick = 0u64;
        let (mut beat, mut playing) = (0.0f64, false);
        let mut rewinds = 0;
        // Four seconds of callbacks, ticks arriving in between.
        let buffer_ns = frames as f64 / sr as f64 * 1e9;
        for k in 0..((4e9 / buffer_ns) as u64) {
            let now = t0 + 5 * MS + (k as f64 * buffer_ns) as u64;
            while t0 + 5 * MS + (next_tick as f64 * period) as u64 <= now {
                f.handle(&[0xF8], t0 + 5 * MS + (next_tick as f64 * period) as u64, &shared);
                next_tick += 1;
            }
            let (b, per_sample, p, rewound) = follow.step(&shared.clock(), now, beat, frames, sr);
            rewinds += rewound as u32;
            playing = p;
            beat = b + if p { per_sample * frames as f64 } else { 0.0 };
            if k > 40 {
                assert!(p, "plays once ticking");
                // On the clock's beat when heard, within a few ms.
                let heard = now + 2 * buffer_ns as u64;
                let target = shared.clock().beat_at(heard);
                assert!((beat - target).abs() < 0.01, "{beat} vs {target}");
            }
        }
        assert!(playing);
        assert_eq!(rewinds, 1);
        // Stop holds the beat.
        f.handle(&[0xFC], t0 + 4_100 * MS, &shared);
        let (b, _, p, _) = follow.step(&shared.clock(), t0 + 4_101 * MS, beat, frames, sr);
        assert!(!p && b == beat);
    }

    /// The whole chain through CoreMIDI, with virtual ports (no hardware):
    /// a clock at 128 BPM into a MIDI input that is the sync source, the
    /// engine following it in real time, and the board's clock sent out to
    /// another port. `cargo test -- --ignored midi_sync_end_to_end`.
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore]
    fn midi_sync_end_to_end() {
        use midir::os::unix::{VirtualInput, VirtualOutput};
        use std::sync::atomic::Ordering::Relaxed;
        let shared = Arc::new(crate::audio::params::EngineShared::new());
        let (audio, mut engine) = crate::audio::offline(shared.clone(), 48_000);

        // A virtual device sending clock, and one receiving the board's.
        let mut source = midir::MidiOutput::new("AB test").unwrap().create_virtual("AB Sync Source").unwrap();
        let heard = Arc::new(std::sync::Mutex::new(Vec::<(u8, u64)>::new()));
        let sink_log = heard.clone();
        let _sink = midir::MidiInput::new("AB test")
            .unwrap()
            .create_virtual("AB Clock Sink", move |_, msg, _| sink_log.lock().unwrap().push((msg[0], now_ns())), ())
            .unwrap();
        std::thread::sleep(Duration::from_millis(300));

        let mut inputs = crate::audio::midi::MidiInputs::new();
        inputs.rescan(&audio.midi, &shared, || {});
        inputs.set_sync_source(Some("AB Sync Source".into()), &shared.sync);
        assert!(shared.sync.source.load(Relaxed) != 0, "the virtual source is connected: {:?}", inputs.names().collect::<Vec<_>>());
        let out = ClockOut::start(shared.clone());
        out.set_outputs(vec!["AB Clock Sink".into()]);

        // The clock: Start, then 128 BPM for three seconds, from a thread.
        let period = Duration::from_secs_f64(60.0 / (128.0 * PPQ));
        let sender = std::thread::spawn(move || {
            source.send(&[0xFA]).unwrap();
            let start = Instant::now();
            for k in 0..(3.0 / period.as_secs_f64()) as u32 {
                let due = start + period * k;
                if let Some(wait) = due.checked_duration_since(Instant::now()) {
                    std::thread::sleep(wait);
                }
                source.send(&[0xF8]).unwrap();
            }
            source.send(&[0xFC]).unwrap();
            source
        });
        // The engine in real time, 512 frames a callback.
        let mut buf = vec![0.0f32; 512 * 2];
        let started = Instant::now();
        let mut beats_seen = Vec::new();
        let mut phase = Vec::new();
        while started.elapsed() < Duration::from_millis(2900) {
            let at = started + Duration::from_secs_f64(beats_seen.len() as f64 * 512.0 / 48_000.0);
            if let Some(wait) = at.checked_duration_since(Instant::now()) {
                std::thread::sleep(wait);
            }
            engine.render(&mut buf, 2);
            let tl = shared.sync.timeline();
            let target = shared.sync.clock().beat_at(tl.at_ns + tl.latency_ns);
            phase.push(tl.beat - target);
            beats_seen.push((tl, shared.running.load(Relaxed)));
        }
        let _source = sender.join().unwrap();
        let clock = shared.sync.clock();
        eprintln!("clock heard: {:.2} BPM, {} ticks", clock.bpm(), clock.ticks);
        assert!((clock.bpm() - 128.0).abs() < 0.5, "tempo {:.2}", clock.bpm());
        let (last, running) = beats_seen[beats_seen.len() - 10];
        assert!(running && last.playing, "the transport plays with the clock");
        // Over the last second, on average at the clock's tempo.
        let recent = &beats_seen[beats_seen.len() - 90..];
        let mean = recent.iter().map(|(t, _)| t.bpm).sum::<f64>() / recent.len() as f64;
        eprintln!("engine tempo over the last second: {mean:.2} BPM");
        assert!((mean - 128.0).abs() < 1.5, "the engine runs at the clock's tempo: {mean:.2}");
        // The engine's beat is on the clock's, within a few milliseconds.
        let ms = |beats: f64| beats.abs() * 60e3 / 128.0;
        let recent = &phase[phase.len() - 90..];
        let worst = recent.iter().fold(0.0f64, |m, e| m.max(ms(*e)));
        let mean = recent.iter().map(|e| ms(*e)).sum::<f64>() / recent.len() as f64;
        eprintln!("engine beat vs the clock's over the last second: mean {mean:.2} ms, worst {worst:.2} ms");
        assert!(worst < 8.0, "the beat follows the clock (worst {worst:.2} ms)");
        std::thread::sleep(Duration::from_millis(200));
        // Stop: the transport stops.
        let mut buf = vec![0.0f32; 512 * 2];
        engine.render(&mut buf, 2);
        assert!(!shared.running.load(Relaxed), "Stop stops the transport");
        std::thread::sleep(Duration::from_millis(100));

        // The sink heard the board's clock: Start, then ticks at about 128 BPM.
        let log = heard.lock().unwrap().clone();
        let starts = log.iter().filter(|(s, _)| *s == 0xFA).count();
        let ticks: Vec<u64> = log.iter().filter(|(s, _)| *s == 0xF8).map(|(_, t)| *t).collect();
        let count = |status: u8| log.iter().filter(|(s, _)| *s == status).count();
        eprintln!("sink: {} Start, {} Continue, {} ticks, {} Stop", starts, count(0xFB), ticks.len(), count(0xFC));
        assert_eq!(starts, 1, "one Start sent");
        assert_eq!(count(0xFC), 1, "one Stop sent");
        assert!(ticks.len() > 100, "ticks sent: {}", ticks.len());
        let span = (ticks[ticks.len() - 1] - ticks[ticks.len() - 49]) as f64 / 48.0;
        let sent_bpm = 60e9 / (span * PPQ);
        eprintln!("ticks sent at {sent_bpm:.2} BPM");
        assert!((sent_bpm - 128.0).abs() < 2.0, "sent at {sent_bpm:.2} BPM");
    }

    #[test]
    fn timeline_round_trips() {
        let shared = SyncShared::new();
        let t = Timeline { beat: 3.5, bpm: 128.0, playing: true, at_ns: 99, latency_ns: 5 };
        shared.publish(t);
        assert_eq!(shared.timeline(), t);
        // Half a beat after it is heard.
        let half = t.time_of(4.0);
        assert!((t.heard_beat(half as u64) - 4.0).abs() < 1e-6);
    }
}
