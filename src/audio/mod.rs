pub mod dsp;
pub mod engine;
pub mod midi;
pub mod modules;
pub mod params;

use std::sync::Arc;
use std::sync::mpsc::{self, Sender};

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
use cpal::{FromSample, SampleFormat, SizedSample, StreamConfig};

use engine::{Command, Engine};
use params::EngineShared;

pub struct AudioInfo {
    pub device: String,
    pub sample_rate: u32,
    pub channels: u16,
    pub error: Option<String>,
}

/// Keeps the output stream alive and is the UI's channel to the engine.
pub struct AudioHandle {
    _stream: Option<cpal::Stream>,
    tx: Sender<Command>,
    pub info: AudioInfo,
    /// External MIDI, read by the MIDI nodes.
    pub midi: Arc<midi::MidiState>,
}

impl AudioHandle {
    pub fn send(&self, cmd: Command) {
        // If the device failed to open there is no receiver; the UI still works.
        let _ = self.tx.send(cmd);
    }
}

/// A handle wired to an engine that the caller drives by hand (no device).
#[cfg(test)]
pub fn offline(shared: Arc<EngineShared>, sample_rate: u32) -> (AudioHandle, Engine) {
    let (tx, rx) = mpsc::channel();
    let handle = AudioHandle {
        midi: Arc::new(midi::MidiState::new()),
        _stream: None,
        tx,
        info: AudioInfo {
            device: "offline".into(),
            sample_rate,
            channels: 2,
            error: None,
        },
    };
    (handle, Engine::new(rx, shared, sample_rate as f32))
}

pub fn start(shared: Arc<EngineShared>) -> AudioHandle {
    let (tx, rx) = mpsc::channel();
    match open_stream(rx, shared) {
        Ok((stream, info)) => AudioHandle {
            _stream: Some(stream),
            tx,
            info,
            midi: Arc::new(midi::MidiState::new()),
        },
        Err(e) => AudioHandle {
            _stream: None,
            tx,
            midi: Arc::new(midi::MidiState::new()),
            info: AudioInfo {
                device: "No device".into(),
                sample_rate: 48_000,
                channels: 0,
                error: Some(e),
            },
        },
    }
}

fn open_stream(
    rx: mpsc::Receiver<Command>,
    shared: Arc<EngineShared>,
) -> Result<(cpal::Stream, AudioInfo), String> {
    let host = cpal::default_host();
    let device = host
        .default_output_device()
        .ok_or("no default output device")?;
    let device_name = device
        .description()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| "Output".into());
    let supported = device.default_output_config().map_err(|e| e.to_string())?;
    let format = supported.sample_format();
    let config: StreamConfig = supported.into();

    let info = AudioInfo {
        device: device_name,
        sample_rate: config.sample_rate,
        channels: config.channels,
        error: None,
    };
    let engine = Engine::new(rx, shared, config.sample_rate as f32);

    let stream = match format {
        SampleFormat::F32 => build::<f32>(&device, config, engine),
        SampleFormat::F64 => build::<f64>(&device, config, engine),
        SampleFormat::I16 => build::<i16>(&device, config, engine),
        SampleFormat::I32 => build::<i32>(&device, config, engine),
        SampleFormat::U16 => build::<u16>(&device, config, engine),
        other => Err(format!("unsupported sample format {other}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok((stream, info))
}

fn build<T>(device: &cpal::Device, config: StreamConfig, mut engine: Engine) -> Result<cpal::Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = config.channels as usize;
    device
        .build_output_stream(
            config,
            move |data: &mut [T], _: &cpal::OutputCallbackInfo| engine.render(data, channels),
            |err| eprintln!("audio stream error: {err}"),
            None,
        )
        .map_err(|e| e.to_string())
}
