//! One of every input kind, for exercising the auto-generated UI.

struct Globals {
    resolution: vec2<f32>,
    time: f32,
}
@group(0) @binding(0) var<uniform> globals: Globals;

struct Params {
    /// Overall brightness of the pattern.
    /// @min(0.0) @max(2.0) @step(0.01) @default(1.0)
    brightness: f32,
    /// Invert the colours.
    /// @bool @default(0)
    invert: u32,
    /// How many times the pattern repeats.
    /// @values(1, 2, 4, 8) @labels("one", "two", "four", "eight") @default(2)
    repeats: i32,
    /// Centre of the pattern.
    /// @default(0.5, 0.5)
    centre: vec2<f32>,
    /// @color @default(0.2, 0.6, 1.0, 1.0)
    tint: vec4<f32>,
}
@group(1) @binding(0) var<uniform> params: Params;

@fragment
fn fragment(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let aspect = vec2<f32>(globals.resolution.x / max(globals.resolution.y, 1.0), 1.0);
    let p = (uv - params.centre) * aspect * f32(params.repeats);
    let rings = 0.5 + 0.5 * sin(length(p) * 20.0 - globals.time * 2.0);
    var colour = params.tint.rgb * rings * params.brightness;
    if params.invert != 0u {
        colour = vec3<f32>(1.0) - colour;
    }
    return vec4<f32>(colour, 1.0);
}
