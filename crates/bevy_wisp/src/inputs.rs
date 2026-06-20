//! Runtime values for a wisp shader's inputs.
//!
//! [`WispInputs`] lives on the camera entity alongside the
//! [`WispHandle`](crate::asset::WispHandle). It is created from the schema when the
//! asset loads (params fields and image inputs, with `@default`s honoured) and can
//! be mutated freely - by user systems or by the `ui` feature's panel. On hot
//! reload, values are re-matched by name and type so tweaks survive shader edits.

use crate::schema::{ParamField, ParamType, ParamsSchema, TextureRole, WispSchema};
use bevy_asset::prelude::Handle;
use bevy_derive::{Deref, DerefMut};
use bevy_ecs::prelude::*;
use bevy_image::prelude::Image;
use bevy_math::{Vec2, Vec3, Vec4};
use bevy_render::extract_component::ExtractComponent;
use std::collections::BTreeMap;

/// The current input values for a wisp shader, keyed by name.
#[derive(Component, ExtractComponent, Clone, Debug, Default, Deref, DerefMut)]
pub struct WispInputs(pub BTreeMap<String, WispValue>);

/// A single input value.
#[derive(Clone, Debug, PartialEq)]
pub enum WispValue {
    Bool(bool),
    I32(i32),
    U32(u32),
    F32(f32),
    Vec2(Vec2),
    Vec3(Vec3),
    Vec4(Vec4),
    Image(Handle<Image>),
}

impl WispValue {
    /// Whether two values are the same variant, regardless of contents.
    pub fn same_kind(&self, other: &WispValue) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}

/// Initial inputs for a schema: params fields at their `@default`s plus image
/// inputs at the default (placeholder) image.
pub fn inputs_from_schema(schema: &WispSchema) -> WispInputs {
    let mut map = BTreeMap::new();
    if let Some(params) = &schema.params {
        for field in &params.fields {
            map.insert(field.name.clone(), default_value(field));
        }
    }
    for texture in &schema.textures {
        if let TextureRole::ImageInput = texture.role {
            map.insert(texture.name.clone(), WispValue::Image(Handle::default()));
        }
    }
    WispInputs(map)
}

/// Inputs for a (re)loaded schema, preserving previous values where the name and
/// type still match.
pub fn rematch_inputs(old: &WispInputs, schema: &WispSchema) -> WispInputs {
    let mut inputs = inputs_from_schema(schema);
    for (name, value) in inputs.0.iter_mut() {
        if let Some(prev) = old.0.get(name)
            && prev.same_kind(value)
        {
            *value = prev.clone();
        }
    }
    inputs
}

/// Pack the params inputs into a byte buffer matching the reflected layout.
///
/// Missing or type-mismatched values fall back to the field's default, so the
/// result is always well-formed.
pub fn pack_params(schema: &ParamsSchema, inputs: &WispInputs) -> Vec<u8> {
    let mut bytes = vec![0u8; schema.size as usize];
    for field in &schema.fields {
        let default = default_value(field);
        let value = match inputs.0.get(&field.name) {
            Some(value) if value.same_kind(&default) => value.clone(),
            _ => default,
        };
        write_value(&mut bytes, field.offset as usize, &value);
    }
    bytes
}

/// The default value for a field, honouring `@default` (zero otherwise).
fn default_value(field: &ParamField) -> WispValue {
    let component = |i: usize| {
        field
            .ui
            .default
            .as_ref()
            .and_then(|d| d.get(i).copied())
            .unwrap_or(0.0)
    };
    match field.ty {
        ParamType::F32 => WispValue::F32(component(0) as f32),
        ParamType::I32 => WispValue::I32(component(0) as i32),
        ParamType::U32 => WispValue::U32(component(0) as u32),
        ParamType::Bool => WispValue::Bool(component(0) != 0.0),
        ParamType::Vec2 => WispValue::Vec2(Vec2::new(component(0) as f32, component(1) as f32)),
        ParamType::Vec3 => WispValue::Vec3(Vec3::new(
            component(0) as f32,
            component(1) as f32,
            component(2) as f32,
        )),
        ParamType::Vec4 => WispValue::Vec4(Vec4::new(
            component(0) as f32,
            component(1) as f32,
            component(2) as f32,
            component(3) as f32,
        )),
    }
}

