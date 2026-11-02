pub mod device;
pub mod dsp;
pub mod engine;
pub mod fastmath;
pub mod input;
pub mod midi;
pub mod modules;
pub mod params;
pub mod plugin;
pub mod record;
pub mod export;

use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{BufferSize, FromSample, SampleFormat, SizedSample, StreamConfig, SupportedBufferSize};

use device::{AudioConfig, InputChoice};
use engine::{Command, Engine};
use params::EngineShared;

pub struct AudioInfo {
    pub device: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub error: Option<String>,
    /// Buffer sizes the device accepts, in frames, if it says.
    pub buffer_range: Option<(u32, u32)>,
    /// The buffer size asked for (None: the device's own).
    pub buffer_size: Option<u32>,
    /// The first channel of the pair the board plays on (0-based).
    pub output_channel: u16,
    /// The input while it is open: its name and channels.
    pub input: Option<String>,
    pub input_channels: u16,
    /// Why the input couldn't open.
    pub input_error: Option<String>,
}

impl AudioInfo {
    fn none(error: Option<String>) -> Self {
        Self {
            device: "No device".into(),
            sample_rate: 48_000,
            channels: 0,
            error,
            buffer_range: None,
            buffer_size: None,
            output_channel: 0,
            input: None,
            input_channels: 0,
            input_error: None,
        }
    }
}

/// The output device as opened, kept to open it again.
struct Output {
    device: cpal::Device,
    config: StreamConfig,
    format: SampleFormat,
    /// The first channel of the board's pair.
    first: usize,
}

/// Keeps the audio streams alive and is the UI's channel to the engine.
pub struct AudioHandle {
    stream: Option<cpal::Stream>,
    input_stream: Option<cpal::Stream>,
    output: Option<Output>,
    /// The engine, shared with the stream's callback so that reopening the
    /// device (new buffer size, another device at the same rate) keeps
    /// every node.
    engine: Option<Arc<Mutex<Engine>>>,
    tx: Sender<Command>,
    /// What the engine let go of, to free here (see `engine::Garbage`).
    trash: Option<mpsc::Receiver<engine::Garbage>>,
    pub shared: Arc<EngineShared>,
    pub info: AudioInfo,
    /// External MIDI, read by the MIDI nodes.
    pub midi: Arc<midi::MidiState>,
    /// The setup asked for (what runs is in `info`).
    pub config: AudioConfig,
    /// Bumped whenever the engine is replaced (a new sample rate): every
    /// node has to be built again, for the new rate.
    pub generation: u32,
    /// Some Audio In node (or the settings window) is listening.
    input_wanted: bool,
}

impl AudioHandle {
    pub fn send(&self, cmd: Command) {
        // If the device failed to open there is no receiver; the UI still works.
        let _ = self.tx.send(cmd);
    }

    /// Frees what the engine let go of (call often, from the UI thread).
    pub fn collect_garbage(&self) {
        if let Some(trash) = &self.trash {
            while trash.try_recv().is_ok() {}
        }
    }

    /// Reopens the output with another buffer size (None: the device's
    /// own). Larger buffers give heavy patches and plugins more headroom,
    /// at the cost of latency. The engine and its nodes carry over.
    pub fn set_buffer_size(&mut self, frames: Option<u32>) -> Result<(), String> {
        self.apply(AudioConfig { buffer_size: frames, ..self.config.clone() })
    }

    /// Switches to another setup. If it can't run, the previous one comes
    /// back and the error is returned; a buffer size the device refuses
    /// falls back to its own (also an error, the rest applied). A new
    /// sample rate builds a new engine: see `generation`.
    pub fn apply(&mut self, config: AudioConfig) -> Result<(), String> {
        let previous = std::mem::replace(&mut self.config, config.clone());
        // Only the input changed: the output keeps playing.
        if self.stream.is_some() && (AudioConfig { input: previous.input.clone(), ..config.clone() }) == previous {
            if self.input_wanted {
                self.open_input();
            }
            return Ok(());
        }
        match self.reopen() {
            Ok(None) => Ok(()),
            Ok(Some(warning)) => Err(warning),
            Err(e) => {
                if previous != config {
                    self.config = previous;
                    if self.reopen().is_ok() {
                        return Err(e);
                    }
                }
                self.info.error = Some(e.clone());
                Err(e)
            }
        }
    }

