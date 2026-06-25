//! Tint a vertical gradient with a tweakable color.

struct Params {
    /// @color @default(0.9, 0.4, 0.1, 1.0)
    tint: vec4<f32>,
    /// How strongly the gradient fades towards the bottom.
    /// @min(0.0) @max(1.0) @default(1.0)
    fade: f32,
}
@group(1) @binding(0) var<uniform> params: Params;

@fragment
fn fragment(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let brightness = 1.0 - uv.y * params.fade;
    return vec4<f32>(params.tint.rgb * brightness, 1.0);
}
