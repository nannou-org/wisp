//! Wisp - interactive WGSL shaders for nannou.
//!
//! A *wisp* is a plain `.wgsl` file describing a (possibly multi-pass) fullscreen
//! shader. There is no external metadata: the shader's own interface is reflected
//! via naga. Members of its params uniform struct become tweakable inputs, `///`
//! doc-comment annotations supply defaults, ranges and pass configuration, and
//! each `@fragment`/`@compute` entry point becomes a pass.
//!
//! ```wgsl
//! struct Globals {
//!     resolution: vec2<f32>,
//!     time: f32,
//! }
//! @group(0) @binding(0) var<uniform> globals: Globals;
//!
//! struct Params {
//!     /// Overall strength of the effect.
//!     /// @min(0.0) @max(1.0) @default(0.5)
//!     level: f32,
//!     /// @color @default(1.0, 0.0, 0.0, 1.0)
//!     tint: vec4<f32>,
//! }
//! @group(1) @binding(0) var<uniform> params: Params;
//!
//! @fragment
//! fn fragment(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
//!     return params.tint * params.level * sin(globals.time);
//! }
//! ```
//!
//! See [`schema`] for the full set of conventions and annotations, [`globals`] for
//! the recognized globals members, and [`asset`] for how loading works.

use crate::asset::{Wisp, WispLoader};
use bevy::prelude::*;

pub mod annot;
pub mod asset;
pub mod globals;
pub mod inputs;
pub mod reflect;
pub mod schema;

pub mod prelude {
    pub use crate::NannouWispPlugin;
    pub use crate::asset::{Wisp, WispHandle};
    pub use crate::inputs::{WispInputs, WispValue};
    pub use crate::schema::WispSchema;
}

pub struct NannouWispPlugin;

impl Plugin for NannouWispPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<Wisp>().init_asset_loader::<WispLoader>();
    }
}
