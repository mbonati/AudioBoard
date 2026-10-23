pub mod dsp;
pub mod engine;
pub mod fastmath;
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
}

/// The output device, kept to reopen it with another buffer size.
struct Output {
    device: cpal::Device,
    config: StreamConfig,
    format: SampleFormat,
    /// Where dropouts the device reports are counted.
    shared: Arc<EngineShared>,
}

/// Keeps the output stream alive and is the UI's channel to the engine.
pub struct AudioHandle {
    stream: Option<cpal::Stream>,
    output: Option<Output>,
    /// The engine, shared with the stream's callback so that reopening the
    /// device (new buffer size) keeps every node.
    engine: Option<Arc<Mutex<Engine>>>,
    tx: Sender<Command>,
    /// What the engine let go of, to free here (see `engine::Garbage`).
    trash: Option<mpsc::Receiver<engine::Garbage>>,
    pub info: AudioInfo,
    /// External MIDI, read by the MIDI nodes.
    pub midi: Arc<midi::MidiState>,
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
        let (Some(out), Some(engine)) = (&self.output, &self.engine) else {
            return Err("no audio device".to_owned());
        };
        // Stop the old callback before the new one starts.
        self.stream = None;
        let result = open(out, frames, engine.clone());
        let (stream, error) = match result {
            Ok(stream) => (stream, None),
            // Not accepted: back to the device's own size.
            Err(e) => (open(out, None, engine.clone())?, Some(e)),
        };
        self.stream = Some(stream);
        self.info.buffer_size = if error.is_none() { frames } else { None };
        error.map_or(Ok(()), Err)
    }
}

/// A handle wired to an engine that the caller drives by hand (no device).
#[cfg(test)]
pub fn offline(shared: Arc<EngineShared>, sample_rate: u32) -> (AudioHandle, Engine) {
    let (tx, rx) = mpsc::channel();
    let handle = AudioHandle {
        midi: Arc::new(midi::MidiState::new()),
        stream: None,
        output: None,
        engine: None,
        tx,
        trash: None,
        info: AudioInfo {
            device: "offline".into(),
            sample_rate,
            channels: 2,
            error: None,
            buffer_range: None,
            buffer_size: None,
        },
    };
    (handle, Engine::new(rx, shared, sample_rate as f32))
}

/// Opens the default output with `buffer_size` frames per callback (None:
/// the device's own size).
pub fn start(shared: Arc<EngineShared>, buffer_size: Option<u32>) -> AudioHandle {
    let (tx, rx) = mpsc::channel();
    let midi = Arc::new(midi::MidiState::new());
    let failed = |tx, error: String| AudioHandle {
        stream: None,
        output: None,
        engine: None,
        tx,
        trash: None,
        midi: midi.clone(),
        info: AudioInfo {
            device: "No device".into(),
            sample_rate: 48_000,
            channels: 0,
            error: Some(error),
            buffer_range: None,
            buffer_size: None,
        },
    };
    let (output, info) = match find_output(shared.clone()) {
        Ok(found) => found,
        Err(e) => return failed(tx, e),
    };
    let mut engine = Engine::new(rx, shared, info.sample_rate as f32);
    let trash = engine.take_trash();
    let engine = Arc::new(Mutex::new(engine));
    let mut handle =
        AudioHandle { stream: None, output: Some(output), engine: Some(engine), tx, trash, info, midi: midi.clone() };
    let opened = handle.set_buffer_size(buffer_size);
    if handle.stream.is_none() {
        return failed(handle.tx, opened.err().unwrap_or_default());
    }
    handle
}

fn find_output(shared: Arc<EngineShared>) -> Result<(Output, AudioInfo), String> {
    let host = cpal::default_host();
    let device = host.default_output_device().ok_or("no default output device")?;
    let device_name = device
        .description()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| "Output".into());
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let format = supported.sample_format();
    let buffer_range = match supported.buffer_size() {
        SupportedBufferSize::Range { min, max } => Some((*min, *max)),
        SupportedBufferSize::Unknown => None,
    };
    let config: StreamConfig = supported.into();
    let info = AudioInfo {
        device: device_name,
        sample_rate: config.sample_rate,
        channels: config.channels,
        error: None,
        buffer_range,
        buffer_size: None,
    };
    Ok((Output { device, config, format, shared }, info))
}

fn open(out: &Output, frames: Option<u32>, engine: Arc<Mutex<Engine>>) -> Result<cpal::Stream, String> {
    let mut config = out.config;
    config.buffer_size = frames.map_or(BufferSize::Default, BufferSize::Fixed);
    let stream = match out.format {
        SampleFormat::F32 => build::<f32>(&out.device, config, engine, out.shared.clone()),
        SampleFormat::F64 => build::<f64>(&out.device, config, engine, out.shared.clone()),
        SampleFormat::I16 => build::<i16>(&out.device, config, engine, out.shared.clone()),
        SampleFormat::I32 => build::<i32>(&out.device, config, engine, out.shared.clone()),
        SampleFormat::U16 => build::<u16>(&out.device, config, engine, out.shared.clone()),
        other => Err(format!("unsupported sample format {other}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

fn build<T>(
    device: &cpal::Device,
    config: StreamConfig,
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
                Ok(mut engine) => engine.render(data, channels),
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
