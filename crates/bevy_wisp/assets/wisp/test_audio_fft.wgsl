//! Spectrum bars from the live audio input.

@group(0) @binding(1) var samp: sampler;
/// @audio_fft(bins = 256)
@group(1) @binding(1) var spectrum: texture_2d<f32>;

@fragment
fn fragment(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let magnitude = textureSample(spectrum, samp, vec2<f32>(uv.x, 0.5)).r;
    let level = sqrt(clamp(magnitude, 0.0, 1.0));
    let lit = step(1.0 - uv.y, level);
    let background = vec3<f32>(0.03, 0.03, 0.08);
    let bar = vec3<f32>(0.9, 0.3 + 0.6 * uv.x, 0.2);
    return vec4<f32>(mix(background, bar, lit), 1.0);
}
