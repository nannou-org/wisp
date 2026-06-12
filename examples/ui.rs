//! The auto-generated wisp control panel (`ui` feature): every param in the
//! shader's interface gets a widget, and load/pipeline errors appear in the
//! panel while you edit.
//!
//! Pass another shader's asset path as an argument to inspect it instead, and
//! add `--features bevy/file_watcher` for live editing.

use bevy::prelude::*;
use bevy_wisp::prelude::*;

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: String::from("bevy_wisp - ui"),
                    ..default()
                }),
                ..default()
            }),
            // Wisp's panel runs in `Update`, which needs egui's single-pass mode.
            bevy_egui::EguiPlugin {
                enable_multipass_for_primary_context: false,
                ..default()
            },
            WispPlugin,
        ))
        .add_systems(Startup, setup)
        .run();
}

fn setup(mut commands: Commands, asset_server: Res<AssetServer>) {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| String::from("wisp/test_inputs.wgsl"));
    let wisp: Handle<Wisp> = asset_server.load(path);
    commands.spawn((Camera3d::default(), WispHandle(wisp)));
}
