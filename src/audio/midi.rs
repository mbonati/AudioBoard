//! External MIDI input. Every connected device feeds one shared state: the
//! MIDI threads write it, the MIDI nodes read it on the audio thread
//! (atomics only) and the UI reads it for learning and knob mapping.

use std::sync::atomic::{
    AtomicBool, AtomicU8, AtomicU32, Ordering::Acquire, Ordering::Relaxed, Ordering::Release,
};
use std::sync::{Arc, Mutex};

use super::params::EngineShared;
use super::sync::{ClockFollower, SyncShared, now_ns};

/// Channel slots: 1..=16, plus 0 for "omni" (the latest event on any channel).
pub const CHANNELS: usize = 17;

/// Frequency of a MIDI note (A4 = 69 = 440 Hz).
pub fn note_hz(note: f32) -> f32 {
    440.0 * ((note - 69.0) / 12.0).exp2()
}

/// The note held on a channel (last-note priority).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct NoteState {
    pub note: u8,
    /// 0..1.
    pub velocity: f32,
    pub gate: bool,
    /// Bumped on every note-on and every change of the sounding note.
    pub seq: u32,
}

pub struct MidiState {
    /// Controller values, `CHANNELS * 128`.
    cc: Box<[AtomicU8]>,
    /// Messages received per controller, `CHANNELS * 128`.
    cc_seq: Box<[AtomicU32]>,
    /// Last controller moved: channel (1..=16) << 8 | number.
    last_cc: AtomicU32,
    /// Controller messages received, for MIDI learn.
    cc_count: AtomicU32,
    note: [AtomicU8; CHANNELS],
    velocity: [AtomicU8; CHANNELS],
    gate: [AtomicBool; CHANNELS],
    note_seq: [AtomicU32; CHANNELS],
    /// Held keys per channel, in press order. Only the MIDI threads lock it.
    held: Mutex<[Vec<u8>; CHANNELS]>,
    /// Any message received, for the activity light.
    events: AtomicU32,
    /// The latest raw messages (status | data1 << 8 | data2 << 16), for
    /// plugins that want the stream itself. Readers keep their own cursor.
    ring: Box<[AtomicU32]>,
    /// Messages written to `ring` so far.
    ring_head: AtomicU32,
    /// Serialises writers (one thread per device); readers never lock.
    ring_writer: Mutex<()>,
}

/// Raw messages kept for stream readers.
const RING: usize = 1024;

impl MidiState {
    pub fn new() -> Self {
        Self {
            cc: (0..CHANNELS * 128).map(|_| AtomicU8::new(0)).collect(),
            cc_seq: (0..CHANNELS * 128).map(|_| AtomicU32::new(0)).collect(),
            last_cc: AtomicU32::new(0),
            cc_count: AtomicU32::new(0),
            note: std::array::from_fn(|_| AtomicU8::new(60)),
            velocity: std::array::from_fn(|_| AtomicU8::new(0)),
            gate: std::array::from_fn(|_| AtomicBool::new(false)),
            note_seq: std::array::from_fn(|_| AtomicU32::new(0)),
            held: Mutex::new(std::array::from_fn(|_| Vec::new())),
            events: AtomicU32::new(0),
            ring: (0..RING).map(|_| AtomicU32::new(0)).collect(),
            ring_head: AtomicU32::new(0),
            ring_writer: Mutex::new(()),
        }
    }

    /// Takes one raw MIDI message (note on/off and control change; the
    /// clock and transport only go to the stream; the rest is ignored).
    pub fn handle(&self, msg: &[u8]) {
        let Some(&status) = msg.first() else {
            return;
        };
        let data = |i: usize| msg.get(i).copied().unwrap_or(0) & 0x7F;
        if is_clock(status) {
            self.push(status, data(1), data(2));
            return;
        }
        if !(0x80..0xF0).contains(&status) {
            return;
        }
        self.events.fetch_add(1, Relaxed);
        let ch = (status & 0x0F) as usize + 1;
        self.push(status, data(1), data(2));
        match status & 0xF0 {
            0x90 if data(2) > 0 => self.note_on(ch, data(1), data(2)),
            0x80 | 0x90 => self.note_off(ch, data(1)),
            0xB0 if data(1) == 123 => self.all_notes_off(ch),
            0xB0 => {
                let (num, value) = (data(1) as usize, data(2));
                for c in [ch, 0] {
                    self.cc[c * 128 + num].store(value, Relaxed);
                    self.cc_seq[c * 128 + num].fetch_add(1, Relaxed);
                }
                self.last_cc.store(((ch as u32) << 8) | num as u32, Relaxed);
                self.cc_count.fetch_add(1, Relaxed);
            }
            _ => {}
        }
    }