    /// Opens the output (and the input, if wanted) as `config` says.
    /// Ok(Some): running, with something not as asked.
    fn reopen(&mut self) -> Result<Option<String>, String> {
        let config = self.config.clone();
        let host = device::host(config.host.as_deref());
        let device = config
            .output
            .as_deref()
            .and_then(|id| device::find(&host, id))
            .or_else(|| host.default_output_device())
            .ok_or("no output device")?;
        let supported = device.default_output_config().map_err(|e| e.to_string())?;
        let format = supported.sample_format();
        let buffer_range = match supported.buffer_size() {
            SupportedBufferSize::Range { min, max } => Some((*min, *max)),
            SupportedBufferSize::Unknown => None,
        };
        let mut stream_config = supported.config();
        let mut warning = None;
        if let Some(rate) = config.sample_rate.filter(|&r| r != stream_config.sample_rate) {
            let channels = stream_config.channels;
            let ok = device
                .supported_output_configs()
                .is_ok_and(|mut ranges| ranges.any(|r| r.channels() == channels && (r.min_sample_rate()..=r.max_sample_rate()).contains(&rate)));
            if ok {
                stream_config.sample_rate = rate;
            } else {
                warning = Some(format!("{} doesn't run at {rate} Hz", device::name_of(&device)));
                self.config.sample_rate = None;
            }
        }

        // Stop what plays now: the old callbacks end before new ones start.
        self.close_input();
        self.stream = None;
        self.output = None;
        let rate = stream_config.sample_rate;
        if self.engine.is_none() || self.info.sample_rate != rate {
            let (tx, rx) = mpsc::channel();
            let mut engine = Engine::new(rx, self.shared.clone(), rate as f32);
            self.trash = engine.take_trash();
            self.tx = tx;
            self.engine = Some(Arc::new(Mutex::new(engine)));
            self.generation += 1;
        }
        let engine = self.engine.clone().ok_or("no engine")?;
        let channels = stream_config.channels;
        let first = if channels > 2 { config.output_channel.min(channels - 2) } else { 0 };
        let output = Output { device, config: stream_config, format, first: first as usize };
        let stream = match open(&output, config.buffer_size, engine.clone(), self.shared.clone()) {
            Ok(stream) => stream,
            // Not accepted: the device's own size.
            Err(e) if config.buffer_size.is_some() => {
                warning = Some(format!("buffer size not accepted: {e}"));
                self.config.buffer_size = None;
                open(&output, None, engine, self.shared.clone())?
            }
            Err(e) => return Err(e),
        };
        self.stream = Some(stream);
        self.info = AudioInfo {
            device: device::name_of(&output.device),
            sample_rate: rate,
            channels,
            error: None,
            buffer_range,
            buffer_size: self.config.buffer_size,
            output_channel: first,
            ..AudioInfo::none(None)
        };
        self.output = Some(output);
        if self.input_wanted {
            self.open_input();
        }
        Ok(warning)
    }

    /// Opens the input while something listens, closes it when nothing
    /// does (call every frame; acts on changes).
    pub fn want_input(&mut self, wanted: bool) {
        if wanted != self.input_wanted {
            self.input_wanted = wanted;
            if wanted {
                self.open_input();
            } else {
                self.close_input();
            }
        }
    }

    fn close_input(&mut self) {
        self.input_stream = None;
        self.shared.input.close();
        self.info.input = None;
        self.info.input_channels = 0;
        self.info.input_error = None;
    }

    fn open_input(&mut self) {
        self.close_input();
        let id = match &self.config.input {
            InputChoice::Off => return,
            InputChoice::SystemDefault => None,
            InputChoice::Device(id) => Some(id.as_str()),
        };
        if self.stream.is_none() {
            return;
        }
        let host = device::host(self.config.host.as_deref());
        let Some(device) = id.and_then(|id| device::find(&host, id)).or_else(|| host.default_input_device()) else {
            self.info.input_error = Some("no input device".into());
            return;
        };
        match open_input(&device, self.info.sample_rate, self.config.buffer_size, self.shared.clone()) {
            Ok((stream, channels)) => {
                self.input_stream = Some(stream);
                self.info.input = Some(device::name_of(&device));
                self.info.input_channels = channels;
            }
            Err(e) => self.info.input_error = Some(e),
        }
    }
}

/// A handle wired to an engine that the caller drives by hand (no device).
#[cfg(test)]
pub fn offline(shared: Arc<EngineShared>, sample_rate: u32) -> (AudioHandle, Engine) {
    let (tx, rx) = mpsc::channel();
    let handle = AudioHandle {
        midi: Arc::new(midi::MidiState::new()),
        stream: None,
        input_stream: None,
        output: None,
        engine: None,
        tx,
        trash: None,
        shared: shared.clone(),
        info: AudioInfo { device: "offline".into(), sample_rate, channels: 2, ..AudioInfo::none(None) },
        config: AudioConfig::default(),
        generation: 1,
        input_wanted: false,
    };
    (handle, Engine::new(rx, shared, sample_rate as f32))
}

