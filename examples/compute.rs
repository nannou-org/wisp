//! A wisp with a `@compute` pass: the simulation step writes its target through
//! a storage texture (reading its own previous frame via ping-pong), and the
//! final fragment pass presents it. Pass another shader's asset path as an
//! argument to run that instead.

use bevy::prelude::*;
use bevy_wisp::prelude::*;

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: String::from("bevy_wisp - compute"),
                    ..default()
                }),
                ..default()
            }),
            WispPlugin,
        ))
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| String::from("wisp/test_compute.wgsl"));
    let wisp: Handle<Wisp> = asset_server.load(path);
    commands.spawn((Camera3d::default(), WispHandle(wisp)));
}
