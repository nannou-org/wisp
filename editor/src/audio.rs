//! Editor-side audio capture and its config panel.
//!
//! `bevy_wisp`'s `audio` feature turns the [`WispAudio`](bevy_wisp::prelude)
//! resource's sample rings into a shader's `@audio`/`@audio_fft` textures; this
//! module is the other half - it captures microphone input and pushes it into
//! `WispAudio`, with a small panel (enable, input device, gain) shown for
//! shaders that declare audio inputs.
//!
//! ISF leaves the audio source and any gain/sensitivity entirely to the host,
//! so the policy here is the editor's own: capture is off until asked for, and
//! the gain defaults low so a hot mic does not blow out the visuals.
//!
//! Capture is native-only for now (via `cpal`, whose web backends do not yet
//! implement input); the web build shows the panel with capture disabled.

use bevy::prelude::*;
use bevy_egui::egui;
use bevy_wisp::prelude::WispSchema;
use bevy_wisp::schema::TextureRole;

#[cfg(not(target_arch = "wasm32"))]
use {
    bevy_wisp::prelude::WispAudio,
    std::sync::{Arc, Mutex},
};

/// The default capture gain: low, so a hot mic does not blow out the visuals.
const DEFAULT_GAIN: f32 = 0.25;

pub(crate) struct AudioPlugin;

impl Plugin for AudioPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AudioConfig>();
        #[cfg(not(target_arch = "wasm32"))]
        app.init_resource::<AudioShared>()
            .insert_non_send(AudioStream::default())
            .add_systems(Startup, enumerate_devices)
            .add_systems(Update, (manage_capture, drain_capture));
    }
}

/// Audio capture configuration, shown in the params pane for shaders that
/// declare `@audio`/`@audio_fft` inputs.
#[derive(Resource)]
pub(crate) struct AudioConfig {
    /// Whether microphone capture is running.
    enabled: bool,
    /// Linear gain applied to captured samples before they reach the shader.
    gain: f32,
    /// The most recent capture error, shown beneath the controls.
    status: Option<String>,
    /// Available input device names (native only).
    #[cfg(not(target_arch = "wasm32"))]
    devices: Vec<String>,
    /// Index into `devices` of the selected capture device.
    #[cfg(not(target_arch = "wasm32"))]
    selected: usize,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            gain: DEFAULT_GAIN,
            status: None,
            #[cfg(not(target_arch = "wasm32"))]
            devices: Vec::new(),
            #[cfg(not(target_arch = "wasm32"))]
            selected: 0,
        }
    }
}

/// Whether a schema declares any audio input texture.
pub(crate) fn schema_has_audio(schema: &WispSchema) -> bool {
    schema.textures.iter().any(|texture| {
        matches!(
            texture.role,
            TextureRole::AudioWaveform { .. } | TextureRole::AudioFft { .. }
        )
    })
}

/// The audio config section: enable toggle, input device and gain.
pub(crate) fn audio_ui(ui: &mut egui::Ui, config: &mut AudioConfig) {
    ui.strong("audio");
    #[cfg(not(target_arch = "wasm32"))]
    {
        ui.checkbox(&mut config.enabled, "capture microphone");
        let selected = config
            .devices
            .get(config.selected)
            .map(String::as_str)
            .unwrap_or("default");
        ui.add_enabled_ui(config.enabled && config.devices.len() > 1, |ui| {
            egui::ComboBox::from_label("device")
                .selected_text(selected)
                .show_ui(ui, |ui| {
                    for (index, name) in config.devices.iter().enumerate() {
                        ui.selectable_value(&mut config.selected, index, name);
                    }
                });
        });
    }
    #[cfg(target_arch = "wasm32")]
    {
        ui.add_enabled(false, egui::Checkbox::new(&mut config.enabled, "capture microphone"));
        ui.weak("microphone capture is coming soon on the web build");
    }
    ui.add(egui::Slider::new(&mut config.gain, 0.0..=4.0).text("gain"));
    if let Some(status) = &config.status {
        ui.colored_label(egui::Color32::LIGHT_RED, status);
    }
}

// ---------------------------------------------------------------------------
// Native capture (cpal)
// ---------------------------------------------------------------------------

/// Captured samples shared between the cpal callback (audio thread) and the
/// drain system (main thread).
#[cfg(not(target_arch = "wasm32"))]
#[derive(Resource, Default)]
struct AudioShared {
    /// Interleaved samples pushed by the callback, drained each frame.
    buffer: Arc<Mutex<Vec<f32>>>,
    /// The live stream's channel count, set when the stream is (re)built.
    channels: usize,
}

/// The live cpal input stream. `cpal::Stream` is `!Send`, so it lives in a
/// non-send resource and is managed only from the main thread.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Default)]
struct AudioStream {
    stream: Option<cpal::Stream>,
    /// The device name the live stream captures from, or `None` when stopped.
    active: Option<String>,
}

/// A device's display name, via cpal's structured device description.
#[cfg(not(target_arch = "wasm32"))]
fn device_name(device: &cpal::Device) -> Option<String> {
    use cpal::traits::DeviceTrait;
    device.description().ok().map(|d| d.name().to_string())
}

