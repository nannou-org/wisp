//! A trail simulation stepped by a compute pass writing a storage texture;
//! the final fragment pass just presents the result.

struct Globals {
    resolution: vec2<f32>,
    time: f32,
}
@group(0) @binding(0) var<uniform> globals: Globals;

@group(0) @binding(1) var samp: sampler;
@group(1) @binding(1) var sim: texture_2d<f32>;
@group(1) @binding(2) var sim_out: texture_storage_2d<rgba16float, write>;

/// @pass(target = "sim", float)
@compute @workgroup_size(8, 8, 1)
fn step_sim(@builtin(global_invocation_id) id: vec3<u32>) {
    let size = textureDimensions(sim_out);
    if id.x >= size.x || id.y >= size.y {
        return;
    }
    let coord = vec2<i32>(id.xy);
    let previous = textureLoad(sim, coord, 0).rgb * 0.98;
    let uv = (vec2<f32>(id.xy) + 0.5) / vec2<f32>(size);
    let centre = vec2<f32>(
        0.5 + 0.35 * cos(globals.time * 1.7),
        0.5 + 0.35 * sin(globals.time * 2.3),
    );
    let spot = smoothstep(0.04, 0.01, distance(uv, centre));
    let colour = max(previous, vec3<f32>(0.3, 0.9, 0.5) * spot);
    textureStore(sim_out, coord, vec4<f32>(colour, 1.0));
}

@fragment
fn present(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    return vec4<f32>(textureSample(sim, samp, uv).rgb, 1.0);
}
