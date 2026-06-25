//! Audio input textures (feature `audio`).
//!
//! Feed sample frames into the [`WispAudio`] resource from any source (a
//! `cpal` capture stream, a decoded file, synthesis) and wisp keeps the
//! shader's `@audio`/`@audio_fft` textures up to date each frame:
//!
//! - `@audio(samples = ..)` waveform textures hold the most recent samples in
//!   `[-1, 1]`, one row per channel.
//! - `@audio_fft(bins = ..)` textures hold Hann-windowed linear FFT magnitudes
//!   (normalized so a full-scale sine peaks near 1.0), one row per channel.
//!
//! Textures are `r16float`, so read the `.r` channel, e.g.
//! `textureSample(waveform, samp, vec2<f32>(uv.x, 0.5)).r`.

use crate::asset::{Wisp, WispHandle};
use crate::inputs::{WispInputs, WispValue};
use crate::schema::TextureRole;
use bevy_asset::RenderAssetUsages;
use bevy_asset::prelude::Assets;
use bevy_ecs::prelude::*;
use bevy_image::prelude::Image;
use bevy_platform::collections::HashMap;
use bevy_render::render_resource::{Extent3d, TextureDimension, TextureFormat, TextureUsages};
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};
use std::collections::VecDeque;
use std::sync::Arc;

/// Per-channel sample history: enough for the largest supported FFT window.
const RING_CAPACITY: usize = 1 << 14;

/// Recent audio sample frames, fed by the user from any source via
/// [`WispAudio::push_frames`].
#[derive(Resource, Default)]
pub struct WispAudio {
    rings: Vec<VecDeque<f32>>,
    ffts: HashMap<usize, Arc<dyn Fft<f32>>>,
}

impl WispAudio {
    /// Append interleaved sample frames (`channels` samples per frame).
    pub fn push_frames(&mut self, interleaved: &[f32], channels: usize) {
        let channels = channels.max(1);
        if self.rings.len() != channels {
            self.rings = vec![VecDeque::with_capacity(RING_CAPACITY); channels];
        }
        for frame in interleaved.chunks(channels) {
            for (ring, sample) in self.rings.iter_mut().zip(frame) {
                if ring.len() == RING_CAPACITY {
                    ring.pop_front();
                }
                ring.push_back(*sample);
            }
        }
    }

    pub fn channels(&self) -> usize {
        self.rings.len().max(1)
    }

    /// The most recent `samples` samples per channel, zero-padded at the front,
    /// one row per channel.
    fn waveform(&self, samples: usize) -> Vec<f32> {
        let mut rows = vec![0.0; samples * self.channels()];
        for (channel, row) in rows.chunks_mut(samples).enumerate() {
            let Some(ring) = self.rings.get(channel) else {
                continue;
            };
            let take = ring.len().min(samples);
            let recent = ring.iter().skip(ring.len() - take);
            for (slot, sample) in row[samples - take..].iter_mut().zip(recent) {
                *slot = *sample;
            }
        }
        rows
    }

    /// Hann-windowed linear FFT magnitudes, `bins` per channel, one row per
    /// channel. The window is `2 * bins` samples.
    fn fft(&mut self, bins: usize) -> Vec<f32> {
        let window = (bins * 2).clamp(2, RING_CAPACITY);
        let fft = self
            .ffts
            .entry(window)
            .or_insert_with(|| FftPlanner::new().plan_fft_forward(window))
            .clone();
        let channels = self.channels();
        let samples = self.waveform(window);
        let mut rows = vec![0.0; bins * channels];
        let mut buffer = vec![Complex::default(); window];
        for channel in 0..channels {
            let input = &samples[channel * window..(channel + 1) * window];
            for (index, (slot, sample)) in buffer.iter_mut().zip(input).enumerate() {
                let phase = std::f32::consts::TAU * index as f32 / window as f32;
                let hann = 0.5 * (1.0 - phase.cos());
                *slot = Complex::new(sample * hann, 0.0);
            }
            fft.process(&mut buffer);
            let row = &mut rows[channel * bins..(channel + 1) * bins];
            for (slot, value) in row.iter_mut().zip(&buffer) {
                // 2/window normalizes a full-scale sine to ~1.0 (the factor 2
                // compensates the Hann window's 0.5 mean).
                *slot = value.norm() * 2.0 / window as f32;
            }
        }
        rows
    }
}

