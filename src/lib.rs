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
//! Load a wisp with `asset_server.load::<Wisp>("path.wgsl")` and insert a
//! [`WispHandle`](asset::WispHandle) on a camera; the shader renders wherever the
//! camera does. Tweak inputs through the camera's [`WispInputs`](inputs::WispInputs)
//! component.
//!
//! See [`schema`] for the full set of conventions and annotations, [`globals`] for
//! the recognized globals members, and [`asset`] for how loading works.

use crate::asset::{Wisp, WispHandle, WispLoader};
use crate::error::WispErrors;
use crate::globals::FrameGlobals;
use crate::inputs::{WispInputs, inputs_from_schema, rematch_inputs};
use bevy::platform::collections::HashSet;
use bevy::prelude::*;

pub mod annot;
pub mod asset;
pub mod error;
pub mod globals;
pub mod inputs;
pub mod reflect;
pub mod render;
pub mod schema;
pub mod targets;
#[cfg(feature = "ui")]
pub mod ui;

pub mod prelude {
    pub use crate::NannouWispPlugin;
    pub use crate::asset::{Wisp, WispHandle};
    pub use crate::error::WispErrors;
    pub use crate::inputs::{WispInputs, WispValue};
    pub use crate::schema::WispSchema;
}

pub struct NannouWispPlugin;

impl Plugin for NannouWispPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<Wisp>()
            .init_asset_loader::<WispLoader>()
            .init_resource::<FrameGlobals>()
            .init_resource::<WispErrors>()
            .add_plugins(render::WispRenderPlugin)
            .add_systems(
                Update,
                (
                    globals::update_frame_globals,
                    sync_wisp_inputs,
                    targets::update_pass_targets,
                    error::collect_load_errors,
                ),
            );
        // The panel needs `EguiPlugin`; stay inert (rather than panicking on the
        // missing resource) when the user hasn't added it.
        #[cfg(feature = "ui")]
        app.add_systems(
            Update,
            ui::wisp_ui.run_if(resource_exists::<bevy_egui::EguiUserTextures>),
        );
    }
}

/// Keep each wisp camera's [`WispInputs`] in sync with its schema: populate them
/// when the handle is added or the asset (re)loads, preserving values that still
/// match by name and type.
fn sync_wisp_inputs(
    mut commands: Commands,
    wisps: Res<Assets<Wisp>>,
    mut events: MessageReader<AssetEvent<Wisp>>,
    mut cameras: Query<(Entity, Ref<WispHandle>, Option<&mut WispInputs>)>,
) {
    let updated: HashSet<AssetId<Wisp>> = events
        .read()
        .filter_map(|event| match event {
            AssetEvent::LoadedWithDependencies { id } | AssetEvent::Modified { id } => Some(*id),
            _ => None,
        })
        .collect();
    for (entity, handle, inputs) in cameras.iter_mut() {
        let stale = handle.is_changed() || inputs.is_none() || updated.contains(&handle.id());
        if !stale {
            continue;
        }
        let Some(wisp) = wisps.get(&**handle) else {
            continue;
        };
        match inputs {
            Some(mut inputs) => {
                let rematched = rematch_inputs(&inputs, &wisp.schema);
                *inputs = rematched;
            }
            None => {
                commands
                    .entity(entity)
                    .insert(inputs_from_schema(&wisp.schema));
            }
        }
    }
}
