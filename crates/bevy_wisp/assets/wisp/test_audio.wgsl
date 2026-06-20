//! Draws the live input waveform as an oscilloscope line.

@group(0) @binding(1) var samp: sampler;
/// @audio(samples = 512)
@group(1) @binding(1) var waveform: texture_2d<f32>;

@fragment
fn fragment(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let sample = textureSample(waveform, samp, vec2<f32>(uv.x, 0.5)).r;
    let trace_y = 0.5 - sample * 0.4;
    let line = smoothstep(0.015, 0.0, abs(uv.y - trace_y));
    let colour = vec3<f32>(0.02, 0.03, 0.05) + line * vec3<f32>(0.2, 0.9, 0.6);
    return vec4<f32>(colour, 1.0);
}
