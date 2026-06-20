//! The first pass renders plasma into a 1/16-resolution buffer; the final pass
//! upscales it into chunky pixels (port of ISF's Test-MultiPassRendering).

struct Globals {
    resolution: vec2<f32>,
    time: f32,
}
@group(0) @binding(0) var<uniform> globals: Globals;

@group(0) @binding(1) var samp: sampler;
@group(1) @binding(1) var low_res: texture_2d<f32>;

/// @pass(target = "low_res", width = "$WIDTH/16.0", height = "$HEIGHT/16.0")
@fragment
fn plasma(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let t = globals.time;
    let v = sin(uv.x * 10.0 + t)
        + sin(uv.y * 8.0 - t * 1.3)
        + sin((uv.x + uv.y) * 12.0 + t * 0.7);
    let colour = 0.5 + 0.5 * vec3<f32>(sin(v), sin(v + 2.094), sin(v + 4.188));
    return vec4<f32>(colour, 1.0);
}

@fragment
fn fragment(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    return textureSample(low_res, samp, uv);
}
