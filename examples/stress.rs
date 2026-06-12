//! Soak test: rapidly switch the camera between shaders (including feedback and
//! multipass ones) and periodically reload them, hunting for races between the
//! per-view render components. Any wgpu validation error quits the app, so a
//! healthy run survives until interrupted.
//!
//! Regression test for the mixed-generation bug where stale bind groups met
//! freshly ping-ponged pass targets while a newly selected shader loaded.

use bevy::prelude::*;
use bevy_wisp::prelude::*;

const SHADERS: &[&str] = &[
    "wisp/test_persistent_buffer.wgsl",
    "wisp/test_multi_pass_rendering.wgsl",
    "wisp/test_inputs.wgsl",
    "wisp/test_float.wgsl",
];

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, WispPlugin))
        .add_systems(Startup, setup)
        .add_systems(Update, switch)
        .run();
}

fn setup(mut commands: Commands) {
    commands.spawn(Camera3d::default());
}

fn switch(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    cameras: Query<Entity, With<Camera3d>>,
    mut frame: Local<u32>,
) {
    *frame += 1;
    let Ok(camera) = cameras.single() else {
        return;
    };
    // Swap shaders every 20 frames; throw in reloads to exercise that path too.
    if frame.is_multiple_of(20) {
        let path = SHADERS[(*frame / 20) as usize % SHADERS.len()];
        let wisp: Handle<Wisp> = asset_server.load(path);
        commands.entity(camera).insert(WispHandle(wisp));
    }
    if frame.is_multiple_of(70) {
        let path = SHADERS[(*frame / 70) as usize % SHADERS.len()];
        asset_server.reload(path);
    }
}
