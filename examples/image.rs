//! Set an image input on a wisp: textures declared in `@group(1)` (that are not
//! pass targets or audio textures) are exposed through `WispInputs` by name.

use nannou::prelude::*;

fn main() {
    nannou::app(model).update(update).run();
}

struct Model {
    camera: Entity,
    image: Handle<Image>,
}

fn model(app: &App) -> Model {
    let camera = app.new_camera().build();
    app.new_window()
        .camera(camera)
        .primary()
        .size_pixels(1024, 512)
        .view(view)
        .build();

    let wisp: Handle<Wisp> = app.asset_server().load("wisp/test_image.wgsl");
    let image: Handle<Image> = app.asset_server().load("images/nature/nature_1.jpg");
    app.command_scope(move |mut commands| {
        commands.entity(camera).insert(WispHandle(wisp));
    });
    Model { camera, image }
}

fn update(app: &App, model: &mut Model) {
    let camera = model.camera;
    let image = model.image.clone();
    app.command_scope(move |mut commands| {
        commands.queue(move |world: &mut World| {
            if let Some(mut inputs) = world.entity_mut(camera).get_mut::<WispInputs>() {
                inputs.insert(String::from("input_image"), WispValue::Image(image));
            }
        });
    });
}

fn view(app: &App, _model: &Model) {
    let _draw = app.draw();
}
