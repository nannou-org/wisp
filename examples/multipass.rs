//! Multi-pass wisp shaders: a persistent feedback trail by default, or pass
//! another shader's asset path as an argument, e.g.
//!
//! ```sh
//! cargo run --example multipass -- wisp/test_multi_pass_rendering.wgsl
//! ```

use bevy::prelude::*;
use bevy_wisp::prelude::*;

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: String::from("bevy_wisp - multipass"),
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
        .unwrap_or_else(|| String::from("wisp/test_persistent_buffer.wgsl"));
    let wisp: Handle<Wisp> = asset_server.load(path);
    commands.spawn((Camera3d::default(), WispHandle(wisp)));
}
