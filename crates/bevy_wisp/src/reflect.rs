//! WGSL parsing, validation and analysis via `naga`.
//!
//! Wisp shaders are plain WGSL - the same source that bevy compiles is parsed here
//! (with doc comments retained) so that [`crate::schema`] can reflect the shader's
//! interface. Bevy hands WGSL to wgpu untouched (WESL imports and `@if`
//! conditional compilation apply only to `.wesl` shaders), so what wisp reflects
//! and what the GPU compiles never diverge, and error line numbers always match
//! the file on disk.

use naga::front::wgsl;
use naga::valid::{Capabilities, ModuleInfo, ValidationFlags, Validator};
use thiserror::Error;

/// A parsed and validated WGSL module along with its analysis info.
///
/// [`ModuleInfo`] provides per-entry-point global usage, used for binding visibility
/// and self-feedback detection.
pub struct ReflectedModule {
    pub module: naga::Module,
    pub info: ModuleInfo,
}

#[derive(Debug, Error)]
pub enum ReflectError {
    #[error("failed to parse WGSL:\n{0}")]
    Parse(String),
    #[error("invalid WGSL:\n{0}")]
    Validation(String),
}

/// Parse and validate wisp WGSL source.
pub fn parse_and_validate(source: &str) -> Result<ReflectedModule, ReflectError> {
    let options = wgsl::Options {
        parse_doc_comments: true,
        capabilities: Capabilities::all(),
    };
    let module = wgsl::Frontend::new_with_options(options)
        .parse(source)
        .map_err(|e| ReflectError::Parse(e.emit_to_string(source)))?;
    // Validation here is advisory - the GPU device validates again at pipeline
    // creation - so be permissive about capabilities.
    let info = Validator::new(ValidationFlags::all(), Capabilities::all())
        .validate(&module)
        .map_err(|e| ReflectError::Validation(e.emit_to_string(source)))?;
    Ok(ReflectedModule { module, info })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_error_includes_location() {
        let Err(ReflectError::Parse(msg)) = parse_and_validate("fn nope( {") else {
            panic!("expected parse error");
        };
        assert!(msg.contains("1:"), "expected a span in: {msg}");
    }

    #[test]
    fn doc_comments_are_retained() {
        let src = "
            struct Params {
                /// @min(0.0)
                level: f32,
            }
            @group(1) @binding(0) var<uniform> params: Params;
            @fragment
            fn fragment(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
                return vec4<f32>(params.level);
            }
        ";
        let reflected = parse_and_validate(src).unwrap();
        let docs = reflected.module.doc_comments.expect("doc comments parsed");
        assert!(
            docs.struct_members
                .values()
                .any(|lines| lines.iter().any(|l| l.contains("@min(0.0)")))
        );
    }
}
