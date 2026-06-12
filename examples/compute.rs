//! A wisp with a `@compute` pass: the simulation step writes its target through
//! a storage texture (reading its own previous frame via ping-pong), and the
//! final fragment pass presents it. Pass another shader's asset path as an
//! argument to run that instead.

use nannou::prelude::*;

fn main() {
    nannou::app(model).run();
}

struct Model;

fn model(app: &App) -> Model {
    let camera = app.new_camera().build();
    app.new_window()
        .camera(camera)
        .primary()
        .size_pixels(1024, 512)
        .view(view)
        .build();

    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| String::from("wisp/test_compute.wgsl"));
    let wisp: Handle<Wisp> = app.asset_server().load(path);
    app.command_scope(move |mut commands| {
        commands.entity(camera).insert(WispHandle(wisp));
    });
    Model
}

fn view(app: &App, _model: &Model) {
    let _draw = app.draw();
}
