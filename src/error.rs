//! Live error reporting for wisp shaders.
//!
//! Wisp is built for live-coding: when a shader edit fails to load or compile, the
//! previous working version keeps rendering and the error is surfaced here (and via
//! the log). The `ui` feature's panel displays [`WispErrors`] on screen.
//!
//! Load errors (parse/validation/schema) never replace the loaded asset, so
//! last-good rendering comes for free. Pipeline errors are mirrored from the
//! render world each frame.

use crate::asset::Wisp;
use bevy::asset::AssetLoadFailedEvent;
use bevy::prelude::*;
use std::collections::BTreeMap;

/// The current wisp errors, for display and logging.
#[derive(Resource, Clone, Debug, Default, PartialEq)]
pub struct WispErrors {
    /// Asset load errors (parse/validation/schema), keyed by asset path.
    pub load: BTreeMap<String, String>,
    /// Pipeline compilation errors, keyed by pass entry point name.
    pub pipeline: BTreeMap<String, String>,
}

impl WispErrors {
    pub fn is_empty(&self) -> bool {
        self.load.is_empty() && self.pipeline.is_empty()
    }
}

/// Record failed wisp loads and clear them once the asset (re)loads.
pub(crate) fn collect_load_errors(
    mut failed: MessageReader<AssetLoadFailedEvent<Wisp>>,
    mut events: MessageReader<AssetEvent<Wisp>>,
    asset_server: Res<AssetServer>,
    mut errors: ResMut<WispErrors>,
) {
    for event in failed.read() {
        let path = event.path.to_string();
        let message = event.error.to_string();
        if errors.load.get(&path) != Some(&message) {
            error!("failed to load wisp `{path}`:\n{message}");
            errors.load.insert(path, message);
        }
    }
    for event in events.read() {
        if let AssetEvent::LoadedWithDependencies { id } | AssetEvent::Modified { id } = event
            && let Some(path) = asset_server.get_path(*id)
        {
            errors.load.remove(&path.to_string());
        }
    }
}
