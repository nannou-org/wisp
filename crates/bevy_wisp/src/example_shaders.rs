//! The bundled example shaders, embedded as `&str` constants.
//!
//! These are the same `.wgsl` files under `assets/wisp/` that this crate's
//! examples load at runtime, exposed here so downstream tools (such as the
//! `wisp_editor`) can embed them directly. A published crate can only
//! `include_str!` files within its own root, so this shared content is offered
//! through the dependency graph rather than the filesystem.
//!
//! Enabled by the `example_shaders` feature.

/// Every bundled example shader as a `(name, source)` pair, where the name is
/// the shader's file stem under `assets/wisp/`.
pub const ALL: &[(&str, &str)] = &[
    ("test_audio", include_str!("../assets/wisp/test_audio.wgsl")),
    (
        "test_audio_fft",
        include_str!("../assets/wisp/test_audio_fft.wgsl"),
    ),
    ("test_color", include_str!("../assets/wisp/test_color.wgsl")),
    (
        "test_compute",
        include_str!("../assets/wisp/test_compute.wgsl"),
    ),
    ("test_float", include_str!("../assets/wisp/test_float.wgsl")),
    ("test_image", include_str!("../assets/wisp/test_image.wgsl")),
    (
        "test_inputs",
        include_str!("../assets/wisp/test_inputs.wgsl"),
    ),
    (
        "test_multi_pass_rendering",
        include_str!("../assets/wisp/test_multi_pass_rendering.wgsl"),
    ),
    (
        "test_persistent_buffer",
        include_str!("../assets/wisp/test_persistent_buffer.wgsl"),
    ),
];
