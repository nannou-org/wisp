# wisp

[![crates.io](https://img.shields.io/crates/v/bevy_wisp.svg)](https://crates.io/crates/bevy_wisp) [![docs.rs](https://docs.rs/bevy_wisp/badge.svg)](https://docs.rs/bevy_wisp) [![CI](https://github.com/nannou-org/wisp/actions/workflows/ci.yml/badge.svg)](https://github.com/nannou-org/wisp/actions/workflows/ci.yml)

The WGSL Interactive Shader Project, based on [Bevy](https://bevy.org), inspired
by [ISF](https://isf.video/).

A *wisp* is a plain `.wgsl` file describing a (possibly multi-pass) fullscreen
shader, with its tweakable interface reflected from the shader itself - no
external metadata.

This is a Cargo workspace of two crates:

- [`bevy_wisp`](crates/bevy_wisp) - the library: a Bevy plugin that turns
  `.wgsl` wisps into rendered, hot-reloadable, introspectable shaders. Its
  [README](crates/bevy_wisp/README.md) is the full guide (shader conventions,
  passes, inputs, errors).
- [`wisp_editor`](crates/wisp_editor) - a live-coding editor pairing a
  syntax-highlighted code editor with an auto-generated param panel; runs
  natively (`cargo run -p wisp_editor`) and on the web
  (`nix run .#serve-wisp-editor-web`).

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally submitted
for inclusion in the work by you, as defined in the Apache-2.0 license, shall be
dual licensed as above, without any additional terms or conditions.
