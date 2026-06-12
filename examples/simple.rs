//! Load a wisp - a plain WGSL shader with a reflected interface - and render it
//! to the window, animating one of its inputs from a system.
//!
//! Run with `--features bevy/file_watcher` to live-edit
//! `assets/wisp/test_float.wgsl` while it renders.

use bevy::prelude::*;
use bevy_wisp::prelude::*;

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: String::from("bevy_wisp - simple"),
                    ..default()
                }),
                ..default()
            }),
            WispPlugin,
        ))
        .add_systems(Startup, setup)
        .add_systems(Update, animate)
        .run();
}

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    let wisp: Handle<Wisp> = asset_server.load("wisp/test_float.wgsl");
    commands.spawn((Camera3d::default(), WispHandle(wisp)));
}

/// Drive the `level` input back and forth; sliders come with the `ui` feature,
/// but inputs are just a component to mutate.
fn animate(time: Res<Time>, mut cameras: Query<&mut WispInputs>) {
    let level = time.elapsed_secs().sin() * 0.5 + 0.5;
    for mut inputs in &mut cameras {
        inputs.insert(String::from("level"), WispValue::F32(level));
    }
}
