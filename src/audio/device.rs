//! Audio devices: which output and input the board uses, at what sample
//! rate and buffer size (kept in the app's settings), and what the system
//! offers to choose from.

use cpal::traits::{DeviceTrait, HostTrait};
use cpal::{Device, DeviceId, Host, SupportedBufferSize};
use serde::{Deserialize, Serialize};

/// Sample rates offered, where the device supports them.
pub const RATES: [u32; 6] = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];

/// Buffer sizes offered, in frames, where the device accepts them.
pub const BUFFER_SIZES: [u32; 6] = [64, 128, 256, 512, 1024, 2048];

/// The board's audio setup. Devices are kept by id, stable across launches;
/// one that is missing falls back to the system's default.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub struct AudioConfig {
    /// The audio system ("CoreAudio", "JACK"…); None: the platform's own.
    #[serde(default)]
    pub host: Option<String>,
    /// The output device; None: the system default.
    #[serde(default)]
    pub output: Option<String>,
    /// First channel (0-based) of the pair the board plays on.
    #[serde(default)]
    pub output_channel: u16,
    #[serde(default)]
    pub input: InputChoice,
    /// None: the device's current rate.
    #[serde(default)]
    pub sample_rate: Option<u32>,
    /// Frames per buffer; None: the device's own.
    #[serde(default)]
    pub buffer_size: Option<u32>,
}

/// Where Audio In nodes listen. The input is opened only while a node (or
/// the settings window) needs it.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Default)]
pub enum InputChoice {
    Off,
    #[default]
    SystemDefault,
    /// A device id.
    Device(String),
}

/// A device as the settings window shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub channels: u16,
    /// Of [`RATES`], those it supports.
    pub rates: Vec<u32>,
    /// Buffer sizes it accepts, in frames, if it says.
    pub buffer_range: Option<(u32, u32)>,
}

/// What the system offers.
#[derive(Clone, Debug, Default)]
pub struct DeviceList {
    /// Audio systems on this platform, and the one listed.
    pub hosts: Vec<String>,
    pub host: String,
    pub outputs: Vec<DeviceInfo>,
    pub inputs: Vec<DeviceInfo>,
    /// The system defaults' ids.
    pub default_output: Option<String>,
    pub default_input: Option<String>,
}

impl DeviceList {
    pub fn output(&self, id: Option<&str>) -> Option<&DeviceInfo> {
        let id = id.or(self.default_output.as_deref())?;
        self.outputs.iter().find(|d| d.id == id)
    }

    pub fn input(&self, choice: &InputChoice) -> Option<&DeviceInfo> {
        let id = match choice {
            InputChoice::Off => return None,
            InputChoice::SystemDefault => self.default_input.as_deref()?,
            InputChoice::Device(id) => id,
        };
        self.inputs.iter().find(|d| d.id == id)
    }
}

/// The audio system named `name`, or the platform's default.
pub fn host(name: Option<&str>) -> Host {
    name.and_then(|n| cpal::available_hosts().into_iter().find(|h| h.name() == n))
        .and_then(|id| cpal::host_from_id(id).ok())
        .unwrap_or_else(cpal::default_host)
}

/// A device by its id.
pub fn find(host: &Host, id: &str) -> Option<Device> {
    let id: DeviceId = id.parse().ok()?;
    host.device_by_id(&id)
}

pub fn id_of(device: &Device) -> Option<String> {
    device.id().ok().map(|id| id.to_string())
}

pub fn name_of(device: &Device) -> String {
    device.description().map(|d| d.name().to_owned()).unwrap_or_else(|_| "Audio device".into())
}

/// Lists the devices of the configured audio system (slow-ish: call when
/// the settings window opens or on Refresh, not every frame).
pub fn list(config: &AudioConfig) -> DeviceList {
    let host = host(config.host.as_deref());
    let mut list = DeviceList {
        hosts: cpal::available_hosts().iter().map(|h| h.name().to_owned()).collect(),
        host: host.id().name().to_owned(),
        default_output: host.default_output_device().as_ref().and_then(id_of),
        default_input: host.default_input_device().as_ref().and_then(id_of),
        ..Default::default()
    };
    if let Ok(devices) = host.devices() {
        for device in devices {
            if let Some(info) = describe(&device, false) {
                list.outputs.push(info);
            }
            if let Some(info) = describe(&device, true) {
                list.inputs.push(info);
            }
        }
    }
    list
}

/// One side of a device (None: it has no such side).
fn describe(device: &Device, input: bool) -> Option<DeviceInfo> {
    let default = if input { device.default_input_config() } else { device.default_output_config() }.ok()?;
    let ranges: Vec<_> = if input {
        device.supported_input_configs().ok()?.collect()
    } else {
        device.supported_output_configs().ok()?.collect()
    };
    // The stream opens with the default layout: all its channels.
    let channels = default.channels();
    let rates = RATES
        .into_iter()
        .filter(|&rate| ranges.iter().any(|r| r.channels() == channels && (r.min_sample_rate()..=r.max_sample_rate()).contains(&rate)))
        .collect();
    Some(DeviceInfo {
        id: id_of(device)?,
        name: name_of(device),
        channels,
        rates,
        buffer_range: match default.buffer_size() {
            SupportedBufferSize::Range { min, max } => Some((*min, *max)),
            SupportedBufferSize::Unknown => None,
        },
    })
}

/// "1–2", "3–4"…: the stereo pairs of `channels` (a lone last channel too).
pub fn pairs(channels: u16) -> Vec<(u16, String)> {
    (0..channels.max(1))
        .step_by(2)
        .map(|c| if c + 1 < channels { (c, format!("{}–{}", c + 1, c + 2)) } else { (c, format!("{}", c + 1)) })
        .collect()
}
