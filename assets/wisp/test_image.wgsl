//! Sample a user-supplied image input, with a tweakable scanline wobble.

struct Globals {
    resolution: vec2<f32>,
    time: f32,
}
@group(0) @binding(0) var<uniform> globals: Globals;

struct Params {
    /// Strength of the horizontal wobble.
    /// @min(0.0) @max(0.1) @default(0.02)
    wobble: f32,
}
@group(1) @binding(0) var<uniform> params: Params;

@group(0) @binding(1) var samp: sampler;
@group(1) @binding(1) var input_image: texture_2d<f32>;

@fragment
fn fragment(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let offset = vec2<f32>(sin(uv.y * 20.0 + globals.time * 2.0), 0.0) * params.wobble;
    return textureSample(input_image, samp, uv + offset);
}
