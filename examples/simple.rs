//! Load a wisp - a plain WGSL shader with a reflected interface - and render it
//! to the window, animating one of its inputs from the model.
//!
//! Run with `--features nannou/hot_reload` to live-edit
//! `examples/assets/wisp/test_float.wgsl` while it renders.

use nannou::prelude::*;

fn main() {
    nannou::app(model).update(update).run();
}

struct Model {
    camera: Entity,
}

fn model(app: &App) -> Model {
    let camera = app.new_camera().build();
    app.new_window()
        .camera(camera)
        .primary()
        .size_pixels(1024, 512)
        .view(view)
        .build();

    let wisp: Handle<Wisp> = app.asset_server().load("wisp/test_float.wgsl");
    app.command_scope(move |mut commands| {
        commands.entity(camera).insert(WispHandle(wisp));
    });
    Model { camera }
}

fn update(app: &App, model: &mut Model) {
    // Drive the `level` input back and forth; sliders land with the `wisp_ui`
    // feature, but inputs are just a component to mutate.
    let camera = model.camera;
    let level = (app.time().sin() * 0.5 + 0.5).clamp(0.0, 1.0);
    app.command_scope(move |mut commands| {
        commands.queue(move |world: &mut World| {
            if let Some(mut inputs) = world.entity_mut(camera).get_mut::<WispInputs>() {
                inputs.insert("level".to_string(), WispValue::F32(level));
            }
        });
    });
}

fn view(app: &App, _model: &Model) {
    let _draw = app.draw();
}
