//! A moving dot leaves a fading trail in a persistent feedback buffer
//! (port of ISF's Test-PersistentBuffer).

struct Globals {
    resolution: vec2<f32>,
    time: f32,
}
@group(0) @binding(0) var<uniform> globals: Globals;

struct Params {
    /// How slowly the trail fades (1.0 never fades).
    /// @min(0.0) @max(1.0) @default(0.97)
    feedback: f32,
}
@group(1) @binding(0) var<uniform> params: Params;

@group(0) @binding(1) var samp: sampler;
@group(1) @binding(1) var trail: texture_2d<f32>;

/// @pass(target = "trail", persistent)
@fragment
fn accumulate(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let previous = textureSample(trail, samp, uv).rgb * params.feedback;
    let centre = vec2<f32>(
        0.5 + 0.35 * cos(globals.time),
        0.5 + 0.35 * sin(globals.time * 1.3),
    );
    let aspect = vec2<f32>(globals.resolution.x / max(globals.resolution.y, 1.0), 1.0);
    let dist = length((uv - centre) * aspect);
    let spot = smoothstep(0.05, 0.02, dist);
    let colour = vec3<f32>(0.9, 0.6, 0.2) * spot;
    return vec4<f32>(max(previous, colour), 1.0);
}

@fragment
fn present(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSample(trail, samp, uv).rgb, 1.0);
}
