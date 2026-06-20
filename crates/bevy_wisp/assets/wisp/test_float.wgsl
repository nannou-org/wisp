//! Slide `level` to sweep a pulsing bar across the window.

struct Globals {
    resolution: vec2<f32>,
    time: f32,
}
@group(0) @binding(0) var<uniform> globals: Globals;

struct Params {
    /// Horizontal position of the sweep edge.
    /// @min(0.0) @max(1.0) @default(0.5)
    level: f32,
}
@group(1) @binding(0) var<uniform> params: Params;

@fragment
fn fragment(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let pulse = 0.6 + 0.4 * sin(globals.time * 2.0);
    let lit = select(0.0, pulse, uv.x < params.level);
    let aspect = globals.resolution.x / max(globals.resolution.y, 1.0);
    let rings = 0.5 + 0.5 * sin(distance(uv * vec2<f32>(aspect, 1.0), vec2<f32>(aspect * 0.5, 0.5)) * 40.0 - globals.time * 4.0);
    return vec4<f32>(lit, lit * rings, rings * 0.4, 1.0);
}