    /// Onto the stream the MIDI nodes read.
    fn push(&self, status: u8, d1: u8, d2: u8) {
        let _writer = self.ring_writer.lock().unwrap_or_else(|e| e.into_inner());
        let head = self.ring_head.load(Relaxed);
        let packed = status as u32 | (d1 as u32) << 8 | (d2 as u32) << 16;
        self.ring[head as usize % RING].store(packed, Relaxed);
        self.ring_head.store(head.wrapping_add(1), Release);
    }

    fn held(&self) -> std::sync::MutexGuard<'_, [Vec<u8>; CHANNELS]> {
        self.held.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn note_on(&self, ch: usize, note: u8, velocity: u8) {
        let mut held = self.held();
        for c in [ch, 0] {
            held[c].retain(|n| *n != note);
            held[c].push(note);
            self.note[c].store(note, Relaxed);
            self.velocity[c].store(velocity, Relaxed);
            self.gate[c].store(true, Relaxed);
            self.note_seq[c].fetch_add(1, Relaxed);
        }
    }

    fn note_off(&self, ch: usize, note: u8) {
        let mut held = self.held();
        for c in [ch, 0] {
            held[c].retain(|n| *n != note);
            match held[c].last() {
                // Fall back to the key still held (last-note priority).
                Some(&n) if n != self.note[c].load(Relaxed) => {
                    self.note[c].store(n, Relaxed);
                    self.note_seq[c].fetch_add(1, Relaxed);
                }
                Some(_) => {}
                None => self.gate[c].store(false, Relaxed),
            }
        }
    }

    fn all_notes_off(&self, ch: usize) {
        let mut held = self.held();
        for c in [ch, 0] {
            held[c].clear();
            self.gate[c].store(false, Relaxed);
        }
    }

    /// Controller value (0..1) and how many messages it has received.
    pub fn cc(&self, channel: usize, number: usize) -> (f32, u32) {
        let i = channel.min(CHANNELS - 1) * 128 + number.min(127);
        (
            self.cc[i].load(Relaxed) as f32 / 127.0,
            self.cc_seq[i].load(Relaxed),
        )
    }

    pub fn note(&self, channel: usize) -> NoteState {
        let c = channel.min(CHANNELS - 1);
        NoteState {
            note: self.note[c].load(Relaxed),
            velocity: self.velocity[c].load(Relaxed) as f32 / 127.0,
            gate: self.gate[c].load(Relaxed),
            seq: self.note_seq[c].load(Relaxed),
        }
    }

    /// (messages so far, channel 1..=16, number) of the last controller moved.
    pub fn last_cc(&self) -> (u32, usize, usize) {
        let packed = self.last_cc.load(Relaxed);
        (
            self.cc_count.load(Relaxed),
            (packed >> 8) as usize,
            (packed & 0x7F) as usize,
        )
    }

    pub fn events(&self) -> u32 {
        self.events.load(Relaxed)
    }

    /// Where the message stream is now: a reader starting here sees only
    /// newer messages.
    pub fn stream_head(&self) -> u32 {
        self.ring_head.load(Acquire)
    }

    /// Calls `f` with each message since `cursor` (status, data1, data2) and
    /// moves the cursor on. A reader that fell more than the ring behind
    /// skips to the oldest message still kept. Lock-free.
    pub fn read_stream(&self, cursor: &mut u32, mut f: impl FnMut([u8; 3])) {
        let head = self.ring_head.load(Acquire);
        if head.wrapping_sub(*cursor) > RING as u32 {
            *cursor = head.wrapping_sub(RING as u32);
        }
        while *cursor != head {
            let packed = self.ring[*cursor as usize % RING].load(Relaxed);
            f([packed as u8, (packed >> 8) as u8, (packed >> 16) as u8]);
            *cursor = cursor.wrapping_add(1);
        }
    }
}

/// The clock and transport: timing clock, Start, Continue, Stop and the
/// song position. The MIDI Looper follows them from the stream; the board's
/// transport follows them from one chosen input (see `sync`).
pub fn is_clock(status: u8) -> bool {
    matches!(status, 0xF2 | 0xF8 | 0xFA | 0xFB | 0xFC)
}

