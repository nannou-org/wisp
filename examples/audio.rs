//! Feed audio into wisp's `@audio`/`@audio_fft` textures (`audio` feature).
//!
//! This example synthesizes a wandering tone straight into the [`WispAudio`]
//! resource. To visualize real input instead, capture with e.g. `cpal` on its
//! own thread (audio streams are not `Send`) and push sample chunks through a
//! channel into `WispAudio::push_frames` from a system, just as done here.
//!
//! Pass another shader's asset path as an argument, e.g.
//! `-- wisp/test_audio.wgsl` for the oscilloscope view.

use bevy::prelude::*;
use bevy_wisp::prelude::*;

const SAMPLE_RATE: f32 = 44_100.0;

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: String::from("bevy_wisp - audio"),
                    ..default()
                }),
                ..default()
            }),
            WispPlugin,
        ))
        .add_systems(Startup, setup)
        .add_systems(Update, synthesize)
        .run();
}

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| String::from("wisp/test_audio_fft.wgsl"));
    let wisp: Handle<Wisp> = asset_server.load(path);
    commands.spawn((Camera3d::default(), WispHandle(wisp)));
}

/// Synthesize a tone whose pitch wanders, one chunk per frame.
fn synthesize(time: Res<Time>, mut audio: ResMut<WispAudio>, mut phase: Local<f32>) {
    let hz = 330.0 + 220.0 * (time.elapsed_secs() * 0.4).sin();
    let frames = (time.delta_secs() * SAMPLE_RATE).clamp(64.0, 4096.0) as usize;
    let samples: Vec<f32> = (0..frames)
        .map(|_| {
            *phase = (*phase + hz / SAMPLE_RATE).fract();
            (*phase * std::f32::consts::TAU).sin() * 0.8
        })
        .collect();
    audio.push_frames(&samples, 1);
}
