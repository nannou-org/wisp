//! Multi-pass wisp shaders: a persistent feedback trail by default, or pass
//! another shader's asset path as an argument, e.g.
//!
//! ```sh
//! cargo run -p examples --example wisp_multipass --features nannou/wisp \
//!     -- wisp/test_multi_pass_rendering.wgsl
//! ```

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
        .unwrap_or_else(|| String::from("wisp/test_persistent_buffer.wgsl"));
    let wisp: Handle<Wisp> = app.asset_server().load(path);
    app.command_scope(move |mut commands| {
        commands.entity(camera).insert(WispHandle(wisp));
    });
    Model
}

fn view(app: &App, _model: &Model) {
    let _draw = app.draw();
}