/// One connected MIDI input.
struct Input {
    name: String,
    /// Unique while the app runs: the sync source is picked by it.
    id: u32,
    _conn: midir::MidiInputConnection<()>,
    /// Messages received: notes and controllers, and clock and transport.
    events: Arc<AtomicU32>,
    clock: Arc<AtomicU32>,
}

/// A MIDI input as the settings window shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct InputActivity {
    pub name: String,
    /// Messages so far (notes, controllers…), and clock messages so far.
    pub events: u32,
    pub clock: u32,
}

/// Open connections to every MIDI input on the system.
pub struct MidiInputs {
    connections: Vec<Input>,
    next_id: u32,
    /// The input followed as the transport's clock (by name), if any.
    sync_source: Option<String>,
    pub error: Option<String>,
}

impl MidiInputs {
    pub fn new() -> Self {
        Self {
            connections: Vec::new(),
            next_id: 1,
            sync_source: None,
            error: None,
        }
    }

    pub fn names(&self) -> impl Iterator<Item = &str> {
        self.connections.iter().map(|c| c.name.as_str())
    }

    /// Every connected input with its message counts.
    pub fn activity(&self) -> Vec<InputActivity> {
        self.connections
            .iter()
            .map(|c| InputActivity {
                name: c.name.clone(),
                events: c.events.load(Relaxed),
                clock: c.clock.load(Relaxed),
            })
            .collect()
    }

    /// Follows the clock of the input named `source` (None: the board's
    /// own transport). An input that isn't connected is followed once it is.
    pub fn set_sync_source(&mut self, source: Option<String>, sync: &SyncShared) {
        self.sync_source = source;
        self.update_source(sync);
    }

    fn update_source(&self, sync: &SyncShared) {
        let id = self
            .sync_source
            .as_ref()
            .and_then(|s| self.connections.iter().find(|c| c.name == *s))
            .map_or(0, |c| c.id);
        sync.source.store(id, Relaxed);
        sync.follow.store(self.sync_source.is_some(), Relaxed);
    }

    /// Connects to inputs that appeared and drops those that went away.
    /// `wake` runs on the MIDI thread after each message (not the clock).
    pub fn rescan(
        &mut self,
        state: &Arc<MidiState>,
        shared: &Arc<EngineShared>,
        wake: impl Fn() + Send + Clone + 'static,
    ) {
        let probe = match midir::MidiInput::new("AudioBoard") {
            Ok(m) => m,
            Err(e) => {
                self.error = Some(e.to_string());
                return;
            }
        };
        self.error = None;
        let present: Vec<String> = probe
            .ports()
            .iter()
            .filter_map(|p| probe.port_name(p).ok())
            .collect();
        self.connections.retain(|c| present.contains(&c.name));

        for name in present {
            if self.connections.iter().any(|c| c.name == name) {
                continue;
            }
            let Ok(mut input) = midir::MidiInput::new("AudioBoard") else {
                continue;
            };
            // The clock and transport come through (for the looper and sync).
            input.ignore(midir::Ignore::SysexAndActiveSense);
            let Some(port) = input
                .ports()
                .into_iter()
                .find(|p| input.port_name(p).ok().as_ref() == Some(&name))
            else {
                continue;
            };
            let id = self.next_id;
            let (events, clock) = (Arc::new(AtomicU32::new(0)), Arc::new(AtomicU32::new(0)));
            let (state, shared, wake) = (state.clone(), shared.clone(), wake.clone());
            let (events_in, clock_in) = (events.clone(), clock.clone());
            let mut follower = ClockFollower::new();
            let callback = move |_: u64, msg: &[u8], _: &mut ()| {
                state.handle(msg);
                if msg.first().is_some_and(|&s| is_clock(s)) {
                    clock_in.fetch_add(1, Relaxed);
                    if shared.sync.source.load(Relaxed) == id {
                        follower.handle(msg, now_ns(), &shared.sync);
                    }
                    // A clock ticks 48 times a second at 120 BPM: no reason
                    // to redraw.
                } else {
                    events_in.fetch_add(1, Relaxed);
                    wake();
                }
            };
            if let Ok(conn) = input.connect(&port, "AudioBoard in", callback, ()) {
                self.next_id += 1;
                self.connections.push(Input {
                    name,
                    id,
                    _conn: conn,
                    events,
                    clock,
                });
            }
        }
        self.update_source(&shared.sync);
    }
}