/// Opens the audio as `config` says (falling back to the system's default
/// output if its device is gone).
pub fn start(shared: Arc<EngineShared>, config: AudioConfig) -> AudioHandle {
    let (tx, _) = mpsc::channel();
    let mut handle = AudioHandle {
        stream: None,
        input_stream: None,
        output: None,
        engine: None,
        tx,
        trash: None,
        shared,
        info: AudioInfo::none(None),
        midi: Arc::new(midi::MidiState::new()),
        config: config.clone(),
        generation: 0,
        input_wanted: false,
    };
    if let Err(e) = handle.apply(config) {
        if handle.stream.is_none() {
            handle.info = AudioInfo::none(Some(e));
        } else {
            handle.info.error = None;
        }
    }
    handle
}

fn open(out: &Output, frames: Option<u32>, engine: Arc<Mutex<Engine>>, shared: Arc<EngineShared>) -> Result<cpal::Stream, String> {
    let mut config = out.config;
    config.buffer_size = frames.map_or(BufferSize::Default, BufferSize::Fixed);
    let first = out.first;
    let stream = match out.format {
        SampleFormat::F32 => build::<f32>(&out.device, config, first, engine, shared),
        SampleFormat::F64 => build::<f64>(&out.device, config, first, engine, shared),
        SampleFormat::I16 => build::<i16>(&out.device, config, first, engine, shared),
        SampleFormat::I32 => build::<i32>(&out.device, config, first, engine, shared),
        SampleFormat::U16 => build::<u16>(&out.device, config, first, engine, shared),
        other => Err(format!("unsupported sample format {other}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

fn build<T>(
    device: &cpal::Device,
    config: StreamConfig,
    first: usize,
    engine: Arc<Mutex<Engine>>,
    shared: Arc<EngineShared>,
) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    device
        .build_output_stream(
            config,
            // Only this callback uses the engine (the previous stream is
            // gone before this one starts), so the lock is never contended.
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| match engine.try_lock() {
                Ok(mut engine) => engine.render_at(data, channels, first),
                Err(_) => data.fill(T::EQUILIBRIUM),
            },
            // Underruns are counted (the status bar shows them), not printed.
            move |err: cpal::Error| {
                if err.kind() == cpal::ErrorKind::Xrun {
                    shared.overloads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                } else {
                    eprintln!("audio stream error: {err}");
                }
            },
            None,
        )
        .map_err(|e| e.to_string())
}

/// Opens `device`'s input at the output's `rate`; returns the stream and
/// its channels.
fn open_input(device: &cpal::Device, rate: u32, frames: Option<u32>, shared: Arc<EngineShared>) -> Result<(cpal::Stream, u16), String> {
    let channels = device.default_input_config().map_err(|e| e.to_string())?.channels();
    // Any sample format the device has at this rate, f32 if it can.
    let format = device
        .supported_input_configs()
        .map_err(|e| e.to_string())?
        .filter(|r| r.channels() == channels && (r.min_sample_rate()..=r.max_sample_rate()).contains(&rate))
        .map(|r| r.sample_format())
        .max_by_key(|f| *f == SampleFormat::F32)
        .ok_or_else(|| format!("{} can't run at {rate} Hz, the output's rate", device::name_of(device)))?;
    let config = |frames: Option<u32>| StreamConfig {
        channels,
        sample_rate: rate,
        buffer_size: frames.map_or(BufferSize::Default, BufferSize::Fixed),
    };
    let build = |frames| match format {
        SampleFormat::F32 => build_input::<f32>(device, config(frames), shared.clone()),
        SampleFormat::F64 => build_input::<f64>(device, config(frames), shared.clone()),
        SampleFormat::I16 => build_input::<i16>(device, config(frames), shared.clone()),
        SampleFormat::I32 => build_input::<i32>(device, config(frames), shared.clone()),
        SampleFormat::U16 => build_input::<u16>(device, config(frames), shared.clone()),
        other => Err(format!("unsupported sample format {other}")),
    };
    // The output's buffer size if the input takes it, else its own.
    let stream = build(frames).or_else(|e| if frames.is_some() { build(None) } else { Err(e) })?;
    shared.input.open(channels as usize);
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, channels))
}

fn build_input<T>(device: &cpal::Device, config: StreamConfig, shared: Arc<EngineShared>) -> Result<cpal::Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = config.channels as usize;
    let errors = shared.clone();
    device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| shared.input.push(data, channels),
            move |err: cpal::Error| {
                if err.kind() == cpal::ErrorKind::Xrun {
                    errors.input.dropouts.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                } else {
                    eprintln!("audio input error: {err}");
                }
            },
            None,
        )
        .map_err(|e| e.to_string())
}
