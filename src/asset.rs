//! The [`Wisp`] asset and its loader.
//!
//! Wisp shaders are plain `.wgsl` files, so [`WispLoader`] registers no extensions
//! of its own: load one explicitly with `asset_server.load::<Wisp>("path.wgsl")`,
//! which resolves the loader by asset type. Untyped `.wgsl` loads (including
//! bevy's own `Shader` loading) are unaffected.
//!
//! The compiled [`Shader`] is stored as a labeled sub-asset (`"shader"`) of the
//! `Wisp`, so the same source is never loaded through bevy's shader loader, and
//! replacing it on hot reload recompiles dependent pipelines automatically.

use crate::reflect::{self, ReflectError};
use crate::schema::{self, SchemaError, WispSchema};
use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext};
use bevy::prelude::*;
use bevy::render::extract_component::ExtractComponent;
use thiserror::Error;

/// A loaded wisp shader: its reflected interface and compiled shader module.
#[derive(Asset, TypePath, Debug, Clone)]
pub struct Wisp {
    pub schema: WispSchema,
    pub shader: Handle<Shader>,
}

/// Renders the wisp to a camera: insert on a camera entity, pointing at a loaded
/// [`Wisp`]. Output goes wherever the camera renders - a window, or an `Image`
/// render target.
#[derive(Component, ExtractComponent, Deref, DerefMut, Debug, Default, Clone)]
pub struct WispHandle(pub Handle<Wisp>);

#[derive(Default, TypePath)]
pub struct WispLoader;

#[derive(Debug, Error)]
pub enum WispLoadError {
    #[error("failed to read wisp shader: {0}")]
    Io(#[from] std::io::Error),
    #[error("wisp shader is not valid UTF-8: {0}")]
    Utf8(#[from] std::str::Utf8Error),
    #[error(transparent)]
    Reflect(#[from] ReflectError),
    #[error(transparent)]
    Schema(#[from] SchemaError),
}

impl AssetLoader for WispLoader {
    type Asset = Wisp;
    type Settings = ();
    type Error = WispLoadError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        _settings: &Self::Settings,
        load_context: &mut LoadContext<'_>,
    ) -> Result<Self::Asset, Self::Error> {
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;
        let source = std::str::from_utf8(&bytes)?;
        let reflected = reflect::parse_and_validate(source)?;
        let schema = schema::schema_from_module(&reflected)?;
        let path = load_context.path().to_string();
        let shader = Shader::from_wgsl(source.to_string(), path);
        let shader = load_context.add_labeled_asset(String::from("shader"), shader);
        Ok(Wisp { schema, shader })
    }

    // No extensions: wisp shaders are plain `.wgsl`, loaded by asset type so that
    // bevy's own `Shader` loader keeps owning the extension.
    fn extensions(&self) -> &[&str] {
        &[]
    }
}
