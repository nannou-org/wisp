# wisp_editor

[![crates.io](https://img.shields.io/crates/v/wisp_editor.svg)](https://crates.io/crates/wisp_editor)

A minimal live-coding editor for [`bevy_wisp`](https://crates.io/crates/bevy_wisp)
shaders, native and on the web.

The UI is a re-arrangeable [`egui_tiles`](https://crates.io/crates/egui_tiles)
layout of three panes: a syntax-highlighted code editor (with the file
controls), the auto-generated shader param widgets, and the shader view itself.
Pick one of the bundled shaders (compiled into the binary, so no assets dir is
needed at runtime) or create your own. Saving (button or ctrl/cmd+S) persists to
a key-value store that works the same on native (a file in the platform data
dir) and on the web (browser local storage), and reloads the shader in place;
broken edits keep the last working version on screen while the error shows in
the params pane.

## Run it

```sh
cargo run -p wisp_editor              # native
```

The editor also builds for the web (via [trunk](https://trunkrs.dev)); from the
workspace, `nix run .#serve-wisp-editor-web` serves it locally. The web build
defaults to the WebGL2 backend; for the WebGPU backend (runs the compute-pass
shaders too) build with `--no-default-features --features webgpu`. On the web,
only the bundled shaders are available and saving is disabled.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](../../LICENSE-APACHE))
- MIT license ([LICENSE-MIT](../../LICENSE-MIT))

at your option.