/// Keep every wisp camera's `@audio`/`@audio_fft` textures up to date.
pub(crate) fn update_audio_textures(
    mut audio: ResMut<WispAudio>,
    wisps: Res<Assets<Wisp>>,
    mut images: ResMut<Assets<Image>>,
    mut cameras: Query<(&WispHandle, &mut WispInputs)>,
) {
    for (wisp, mut inputs) in cameras.iter_mut() {
        let Some(wisp) = wisps.get(&**wisp) else {
            continue;
        };
        for texture in &wisp.schema.textures {
            let (width, data) = match texture.role {
                TextureRole::AudioWaveform { samples } => {
                    (samples as usize, audio.waveform(samples as usize))
                }
                TextureRole::AudioFft { bins } => (bins as usize, audio.fft(bins as usize)),
                _ => continue,
            };
            let channels = audio.channels();
            write_audio_image(
                &mut images,
                &mut inputs,
                &texture.name,
                width,
                channels,
                &data,
            );
        }
    }
}

/// Write one row of `f32`s per channel into the named input's `r16float` image,
/// (re)creating it when missing or the wrong size.
fn write_audio_image(
    images: &mut Assets<Image>,
    inputs: &mut WispInputs,
    name: &str,
    width: usize,
    channels: usize,
    data: &[f32],
) {
    let bytes: Vec<u8> = data
        .iter()
        .flat_map(|v| half::f16::from_f32(*v).to_ne_bytes())
        .collect();
    if let Some(WispValue::Image(handle)) = inputs.get(name) {
        let handle = handle.clone();
        if let Some(mut image) = images.get_mut(&handle) {
            let size = image.texture_descriptor.size;
            if (size.width, size.height) == (width as u32, channels as u32) {
                image.data = Some(bytes);
                return;
            }
        }
    }
    let mut image = Image::new(
        Extent3d {
            width: width as u32,
            height: channels as u32,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        bytes,
        TextureFormat::R16Float,
        RenderAssetUsages::default(),
    );
    image.texture_descriptor.usage = TextureUsages::TEXTURE_BINDING | TextureUsages::COPY_DST;
    let handle = images.add(image);
    inputs.insert(name.to_string(), WispValue::Image(handle));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waveform_keeps_most_recent_samples() {
        let mut audio = WispAudio::default();
        audio.push_frames(&[0.1, 0.2, 0.3, 0.4], 1);
        let row = audio.waveform(2);
        assert_eq!(row, vec![0.3, 0.4]);
        // Shorter history zero-pads at the front.
        let row = audio.waveform(6);
        assert_eq!(row, vec![0.0, 0.0, 0.1, 0.2, 0.3, 0.4]);
    }

    #[test]
    fn interleaved_channels_are_split() {
        let mut audio = WispAudio::default();
        audio.push_frames(&[0.1, -0.1, 0.2, -0.2], 2);
        assert_eq!(audio.channels(), 2);
        let rows = audio.waveform(2);
        assert_eq!(rows, vec![0.1, 0.2, -0.1, -0.2]);
    }

    #[test]
    fn fft_peaks_at_the_tone_bin() {
        let mut audio = WispAudio::default();
        let bins = 64;
        let window = bins * 2;
        // A full-scale sine completing exactly 8 cycles per window lands in bin 8.
        let samples: Vec<f32> = (0..window)
            .map(|i| (std::f32::consts::TAU * 8.0 * i as f32 / window as f32).sin())
            .collect();
        audio.push_frames(&samples, 1);
        let spectrum = audio.fft(bins);
        let peak = spectrum
            .iter()
            .enumerate()
            .max_by(|a, b| a.1.total_cmp(b.1))
            .map(|(i, _)| i);
        assert_eq!(peak, Some(8));
        assert!(spectrum[8] > 0.4, "peak magnitude ~0.5: {}", spectrum[8]);
    }
}
