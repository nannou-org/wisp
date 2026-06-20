//! Pad wisp's uniform structs to a 16-byte multiple before compilation.
//!
//! Devices without `DownlevelFlags::BUFFER_BINDINGS_NOT_16_BYTE_ALIGNED` -
//! notably WebGL2 - reject a uniform buffer binding whose *type* size is not a
//! multiple of 16 bytes. wisp's globals/params structs can be smaller (a single
//! `f32` is 4 bytes), so the pipeline would fail validation there and bevy's
//! render-error handler would quit the app.
//!
//! Before compiling, [`pad_uniform_structs`] appends trailing `f32` padding
//! members to each uniform struct (`@group(0)`/`@group(1)` `@binding(0)`) whose
//! size is not a multiple of 16. The schema rounds its reported size to match
//! (see [`crate::schema`]), so the buffer, the bind group layout and the
//! compiled shader all agree. Padding is harmless on devices that don't need it.
//!
//! Padding uses individual `f32` members rather than an array: WGSL requires
//! array elements in the uniform address space to have a 16-byte stride, so an
//! `array<f32, N>` would not pack tightly (or validate) here.

/// The prefix of every injected padding member. The members are unused by the
/// schema (reflection runs on the original, unpadded source).
const PAD_FIELD: &str = "_wisp_align_pad";

/// Append trailing `f32` padding to each `@binding(0)` uniform struct in
/// `@group(0)`/`@group(1)` whose byte size is not a multiple of 16, returning
/// the rewritten source. `module` must be the reflection of `source`.
pub(crate) fn pad_uniform_structs(source: &str, module: &naga::Module) -> String {
    let mut targets: Vec<(usize, u32)> = module
        .global_variables
        .iter()
        .filter_map(|(_, var)| pad_target(source, module, var))
        .collect();
    // Insert from the latest position first so earlier byte offsets stay valid.
    targets.sort_by_key(|&(close, _)| std::cmp::Reverse(close));
    let mut out = source.to_string();
    for (close, floats) in targets {
        let field: String = (0..floats)
            .map(|i| format!("\n    {PAD_FIELD}_{i}: f32,"))
            .collect();
        out.insert_str(close, &format!("{field}\n"));
    }
    out
}

/// The closing-brace byte index and `f32` padding count for a uniform struct
/// that needs padding, or `None` if it is already 16-byte aligned (or not a
/// wisp uniform).
fn pad_target(
    source: &str,
    module: &naga::Module,
    var: &naga::GlobalVariable,
) -> Option<(usize, u32)> {
    if var.space != naga::AddressSpace::Uniform {
        return None;
    }
    // wisp binds its uniforms at `@binding(0)` of `@group(0)` (globals) and
    // `@group(1)` (params); other uniforms are not ours to pad.
    let binding = var.binding.as_ref()?;
    if binding.binding != 0 || binding.group > 1 {
        return None;
    }
    let naga::TypeInner::Struct { span, .. } = module.types[var.ty].inner else {
        return None;
    };
    let remainder = span % 16;
    if remainder == 0 {
        return None;
    }
    // The struct's source span ends just past its closing `}`; the last `}`
    // within it is that brace (struct members contain no braces of their own).
    let range = module.types.get_span(var.ty).to_range()?;
    let close = source.get(..range.end)?.rfind('}')?;
    Some((close, (16 - remainder) / 4))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflect::parse_and_validate;

    /// The size of the `@group(g) @binding(0)` uniform struct in `source`.
    fn uniform_size(source: &str, group: u32) -> u32 {
        let module = parse_and_validate(source).unwrap().module;
        module
            .global_variables
            .iter()
            .find_map(|(_, var)| {
                let binding = var.binding.as_ref()?;
                (var.space == naga::AddressSpace::Uniform
                    && binding.group == group
                    && binding.binding == 0)
                    .then_some(())?;
                match module.types[var.ty].inner {
                    naga::TypeInner::Struct { span, .. } => Some(span),
                    _ => None,
                }
            })
            .unwrap()
    }

    const SHADER: &str = "\
struct Params {
    wobble: f32,
}
@group(1) @binding(0) var<uniform> params: Params;
@fragment
fn frag() -> @location(0) vec4<f32> {
    return vec4<f32>(params.wobble);
}
";

    #[test]
    fn pads_small_params_to_16() {
        let module = parse_and_validate(SHADER).unwrap().module;
        assert_eq!(uniform_size(SHADER, 1), 4, "single f32 starts at 4 bytes");
        let padded = pad_uniform_structs(SHADER, &module);
        // The padded source still parses and validates, now at 16 bytes.
        assert_eq!(uniform_size(&padded, 1), 16);
    }

    #[test]
    fn leaves_aligned_structs_untouched() {
        let source = "\
struct Params {
    tint: vec4<f32>,
}
@group(1) @binding(0) var<uniform> params: Params;
@fragment
fn frag() -> @location(0) vec4<f32> {
    return params.tint;
}
";
        let module = parse_and_validate(source).unwrap().module;
        assert_eq!(uniform_size(source, 1), 16);
        assert_eq!(pad_uniform_structs(source, &module), source);
    }
}