fn write_value(bytes: &mut [u8], offset: usize, value: &WispValue) {
    let write = |bytes: &mut [u8], src: &[u8]| {
        bytes[offset..offset + src.len()].copy_from_slice(src);
    };
    match value {
        WispValue::Bool(b) => write(bytes, &u32::from(*b).to_ne_bytes()),
        WispValue::I32(v) => write(bytes, &v.to_ne_bytes()),
        WispValue::U32(v) => write(bytes, &v.to_ne_bytes()),
        WispValue::F32(v) => write(bytes, &v.to_ne_bytes()),
        WispValue::Vec2(v) => write(bytes, bytemuck::cast_slice(&v.to_array())),
        WispValue::Vec3(v) => write(bytes, bytemuck::cast_slice(&v.to_array())),
        WispValue::Vec4(v) => write(bytes, bytemuck::cast_slice(&v.to_array())),
        // Image inputs are bound as textures, not packed into the uniform.
        WispValue::Image(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::UiHints;

    fn field(name: &str, ty: ParamType, offset: u32, default: Option<Vec<f64>>) -> ParamField {
        ParamField {
            name: name.to_string(),
            ty,
            offset,
            ui: UiHints {
                default,
                ..Default::default()
            },
        }
    }

    fn test_schema() -> WispSchema {
        WispSchema {
            description: String::new(),
            globals: None,
            params: Some(ParamsSchema {
                name: "params".to_string(),
                size: 32,
                fields: vec![
                    field("level", ParamType::F32, 0, Some(vec![0.5])),
                    field("enabled", ParamType::Bool, 4, Some(vec![1.0])),
                    field("center", ParamType::Vec2, 8, None),
                    field("tint", ParamType::Vec4, 16, Some(vec![1.0, 0.5, 0.25, 1.0])),
                ],
            }),
            textures: Vec::new(),
            samplers: Vec::new(),
            passes: Vec::new(),
            bindings: Vec::new(),
        }
    }

    #[test]
    fn defaults_from_schema() {
        let inputs = inputs_from_schema(&test_schema());
        assert_eq!(inputs.0["level"], WispValue::F32(0.5));
        assert_eq!(inputs.0["enabled"], WispValue::Bool(true));
        assert_eq!(inputs.0["center"], WispValue::Vec2(Vec2::ZERO));
        assert_eq!(
            inputs.0["tint"],
            WispValue::Vec4(Vec4::new(1.0, 0.5, 0.25, 1.0))
        );
    }

    #[test]
    fn pack_byte_exact() {
        let schema = test_schema();
        let mut inputs = inputs_from_schema(&schema);
        inputs
            .0
            .insert("center".to_string(), WispValue::Vec2(Vec2::new(2.0, 3.0)));
        let bytes = pack_params(schema.params.as_ref().unwrap(), &inputs);
        assert_eq!(bytes.len(), 32);
        let floats: &[f32] = bytemuck::cast_slice(&bytes);
        // `enabled` is a bool packed as u32 1, which is f32-bit-pattern ~1e-45;
        // compare it as a u32 word instead.
        assert_eq!(floats[0], 0.5);
        let words: &[u32] = bytemuck::cast_slice(&bytes);
        assert_eq!(words[1], 1);
        assert_eq!(&floats[2..4], &[2.0, 3.0]);
        assert_eq!(&floats[4..8], &[1.0, 0.5, 0.25, 1.0]);
    }

    #[test]
    fn pack_falls_back_on_type_mismatch() {
        let schema = test_schema();
        let mut inputs = inputs_from_schema(&schema);
        inputs
            .0
            .insert("level".to_string(), WispValue::Vec4(Vec4::ONE));
        let bytes = pack_params(schema.params.as_ref().unwrap(), &inputs);
        let floats: &[f32] = bytemuck::cast_slice(&bytes);
        assert_eq!(floats[0], 0.5, "mismatched value falls back to the default");
    }

    #[test]
    fn rematch_preserves_matching_values() {
        let schema = test_schema();
        let mut old = inputs_from_schema(&schema);
        old.0.insert("level".to_string(), WispValue::F32(0.9));
        // Simulate the field changing type across a reload.
        old.0.insert("enabled".to_string(), WispValue::F32(3.0));
        let inputs = rematch_inputs(&old, &schema);
        assert_eq!(inputs.0["level"], WispValue::F32(0.9));
        assert_eq!(
            inputs.0["enabled"],
            WispValue::Bool(true),
            "type change resets to the default"
        );
    }
}
