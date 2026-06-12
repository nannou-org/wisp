//! Wisp - interactive WGSL shaders for Bevy.
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
//! Add [`WispPlugin`] to the app, load a wisp with
//! `asset_server.load::<Wisp>("path.wgsl")` and insert a
//! [`WispHandle`](asset::WispHandle) on a camera; the shader renders wherever the
//! camera does. Tweak inputs through the camera's [`WispInputs`](inputs::WispInputs)
//! component.
//!
//! See [`schema`] for the full set of conventions and annotations, [`globals`] for
//! the recognized globals members, and [`asset`] for how loading works.
//!
//! Wisp grew out of [nannou](https://nannou.cc)'s `nannou_isf` (an implementation
//! of the Interactive Shader Format) and is the modern, WGSL-first successor to
//! that idea.

// Bevy system signatures routinely exceed clippy's tuple-complexity threshold.
#![allow(clippy::type_complexity)]

use crate::asset::{Wisp, WispHandle, WispLoader};
use crate::error::WispErrors;
use crate::globals::FrameGlobals;
use crate::inputs::{WispInputs, inputs_from_schema, rematch_inputs};
use bevy::camera::RenderTarget;
use bevy::platform::collections::HashSet;
use bevy::prelude::*;
use bevy::window::{PresentMode, PrimaryWindow, WindowRef};

pub mod annot;
pub mod asset;
#[cfg(feature = "audio")]
pub mod audio;
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
    pub use crate::asset::{Wisp, WispHandle};
    #[cfg(feature = "audio")]
    pub use crate::audio::WispAudio;
    pub use crate::error::WispErrors;
    pub use crate::inputs::{WispInputs, WispValue};
    pub use crate::schema::WispSchema;
    pub use crate::{WispConfig, WispPlugin};
}

/// Behavioural configuration for [`WispPlugin`]. Insert before adding the
/// plugin to override the defaults.
#[derive(Resource, Clone, Debug)]
pub struct WispConfig {
    /// Request [`PresentMode::Mailbox`] for windows displaying a wisp that are
    /// still on bevy's default present mode. Mailbox keeps vsync (no tearing)
    /// without blocking rendering, noticeably lowering latency on native
    /// platforms - good for live visuals; bevy falls back (`Mailbox` ->
    /// `Immediate` -> `Fifo`) where unsupported.
    ///
    /// A window whose present mode differs from the default is always left
    /// alone. To keep the default `Fifo` explicitly, disable this.
    pub prefer_mailbox: bool,
    /// Show the floating params/errors window provided by the `ui` feature.
    /// Disable to embed the widgets in your own UI via `ui::params_ui` and
    /// `ui::errors_ui` instead (see the `editor` example).
    pub ui_window: bool,
}

impl Default for WispConfig {
    fn default() -> Self {
        Self {
            prefer_mailbox: true,
            ui_window: true,
        }
    }
}

pub struct WispPlugin;

impl Plugin for WispPlugin {
    fn build(&self, app: &mut App) {
        app.init_asset::<Wisp>()
            .init_asset_loader::<WispLoader>()
            .init_resource::<FrameGlobals>()
            .init_resource::<WispConfig>()
            .init_resource::<WispErrors>()
            .add_plugins(render::WispRenderPlugin)
            .add_systems(
                Update,
                (
                    globals::update_frame_globals,
                    sync_wisp_inputs,
                    targets::update_pass_targets,
                    error::collect_load_errors,
                    prefer_mailbox_present_mode,
                ),
            );
        #[cfg(feature = "audio")]
        app.init_resource::<audio::WispAudio>()
            .add_systems(Update, audio::update_audio_textures.after(sync_wisp_inputs));
        // The panel needs `EguiPlugin`; stay inert (rather than panicking on the
        // missing resource) when the user hasn't added it.
        #[cfg(feature = "ui")]
        app.add_systems(
            Update,
            ui::wisp_ui.run_if(resource_exists::<bevy_egui::EguiUserTextures>),
        );
    }
}

/// Request low-latency mailbox presentation for windows displaying a wisp,
/// where the window is still on bevy's default present mode (see
/// [`WispConfig::prefer_mailbox`]).
fn prefer_mailbox_present_mode(
    config: Res<WispConfig>,
    cameras: Query<&RenderTarget, With<WispHandle>>,
    primary: Query<Entity, With<PrimaryWindow>>,
    mut windows: Query<&mut Window>,
) {
    if !config.prefer_mailbox {
        return;
    }
    for target in &cameras {
        let RenderTarget::Window(window_ref) = target else {
            continue;
        };
        let entity = match window_ref {
            WindowRef::Primary => primary.iter().next(),
            WindowRef::Entity(entity) => Some(*entity),
        };
        let Some(mut window) = entity.and_then(|entity| windows.get_mut(entity).ok()) else {
            continue;
        };
        // Only override the default - an explicit choice wins.
        if window.present_mode == PresentMode::default() {
            window.present_mode = PresentMode::Mailbox;
        }
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