/// Populate the device list with the host's input devices, selecting the
/// default one.
#[cfg(not(target_arch = "wasm32"))]
fn enumerate_devices(mut config: ResMut<AudioConfig>) {
    use cpal::traits::HostTrait;
    let host = cpal::default_host();
    let default_name = host.default_input_device().as_ref().and_then(device_name);
    let devices: Vec<String> = host
        .input_devices()
        .map(|devices| devices.filter_map(|d| device_name(&d)).collect())
        .unwrap_or_default();
    config.selected = default_name
        .and_then(|name| devices.iter().position(|device| *device == name))
        .unwrap_or(0);
    config.devices = devices;
}

/// Start, stop or switch the capture stream to match the config.
#[cfg(not(target_arch = "wasm32"))]
fn manage_capture(
    mut config: ResMut<AudioConfig>,
    mut shared: ResMut<AudioShared>,
    mut stream: NonSendMut<AudioStream>,
) {
    // When enabled, capture from the selected device, falling back to the
    // host default (an empty name) when none is listed. `None` means stopped.
    let want = config
        .enabled
        .then(|| config.devices.get(config.selected).cloned().unwrap_or_default());
    if want == stream.active {
        return;
    }
    // Dropping the old stream stops capture; clear stale samples before the new
    // (possibly different-channel) stream starts filling the buffer.
    stream.stream = None;
    stream.active = None;
    shared.buffer.lock().unwrap().clear();
    let Some(name) = want else {
        return;
    };
    match build_stream(&name, &mut shared) {
        Ok(built) => {
            stream.stream = Some(built);
            stream.active = Some(name);
            config.status = None;
        }
        Err(err) => {
            // Leave capture off so a failing device is not retried every frame;
            // the user can re-enable to try again.
            config.enabled = false;
            config.status = Some(err);
        }
    }
}

/// Build and start a cpal input stream feeding the shared buffer.
#[cfg(not(target_arch = "wasm32"))]
fn build_stream(name: &str, shared: &mut AudioShared) -> Result<cpal::Stream, String> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let device = host
        .input_devices()
        .map_err(|e| e.to_string())?
        .find(|device| device_name(device).as_deref() == Some(name))
        .or_else(|| host.default_input_device())
        .ok_or_else(|| String::from("no audio input device"))?;
    let supported = device.default_input_config().map_err(|e| e.to_string())?;
    shared.channels = supported.channels() as usize;
    let sample_format = supported.sample_format();
    let config: cpal::StreamConfig = supported.into();
    let buffer = shared.buffer.clone();
    let err_fn = |err| error!("wisp editor: audio capture error: {err}");
    let stream = match sample_format {
        cpal::SampleFormat::F32 => input_stream::<f32>(&device, &config, buffer, err_fn),
        cpal::SampleFormat::I16 => input_stream::<i16>(&device, &config, buffer, err_fn),
        cpal::SampleFormat::U16 => input_stream::<u16>(&device, &config, buffer, err_fn),
        fmt => Err(format!("unsupported sample format {fmt:?}")),
    }?;
    stream.play().map_err(|e| e.to_string())?;
    Ok(stream)
}

/// Build a typed input stream that converts samples to `f32` and appends them
/// to the shared buffer.
#[cfg(not(target_arch = "wasm32"))]
fn input_stream<T>(
    device: &cpal::Device,
    config: &cpal::StreamConfig,
    buffer: Arc<Mutex<Vec<f32>>>,
    err_fn: impl FnMut(cpal::StreamError) + Send + 'static,
) -> Result<cpal::Stream, String>
where
    T: cpal::SizedSample,
    f32: cpal::FromSample<T>,
{
    use cpal::Sample;
    use cpal::traits::DeviceTrait;
    device
        .build_input_stream(
            config,
            move |data: &[T], _: &cpal::InputCallbackInfo| {
                let mut buf = buffer.lock().unwrap();
                buf.extend(data.iter().map(|&sample| f32::from_sample(sample)));
                // Bound the backlog so a stalled drain (e.g. a slow frame) can
                // never grow it without limit.
                const MAX: usize = 1 << 16;
                if buf.len() > MAX {
                    let excess = buf.len() - MAX;
                    buf.drain(..excess);
                }
            },
            err_fn,
            None,
        )
        .map_err(|e| e.to_string())
}

/// Drain the captured samples into [`WispAudio`], applying the config gain.
#[cfg(not(target_arch = "wasm32"))]
fn drain_capture(config: Res<AudioConfig>, shared: Res<AudioShared>, mut audio: ResMut<WispAudio>) {
    if !config.enabled {
        return;
    }
    let mut samples = {
        let mut buf = shared.buffer.lock().unwrap();
        if buf.is_empty() {
            return;
        }
        std::mem::take(&mut *buf)
    };
    let gain = config.gain;
    for sample in &mut samples {
        *sample *= gain;
    }
    audio.push_frames(&samples, shared.channels.max(1));
}
