//! Set an image input on a wisp: textures declared in `@group(1)` (that are not
//! pass targets or audio textures) are exposed through `WispInputs` by name.
//!
//! The image here is generated procedurally; any `Handle<Image>` works the same
//! way (loaded files, render targets, ...).

use bevy::asset::RenderAssetUsages;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_wisp::prelude::*;

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: String::from("bevy_wisp - image"),
                    ..default()
                }),
                ..default()
            }),
            WispPlugin,
        ))
        .add_systems(Startup, setup)
        .add_systems(Update, set_image_input)
        .run();
}

#[derive(Resource)]
struct InputImage(Handle<Image>);

fn setup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
) {
    let wisp: Handle<Wisp> = asset_server.load("wisp/test_image.wgsl");
    commands.spawn((Camera3d::default(), WispHandle(wisp)));
    commands.insert_resource(InputImage(images.add(checkerboard(512, 32))));
}

/// Keep the shader's `input_image` pointed at our image (inputs are rebuilt
/// from the schema on load and hot reload).
fn set_image_input(image: Res<InputImage>, mut cameras: Query<&mut WispInputs>) {
    for mut inputs in &mut cameras {
        inputs.insert(
            String::from("input_image"),
            WispValue::Image(image.0.clone()),
        );
    }
}

/// A colourful checkerboard over a gradient.
fn checkerboard(size: u32, cell: u32) -> Image {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let on = ((x / cell) + (y / cell)).is_multiple_of(2);
            let r = if on { 230u8 } else { 30 };
            let g = (x * 255 / size) as u8;
            let b = (y * 255 / size) as u8;
            data.extend_from_slice(&[r, g, b, 255]);
        }
    }
    Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}
