# wisp

[![crates.io](https://img.shields.io/crates/v/bevy_wisp.svg)](https://crates.io/crates/bevy_wisp) [![docs.rs](https://docs.rs/bevy_wisp/badge.svg)](https://docs.rs/bevy_wisp) [![CI](https://github.com/nannou-org/wisp/actions/workflows/ci.yml/badge.svg)](https://github.com/nannou-org/wisp/actions/workflows/ci.yml)

The WGSL Interactive Shader Project, based on [Bevy](https://bevy.org), inspired by [ISF](https://isf.video/).

A *wisp* is a plain `.wgsl` file describing a (possibly multi-pass) fullscreen
shader. There is no external metadata: the shader's own interface is reflected
via `naga`. Members of its params uniform struct become tweakable inputs, `///`
doc-comment annotations supply defaults, ranges and pass configuration, and
each `@fragment`/`@compute` entry point becomes a pass.

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

## Quick start

```rust,ignore
use bevy::prelude::*;
use bevy_wisp::prelude::*;

fn main() {
    App::new()
        .add_plugins((DefaultPlugins, WispPlugin))
        .add_systems(Startup, |mut commands: Commands, assets: Res<AssetServer>| {
            // Typed load: wisps are plain .wgsl, resolved by asset type.
            let wisp: Handle<Wisp> = assets.load("wisp/my_shader.wgsl");
            // Output goes wherever the camera renders - a window, or an
            // `Image` render target.
            commands.spawn((Camera3d::default(), WispHandle(wisp)));
        })
        .run();
}
```

Tweak inputs by mutating the camera's `WispInputs` component, or enable the
`ui` feature for an auto-generated egui panel. With bevy's `file_watcher`
feature, edits to the file show up live; broken edits keep the last working
shader on screen and surface the error (`WispErrors`, the log, and the panel).

For low latency, windows displaying a wisp prefer `PresentMode::Mailbox`
(vsync without blocking) when still on bevy's default present mode, falling
back gracefully where unsupported. Opt out via `WispConfig::prefer_mailbox`
or by setting a present mode explicitly.

Try the examples: `simple`, `multipass`, `compute`, `image`,
`ui` (`--features ui`) and `audio` (`--features audio`), e.g.

```sh
cargo run --example ui --features ui,bevy/file_watcher
```

For a self-contained live-coding setup, the `editor` example pairs a
syntax-highlighted code editor with the param panel - pick a bundled shader
or create your own (kept in the platform data dir, e.g. `~/.local/share/wisp`),
with saves reloading the shader in place:

```sh
cargo run --example editor --features ui
```

## Shader conventions

Wisp builds the pipeline layout from whatever the shader declares, under these
conventions:

| Group | Binding | Contents |
|---|---|---|
| `@group(0)` | `@binding(0)` | optional *globals* uniform struct (wisp-provided values) |
| `@group(0)` | `@binding(1)` | optional sampler (the default filtering sampler) |
| `@group(1)` | `@binding(0)` | optional *params* uniform struct (your tweakable inputs) |
| `@group(1)` | any | textures and further samplers |

The fullscreen vertex stage is provided by bevy; fragment entry points take
`@location(0) uv: vec2<f32>` (origin top-left).

### Globals

Declare any subset of the recognized members - wisp writes each at its
reflected offset:

| Member | Type | Meaning |
|---|---|---|
| `resolution` | `vec2<f32>` | current pass target size in physical pixels |
| `time` | `f32` | seconds since the app started |
| `time_delta` | `f32` | seconds since the previous frame |
| `frame` | `u32` | frames since the app started |
| `pass_index` | `u32` | index of the current pass, in declaration order |
| `mouse` | `vec4<f32>` | cursor xy in pixels; z = 1 while the primary button is held; w = 1 on press |
| `date` | `vec4<f32>` | (year, month, day, seconds since midnight), UTC |

### Params

Members of the params struct may be `f32`, `i32`, `u32`, `vec2<f32>`,
`vec3<f32>` or `vec4<f32>`. Doc-comment annotations supply UI hints; free text
becomes the tooltip:

| Annotation | On | Meaning |
|---|---|---|
| `@min(x)` `@max(x)` `@step(x)` | scalars | slider/drag range |
| `@default(x, ..)` | any member | initial value (component count must match) |
| `@bool` | `u32` | expose as a toggle (WGSL forbids `bool` in uniforms) |
| `@color` | `vec3`/`vec4` | colour picker |
| `@label("..")` | any member | display name override |
| `@values(a, b, ..)` + `@labels("..", ..)` | `i32`/`u32` | dropdown |

Unknown annotations are load errors, so typos can't pass silently.

### Textures

`texture_2d<f32>` bindings in `@group(1)` are classified by name:

- a name matching a pass target reads that target (see below);
- `/// @audio(samples = 512)` or `/// @audio_fft(bins = 256)` become audio
  textures (`audio` feature) - `r16float`, one row per channel, waveforms in
  `[-1, 1]` and Hann-windowed linear FFT magnitudes respectively;
- anything else is an *image input*, settable by name through `WispInputs`
  (a placeholder image is bound until set).

All samplers are bound to the default filtering sampler.

## Passes

Every `@fragment`/`@compute` entry point is a pass, executed in declaration
order. Exactly one `@fragment` entry point omits a target - the *final pass*,
rendering to the view. The rest are configured by a `@pass` annotation:

```wgsl
/// @pass(target = "trail", persistent, float, width = "$WIDTH/2", height = "$HEIGHT/2")
@fragment
fn accumulate(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> { .. }
```

- `target = ".."` names the pass's intermediate image; any pass can read it by
  declaring a `texture_2d<f32>` of the same name. Reads of targets written
  earlier in the frame see the fresh contents; a pass reading its *own* target
  (feedback) sees the previous frame via ping-pong buffering.
- `persistent` keeps the previous frame's contents instead of clearing.
- `float` renders at `rgba16float` precision (default `rgba8unorm-srgb`, or
  `rgba8unorm` for compute targets).
- `width`/`height` are `evalexpr` expressions over `$WIDTH`/`$HEIGHT` (the
  view size); targets default to the view size.

A `@compute` entry point must have a target, written through a write-only
`texture_storage_2d` named `<target>_out` (format `rgba16float` with `float`,
`rgba8unorm` otherwise). Workgroup counts default to
`ceil(target_size / workgroup_size)`, or set
`dispatch = "$WIDTH/8, $HEIGHT/8, 1"` (evaluated against the *target* size).
Compute targets are never cleared.

## Errors and hot reload

Wisp validates at load time with the same `naga` that compiles the shader, so
errors carry exact source spans. Failed loads never replace the loaded asset -
the last working shader keeps rendering while `WispErrors` (and the `ui`
panel) show what went wrong. naga_oil directives (`#import` etc.) are
rejected: wisp shaders are plain WGSL.

## Cargo features

- `ui` - auto-generated egui control panel (requires `bevy_egui`'s
  `EguiPlugin` in single-pass mode; see the `ui` example).
- `audio` - waveform/FFT textures fed from the `WispAudio` resource.

## Bevy compatibility

| bevy | bevy_wisp |
|------|-----------|
| 0.19 | 0.1       |

## Relation to ISF and nannou

Wisp grew out of [nannou](https://nannou.cc) as the successor to `nannou_isf`
and the [Interactive Shader Format](https://isf.video): the same idea -
shaders as portable, introspectable assets with tweakable inputs and
multi-pass rendering - rebuilt WGSL-first with the interface reflected from
the shader itself instead of a JSON comment block.

Differences to be aware of: passes are entry points rather than `PASSES`
entries, float targets are `rgba16float` (not `rgba32float`), there is no
`event` input type (use `@bool`), and audio waveforms are signed. Ideally, one
day we'd have support for loading ISF files via a GLSL-to-WGSL translation.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.

Unless you explicitly state otherwise, any contribution intentionally
submitted for inclusion in the work by you, as defined in the Apache-2.0
license, shall be dual licensed as above, without any additional terms or
conditions.
