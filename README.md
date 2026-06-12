# nannou_wisp

Interactive WGSL shaders for [nannou](https://nannou.cc), built on Bevy.

A *wisp* is a plain `.wgsl` file describing a (possibly multi-pass) fullscreen
shader. There is no external metadata: the shader's own interface is reflected
via `naga`. Members of its params uniform struct become tweakable inputs, `///`
doc-comment annotations supply defaults, ranges and pass configuration, and each
`@fragment`/`@compute` entry point becomes a pass.

```wgsl
struct Globals {
    resolution: vec2<f32>,
    time: f32,
}
@group(0) @binding(0) var<uniform> globals: Globals;

struct Params {
    /// Overall strength of the effect.
    /// @min(0.0) @max(1.0) @default(0.5)
    level: f32,
    /// @color @default(1.0, 0.0, 0.0, 1.0)
    tint: vec4<f32>,
}
@group(1) @binding(0) var<uniform> params: Params;

@fragment
fn fragment(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    return params.tint * params.level * sin(globals.time);
}
```

Wisp is the modern successor to `nannou_isf`. Support for the Interactive
Shader Format (GLSL + JSON) is planned as a translation layer on top of wisp.
