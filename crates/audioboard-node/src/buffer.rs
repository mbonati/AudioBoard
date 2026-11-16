//! What flows through a patch: blocks of stereo audio ([`Buf`]), blocks of
//! MIDI ([`MidiBuf`]), and the transport a node runs under ([`ProcessCtx`]).

/// Frames per block: nodes run block by block. Part of the plugin ABI: a
/// plugin built for another block size is refused.
pub const BLOCK: usize = 64;

/// One block of stereo audio, `[left, right]`. Every cable carries stereo;
/// mono sources write the same signal to both channels.
pub type Buf = [[f32; BLOCK]; 2];

/// Events a MIDI cable can hold per block (more are dropped).
pub const MIDI_CAP: usize = 128;

/// A raw MIDI message at a frame within the block.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct MidiMsg {
    pub frame: u16,
    pub data: [u8; 3],
}

impl MidiMsg {
    /// The status nibble (0x90 note on, 0x80 note off, 0xB0 CC…).
    pub fn kind(&self) -> u8 {
        self.data[0] & 0xF0
    }

    /// The channel, 0–15.
    pub fn channel(&self) -> u8 {
        self.data[0] & 0x0F
    }

    /// A note on with a velocity: (note, velocity 0–127).
    pub fn note_on(&self) -> Option<(u8, u8)> {
        (self.kind() == 0x90 && self.data[2] > 0).then_some((self.data[1], self.data[2]))
    }

    /// A note off (or a note on at velocity 0): the note.
    pub fn note_off(&self) -> Option<u8> {
        (self.kind() == 0x80 || (self.kind() == 0x90 && self.data[2] == 0)).then_some(self.data[1])
    }
}

/// One block of MIDI, fixed size so the audio thread never allocates.
#[derive(Clone)]
pub struct MidiBuf {
    events: [MidiMsg; MIDI_CAP],
    len: usize,
}

impl Default for MidiBuf {
    fn default() -> Self {
        Self { events: [MidiMsg::default(); MIDI_CAP], len: 0 }
    }
}

impl MidiBuf {
    pub fn clear(&mut self) {
        self.len = 0;
    }

    pub fn push(&mut self, frame: usize, data: [u8; 3]) {
        if self.len < MIDI_CAP {
            self.events[self.len] = MidiMsg { frame: frame as u16, data };
            self.len += 1;
        }
    }

    pub fn events(&self) -> &[MidiMsg] {
        &self.events[..self.len]
    }

    pub fn copy_from(&mut self, other: &MidiBuf) {
        self.len = other.len;
        self.events[..other.len].copy_from_slice(other.events());
    }

    /// All the room, to be filled from the start; then [`Self::set_len`].
    pub fn room(&mut self) -> &mut [MidiMsg; MIDI_CAP] {
        &mut self.events
    }

    /// Keeps the first `len` events (at most [`MIDI_CAP`]).
    pub fn set_len(&mut self, len: usize) {
        self.len = len.min(MIDI_CAP);
    }
}

/// The transport and timing a block is processed under.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ProcessCtx {
    pub sample_rate: f32,
    /// One-pole coefficient for ~10 ms parameter smoothing.
    pub smooth: f32,
    /// Transport position at the start of the block, in beats. Advances only
    /// while audio runs, so every rhythmic node shares one clock.
    pub beat: f64,
    /// Beats per sample at the current tempo.
    pub beats_per_sample: f64,
    /// The transport is playing. Stopped, sources fall silent and rhythm
    /// generators end their notes, while effects keep processing their
    /// tails; the beat stands still.
    pub playing: bool,
    /// The transport was just rewound to beat 0 (Stop, then Start): rhythm
    /// generators start over and end the notes they were holding.
    pub restart: bool,
}

impl ProcessCtx {
    /// The tempo, in beats per minute.
    pub fn tempo(&self) -> f64 {
        self.beats_per_sample * self.sample_rate as f64 * 60.0
    }

    /// The beat at frame `i` of the block.
    pub fn beat_at(&self, i: usize) -> f64 {
        self.beat + self.beats_per_sample * i as f64
    }
}
