//! The auto-generated wisp control panel: every param in the shader's interface
//! gets a widget, and load/pipeline errors appear in the panel while you edit.
//!
//! Pass another shader's asset path as an argument to inspect it instead, and
//! add `nannou/hot_reload` for live editing.

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
        .unwrap_or_else(|| String::from("wisp/test_inputs.wgsl"));
    let wisp: Handle<Wisp> = app.asset_server().load(path);
    app.command_scope(move |mut commands| {
        commands.entity(camera).insert(WispHandle(wisp));
    });
    Model
}

fn view(app: &App, _model: &Model) {
    let _draw = app.draw();
}
