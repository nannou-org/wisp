//! The reflected interface of a wisp shader.
//!
//! A [`WispSchema`] is built from a validated naga module and fully describes the
//! shader's interface: the globals/params uniforms, the textures and samplers it
//! declares, and the passes formed by its entry points. Downstream layers (render
//! pipeline, UI) work from the schema alone - naga types never leave this module
//! and [`crate::reflect`].
//!
//! The conventions enforced here:
//!
//! - `@group(0)` is wisp-provided: an optional globals uniform struct at
//!   `@binding(0)` (see [`crate::globals`]) and an optional sampler at `@binding(1)`.
//! - `@group(1)` is shader-declared: at most one uniform struct (the params, whose
//!   members become tweakable inputs), plus textures and samplers.
//! - Every `@fragment`/`@compute` entry point is a pass, executed in declaration
//!   order and configured by a `/// @pass(..)` doc-comment annotation. Exactly one
//!   `@fragment` entry point omits `target = ".."` - the final pass, rendering to
//!   the view. A pass's named target can be read by any pass as a `texture_2d<f32>`
//!   of the same name; a compute pass writes its target through a
//!   `texture_storage_2d` named `<target>_out`.

use crate::annot::{self, Annotation, Arg, Docs, Value};
use crate::globals::{GlobalKind, GlobalsSchema};
use crate::reflect::ReflectedModule;
use bevy::math::UVec2;
use bevy::render::render_resource::{ShaderStages, TextureFormat};
use naga::ir::DocComments;
use std::collections::BTreeMap;
use thiserror::Error;

/// The complete reflected interface of a wisp shader.
#[derive(Clone, Debug, PartialEq)]
pub struct WispSchema {
    /// Module-level doc comments (`//!`).
    pub description: String,
    /// The globals uniform struct at `@group(0) @binding(0)`, if declared.
    pub globals: Option<GlobalsSchema>,
    /// The params uniform struct in `@group(1)`, if declared.
    pub params: Option<ParamsSchema>,
    /// Every texture the shader declares, in `(group, binding)` order.
    pub textures: Vec<TextureSchema>,
    /// Every sampler the shader declares, as `(group, binding)`, sorted. All
    /// samplers are bound to the default filtering sampler.
    pub samplers: Vec<(u32, u32)>,
    /// The shader's passes, one per entry point, in declaration order.
    pub passes: Vec<PassSchema>,
    /// The full bind group interface, sorted by `(group, binding)`.
    pub bindings: Vec<BindingDesc>,
}

/// One entry in the shader's bind group interface.
#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct BindingDesc {
    pub group: u32,
    pub binding: u32,
    /// Union of the stages of every entry point that uses the binding.
    pub visibility: ShaderStages,
    pub ty: BindingTy,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub enum BindingTy {
    /// The wisp globals uniform, bound with a per-pass dynamic offset.
    Globals { size: u32 },
    /// The user params uniform.
    Params { size: u32 },
    /// A filterable 2d float texture.
    Texture2d,
    /// A filtering sampler.
    Sampler,
    /// A write-only 2d storage texture.
    StorageTexture2d { format: TextureFormat },
}

/// The reflected params uniform struct: the shader's tweakable inputs.
#[derive(Clone, Debug, PartialEq)]
pub struct ParamsSchema {
    /// The name of the uniform's global variable.
    pub name: String,
    /// Total size of the struct in bytes, including trailing padding.
    pub size: u32,
    pub fields: Vec<ParamField>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ParamField {
    pub name: String,
    pub ty: ParamType,
    /// Byte offset within the params uniform.
    pub offset: u32,
    pub ui: UiHints,
}

/// The WGSL type of a params struct member.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum ParamType {
    F32,
    I32,
    U32,
    /// A `u32` member annotated `@bool`, exposed as a toggle. (WGSL forbids `bool`
    /// in uniform structs.)
    Bool,
    Vec2,
    Vec3,
    Vec4,
}

/// UI hints parsed from a params member's doc-comment annotations.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct UiHints {
    /// Free doc-comment text, shown as a tooltip.
    pub description: String,
    /// `@label("..")` - display name override.
    pub label: Option<String>,
    /// `@min(..)`.
    pub min: Option<f64>,
    /// `@max(..)`.
    pub max: Option<f64>,
    /// `@step(..)`.
    pub step: Option<f64>,
    /// `@default(..)` components; length matches the field's arity.
    pub default: Option<Vec<f64>>,
    /// `@color` - expose a vec3/vec4 as a color picker.
    pub color: bool,
    /// `@values(..)` - restrict an integer to a set of values (dropdown).
    pub values: Vec<i64>,
    /// `@labels(..)` - display names for the `@values` entries.
    pub labels: Vec<String>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct TextureSchema {
    pub name: String,
    pub group: u32,
    pub binding: u32,
    pub role: TextureRole,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TextureRole {
    /// A user-supplied image, settable via [`crate::inputs::WispInputs`].
    ImageInput,
    /// Reads the target of the pass at this index in [`WispSchema::passes`].
    PassTarget { pass: usize },
    /// The storage texture through which a compute pass writes its target.
    StorageTarget { pass: usize },
    /// `@audio(samples = ..)` - waveform texture (requires the `audio` feature).
    AudioWaveform { samples: u32 },
    /// `@audio_fft(bins = ..)` - FFT magnitude texture (requires the `audio` feature).
    AudioFft { bins: u32 },
}

/// One pass: a `@fragment` or `@compute` entry point.
#[derive(Clone, Debug, PartialEq)]
pub struct PassSchema {
    /// The entry point name.
    pub entry: String,
    /// The entry point's index in the module.
    pub entry_index: usize,
    pub stage: PassStage,
    /// The named intermediate target, or `None` for the final pass.
    pub target: Option<TargetSchema>,
    /// Compute workgroup size (zeroes for fragment passes).
    pub workgroup_size: [u32; 3],
    /// `dispatch = ".."` expressions; when absent the dispatch size is derived from
    /// the target size and workgroup size.
    pub dispatch: Option<[String; 3]>,
    /// Whether the pass reads its own target (requires ping-pong buffering).
    pub self_feedback: bool,
    /// Free doc-comment text.
    pub description: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PassStage {
    Fragment,
    Compute,
}

/// Configuration of a pass's named intermediate target.
#[derive(Clone, Debug, PartialEq)]
pub struct TargetSchema {
    pub name: String,
    /// Keep the previous frame's contents instead of clearing.
    pub persistent: bool,
    /// `@pass(.., float)` - render at `Rgba16Float` precision.
    pub float: bool,
    pub format: TextureFormat,
    /// `$WIDTH`/`$HEIGHT` size expressions; the target size defaults to the view size.
    pub width: Option<String>,
    pub height: Option<String>,
}

#[derive(Debug, Error)]
pub enum SchemaError {
    #[error("{context}: {err}")]
    Annot {
        context: String,
        err: annot::AnnotError,
    },
    #[error("{context}: invalid arguments for `@{annotation}`: expected {expected}")]
    AnnotationArgs {
        context: String,
        annotation: &'static str,
        expected: &'static str,
    },
    #[error("unknown annotation `@{name}` on {context} (supported: {valid})")]
    UnknownAnnotation {
        context: String,
        name: String,
        valid: String,
    },
    #[error(
        "`{name}`: resources must be declared in `@group(1)` (`@group(0)` is \
         provided by wisp), found `@group({group})`"
    )]
    BadGroup { name: String, group: u32 },
    #[error(
        "`{name}`: `@group(0)` is reserved for the wisp globals uniform at \
         `@binding(0)` and an optional sampler at `@binding(1)`, found \
         `@binding({binding})`"
    )]
    Group0Binding { name: String, binding: u32 },
    #[error("`{name}`: uniform bindings must be structs")]
    UniformNotStruct { name: String },
    #[error("unknown globals member `{name}` (recognized: {valid})")]
    UnknownGlobal { name: String, valid: String },
    #[error("globals member `{name}` must be declared as `{expected}`")]
    GlobalType {
        name: String,
        expected: &'static str,
    },
    #[error("multiple params uniforms in `@group(1)`: `{first}` and `{second}`")]
    MultipleParams { first: String, second: String },
    #[error(
        "param `{field}` has an unsupported type (supported: f32, i32, u32, \
         vec2<f32>, vec3<f32>, vec4<f32>)"
    )]
    ParamType { field: String },
    #[error("{context}: `@bool` requires the member to be declared as `u32`")]
    BoolType { context: String },
    #[error("{context}: `@color` requires a `vec3<f32>` or `vec4<f32>` member")]
    ColorType { context: String },
    #[error("{context}: `@min`/`@max`/`@step` only apply to scalar members")]
    ScalarHint { context: String },
    #[error("{context}: `@default` expects {expected} component(s), found {found}")]
    DefaultArity {
        context: String,
        expected: usize,
        found: usize,
    },
    #[error("{context}: `@values` only applies to `i32`/`u32` members")]
    ValuesType { context: String },
    #[error("{context}: `@labels` must accompany `@values`, with one label per value")]
    LabelsMismatch { context: String },
    #[error("`{name}` is declared both as a param and as a texture input")]
    NameCollision { name: String },
    #[error(
        "entry point `{name}`: wisp provides the fullscreen vertex shader; only \
         `@fragment` and `@compute` entry points are supported"
    )]
    UnsupportedStage { name: String },
    #[error(
        "no final pass: exactly one `@fragment` entry point must omit \
         `@pass(target = ..)` so that it renders to the view"
    )]
    NoFinalPass,
    #[error("multiple final passes: `{first}` and `{second}` both omit `@pass(target = ..)`")]
    MultipleFinalPasses { first: String, second: String },
    #[error(
        "{context}: pass configuration (persistent/float/width/height/dispatch) \
         requires `target = \"..\"`"
    )]
    FinalPassConfig { context: String },
    #[error("{context}: `dispatch` only applies to `@compute` passes")]
    DispatchOnFragment { context: String },
    #[error(
        "{context}: unknown `@pass` argument `{arg}` (supported: target, \
         persistent, float, width, height, dispatch)"
    )]
    PassArg { context: String, arg: String },
    #[error("{context}: `dispatch` expects 1-3 comma-separated expressions")]
    DispatchArity { context: String },
    #[error("compute entry point `{entry}` requires `@pass(target = \"..\")`")]
    ComputeWithoutTarget { entry: String },
    #[error("multiple passes target `{name}`")]
    DuplicateTarget { name: String },
    #[error("two bindings declared at `@group({group}) @binding({binding})`")]
    DuplicateBinding { group: u32, binding: u32 },
    #[error(
        "compute pass `{entry}` targets `{target}` but declares no \
         `texture_storage_2d` named `{target}_out`"
    )]
    MissingStorageTexture { entry: String, target: String },
    #[error("storage texture `{name}` must be named `<target>_out` after a `@compute` pass target")]
    StorageTextureName { name: String },
    #[error("storage texture `{name}`: no `@compute` pass targets `{target}`")]
    StorageTextureOrphan { name: String, target: String },
    #[error("storage texture `{name}` must be declared with `write` access")]
    StorageAccess { name: String },
    #[error(
        "storage texture `{name}` must be declared `{expected}` to match its pass \
         (`rgba16float` for `float` targets, `rgba8unorm` otherwise)"
    )]
    StorageFormat {
        name: String,
        expected: &'static str,
    },
    #[error(
        "texture `{name}` has an unsupported type (only `texture_2d<f32>` and 2d \
         `texture_storage_2d` are supported)"
    )]
    UnsupportedTexture { name: String },
    #[error(
        "binding `{name}` has an unsupported type (storage buffers and comparison \
         samplers are not supported)"
    )]
    UnsupportedBinding { name: String },
    #[error("{context}: {err}")]
    SizeExpr { context: String, err: SizeExprError },
}

#[derive(Debug, Error)]
pub enum SizeExprError {
    #[error("failed to evaluate size expression `{expr}`: {err}")]
    Eval { expr: String, err: String },
    #[error("size expression `{expr}` must yield a positive number")]
    NonPositive { expr: String },
}

/// Annotations recognized on params struct members.
const MEMBER_ANNOTATIONS: &[&str] = &[
    "min", "max", "step", "default", "color", "label", "values", "labels", "bool",
];
/// Annotations recognized on entry points.
const ENTRY_ANNOTATIONS: &[&str] = &["pass"];
/// Annotations recognized on texture globals.
const TEXTURE_ANNOTATIONS: &[&str] = &["audio", "audio_fft"];

impl ParamType {
    /// The number of scalar components.
    pub fn arity(self) -> usize {
        match self {
            Self::F32 | Self::I32 | Self::U32 | Self::Bool => 1,
            Self::Vec2 => 2,
            Self::Vec3 => 3,
            Self::Vec4 => 4,
        }
    }

    pub fn is_scalar(self) -> bool {
        self.arity() == 1
    }

    /// The WGSL name of the type, for error messages.
    pub fn wgsl_name(self) -> &'static str {
        match self {
            Self::F32 => "f32",
            Self::I32 => "i32",
            Self::U32 | Self::Bool => "u32",
            Self::Vec2 => "vec2<f32>",
            Self::Vec3 => "vec3<f32>",
            Self::Vec4 => "vec4<f32>",
        }
    }
}

impl PassStage {
    pub fn shader_stages(self) -> ShaderStages {
        match self {
            Self::Fragment => ShaderStages::FRAGMENT,
            Self::Compute => ShaderStages::COMPUTE,
        }
    }
}

/// Build a [`WispSchema`] from a parsed and validated module.
pub fn schema_from_module(reflected: &ReflectedModule) -> Result<WispSchema, SchemaError> {
    let ReflectedModule { module, info } = reflected;
    let docs = module.doc_comments.as_deref();

    let description = docs
        .map(|d| annot::clean_text(&d.module))
        .unwrap_or_default();
    let mut passes = passes_from_entry_points(module, docs)?;
    let target_passes: BTreeMap<String, usize> = passes
        .iter()
        .enumerate()
        .filter_map(|(i, pass)| Some((pass.target.as_ref()?.name.clone(), i)))
        .collect();

    let mut globals = None;
    let mut params: Option<ParamsSchema> = None;
    let mut textures: Vec<(naga::Handle<naga::GlobalVariable>, TextureSchema)> = Vec::new();
    let mut samplers: Vec<(u32, u32)> = Vec::new();
    let mut raw_bindings: Vec<(naga::Handle<naga::GlobalVariable>, u32, u32, BindingTy)> =
        Vec::new();

    for (handle, var) in module.global_variables.iter() {
        let Some(resource) = &var.binding else {
            continue;
        };
        let (group, binding) = (resource.group, resource.binding);
        if raw_bindings
            .iter()
            .any(|&(_, g, b, _)| (g, b) == (group, binding))
        {
            return Err(SchemaError::DuplicateBinding { group, binding });
        }
        let name = var
            .name
            .clone()
            .unwrap_or_else(|| format!("binding {group}:{binding}"));
        let ty = match var.space {
            naga::AddressSpace::Uniform => {
                let naga::TypeInner::Struct { ref members, span } = module.types[var.ty].inner
                else {
                    return Err(SchemaError::UniformNotStruct { name });
                };
                match group {
                    0 => {
                        if binding != 0 {
                            return Err(SchemaError::Group0Binding { name, binding });
                        }
                        globals = Some(globals_schema(module, var.ty, members, span, docs)?);
                        BindingTy::Globals { size: span }
                    }
                    1 => {
                        if let Some(first) = &params {
                            return Err(SchemaError::MultipleParams {
                                first: first.name.clone(),
                                second: name,
                            });
                        }
                        params = Some(params_schema(module, var.ty, &name, members, span, docs)?);
                        BindingTy::Params { size: span }
                    }
                    _ => return Err(SchemaError::BadGroup { name, group }),
                }
            }
            naga::AddressSpace::Handle => match module.types[var.ty].inner {
                naga::TypeInner::Image {
                    dim: naga::ImageDimension::D2,
                    arrayed: false,
                    class,
                } => {
                    if group != 1 {
                        return Err(SchemaError::BadGroup { name, group });
                    }
                    match class {
                        naga::ImageClass::Sampled {
                            kind: naga::ScalarKind::Float,
                            multi: false,
                        } => {
                            let raw = docs
                                .and_then(|d| d.global_variables.get(&handle))
                                .map(Vec::as_slice)
                                .unwrap_or(&[]);
                            let role = texture_role(&name, raw, &target_passes)?;
                            textures.push((
                                handle,
                                TextureSchema {
                                    name,
                                    group,
                                    binding,
                                    role,
                                },
                            ));
                            BindingTy::Texture2d
                        }
                        naga::ImageClass::Storage { format, access } => {
                            let (pass, format) =
                                storage_target(&name, format, access, &target_passes, &mut passes)?;
                            textures.push((
                                handle,
                                TextureSchema {
                                    name,
                                    group,
                                    binding,
                                    role: TextureRole::StorageTarget { pass },
                                },
                            ));
                            BindingTy::StorageTexture2d { format }
                        }
                        _ => return Err(SchemaError::UnsupportedTexture { name }),
                    }
                }
                naga::TypeInner::Image { .. } => {
                    return Err(SchemaError::UnsupportedTexture { name });
                }
                naga::TypeInner::Sampler { comparison } => {
                    if comparison {
                        return Err(SchemaError::UnsupportedBinding { name });
                    }
                    match group {
                        0 if binding != 1 => {
                            return Err(SchemaError::Group0Binding { name, binding });
                        }
                        0 | 1 => {}
                        _ => return Err(SchemaError::BadGroup { name, group }),
                    }
                    samplers.push((group, binding));
                    BindingTy::Sampler
                }
                _ => return Err(SchemaError::UnsupportedBinding { name }),
            },
            _ => return Err(SchemaError::UnsupportedBinding { name }),
        };
        raw_bindings.push((handle, group, binding, ty));
    }

    // Every compute pass must write its target through a `<target>_out` storage texture.
    for (index, pass) in passes.iter().enumerate() {
        let Some(target) = &pass.target else { continue };
        let written = textures
            .iter()
            .any(|(_, t)| matches!(t.role, TextureRole::StorageTarget { pass } if pass == index));
        if pass.stage == PassStage::Compute && !written {
            return Err(SchemaError::MissingStorageTexture {
                entry: pass.entry.clone(),
                target: target.name.clone(),
            });
        }
    }

    // A pass reading its own target needs ping-pong buffering.
    for (index, pass) in passes.iter_mut().enumerate() {
        if pass.target.is_none() {
            continue;
        }
        let uses = info.get_entry_point(index);
        pass.self_feedback = textures.iter().any(|&(handle, ref t)| {
            matches!(t.role, TextureRole::PassTarget { pass } if pass == index)
                && !uses[handle].is_empty()
        });
    }

    // Params and image inputs share the `WispInputs` namespace.
    if let Some(params) = &params {
        for field in &params.fields {
            let collision = textures
                .iter()
                .any(|(_, t)| t.name == field.name && matches!(t.role, TextureRole::ImageInput));
            if collision {
                return Err(SchemaError::NameCollision {
                    name: field.name.clone(),
                });
            }
        }
    }

    // Per-binding visibility: the union of stages of the entry points using it.
    // Unused bindings get every stage present - extra layout entries are harmless.
    let all_stages = passes.iter().fold(ShaderStages::NONE, |acc, pass| {
        acc | pass.stage.shader_stages()
    });
    let mut bindings: Vec<BindingDesc> = raw_bindings
        .into_iter()
        .map(|(handle, group, binding, ty)| {
            let mut visibility = ShaderStages::NONE;
            for (index, pass) in passes.iter().enumerate() {
                if !info.get_entry_point(index)[handle].is_empty() {
                    visibility |= pass.stage.shader_stages();
                }
            }
            if visibility.is_empty() {
                visibility = all_stages;
            }
            BindingDesc {
                group,
                binding,
                visibility,
                ty,
            }
        })
        .collect();
    bindings.sort_by_key(|b| (b.group, b.binding));
    let mut textures: Vec<TextureSchema> = textures.into_iter().map(|(_, t)| t).collect();
    textures.sort_by_key(|t| (t.group, t.binding));
    samplers.sort_unstable();

    Ok(WispSchema {
        description,
        globals,
        params,
        textures,
        samplers,
        passes,
        bindings,
    })
}

/// Evaluate a `$WIDTH`/`$HEIGHT` size expression against the given base size.
pub fn eval_size(expr: &str, base: UVec2) -> Result<u32, SizeExprError> {
    let resolved = expr
        .replace("$WIDTH", &base.x.to_string())
        .replace("$HEIGHT", &base.y.to_string());
    let value = match evalexpr::eval(&resolved) {
        Ok(evalexpr::Value::Int(i)) => i as f64,
        Ok(evalexpr::Value::Float(f)) => f,
        Ok(other) => {
            return Err(SizeExprError::Eval {
                expr: expr.to_string(),
                err: format!("expected a number, found `{other}`"),
            });
        }
        Err(err) => {
            return Err(SizeExprError::Eval {
                expr: expr.to_string(),
                err: err.to_string(),
            });
        }
    };
    if !value.is_finite() || value < 1.0 {
        return Err(SizeExprError::NonPositive {
            expr: expr.to_string(),
        });
    }
    Ok(value as u32)
}

fn passes_from_entry_points(
    module: &naga::Module,
    docs: Option<&DocComments>,
) -> Result<Vec<PassSchema>, SchemaError> {
    let mut passes: Vec<PassSchema> = Vec::new();
    let mut final_pass: Option<usize> = None;
    let mut targets: BTreeMap<String, usize> = BTreeMap::new();
    for (index, ep) in module.entry_points.iter().enumerate() {
        let context = format!("entry point `{}`", ep.name);
        let raw = docs
            .and_then(|d| d.entry_points.get(&index))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let parsed = annot::parse_docs(raw).map_err(|err| SchemaError::Annot {
            context: context.clone(),
            err,
        })?;
        check_known(&parsed, ENTRY_ANNOTATIONS, &context)?;
        let stage = match ep.stage {
            naga::ShaderStage::Fragment => PassStage::Fragment,
            naga::ShaderStage::Compute => PassStage::Compute,
            _ => {
                return Err(SchemaError::UnsupportedStage {
                    name: ep.name.clone(),
                });
            }
        };
        let (target, dispatch) = match parsed.get("pass") {
            None => (None, None),
            Some(annotation) => pass_config(annotation, &context, stage)?,
        };
        // Catch malformed size expressions at load time rather than at render time.
        let exprs = target
            .iter()
            .flat_map(|t| [t.width.as_deref(), t.height.as_deref()])
            .chain(
                dispatch
                    .iter()
                    .flat_map(|d| d.iter().map(|e| Some(e.as_str()))),
            )
            .flatten();
        for expr in exprs {
            eval_size(expr, UVec2::new(1920, 1080)).map_err(|err| SchemaError::SizeExpr {
                context: context.clone(),
                err,
            })?;
        }
        match (stage, &target) {
            (PassStage::Compute, None) => {
                return Err(SchemaError::ComputeWithoutTarget {
                    entry: ep.name.clone(),
                });
            }
            (PassStage::Fragment, None) => {
                if let Some(first) = final_pass {
                    return Err(SchemaError::MultipleFinalPasses {
                        first: passes[first].entry.clone(),
                        second: ep.name.clone(),
                    });
                }
                final_pass = Some(index);
            }
            (_, Some(target)) => {
                if targets.insert(target.name.clone(), index).is_some() {
                    return Err(SchemaError::DuplicateTarget {
                        name: target.name.clone(),
                    });
                }
            }
        }
        passes.push(PassSchema {
            entry: ep.name.clone(),
            entry_index: index,
            stage,
            target,
            workgroup_size: ep.workgroup_size,
            dispatch,
            self_feedback: false,
            description: parsed.description,
        });
    }
    if final_pass.is_none() {
        return Err(SchemaError::NoFinalPass);
    }
    Ok(passes)
}

/// Interpret a `@pass(..)` annotation's arguments.
fn pass_config(
    annotation: &Annotation,
    context: &str,
    stage: PassStage,
) -> Result<(Option<TargetSchema>, Option<[String; 3]>), SchemaError> {
    let mut target_name = None;
    let mut persistent = false;
    let mut float = false;
    let mut width = None;
    let mut height = None;
    let mut dispatch = None;
    for arg in &annotation.args {
        match arg {
            Arg::Pos(Value::Ident(flag)) if flag == "persistent" => persistent = true,
            Arg::Pos(Value::Ident(flag)) if flag == "float" => float = true,
            Arg::Named(key, value) => match key.as_str() {
                "target" => match value.as_str() {
                    Some(name) => target_name = Some(name.to_string()),
                    None => {
                        return Err(SchemaError::AnnotationArgs {
                            context: context.to_string(),
                            annotation: "pass",
                            expected: "target = \"<name>\"",
                        });
                    }
                },
                "width" => width = Some(expr_string(value)),
                "height" => height = Some(expr_string(value)),
                "dispatch" => dispatch = Some(dispatch_exprs(value, context)?),
                _ => {
                    return Err(SchemaError::PassArg {
                        context: context.to_string(),
                        arg: key.clone(),
                    });
                }
            },
            Arg::Pos(value) => {
                return Err(SchemaError::PassArg {
                    context: context.to_string(),
                    arg: value.as_str().unwrap_or("<number>").to_string(),
                });
            }
        }
    }
    let Some(name) = target_name else {
        if persistent || float || width.is_some() || height.is_some() || dispatch.is_some() {
            return Err(SchemaError::FinalPassConfig {
                context: context.to_string(),
            });
        }
        return Ok((None, None));
    };
    if stage == PassStage::Fragment && dispatch.is_some() {
        return Err(SchemaError::DispatchOnFragment {
            context: context.to_string(),
        });
    }
    let format = match (stage, float) {
        (_, true) => TextureFormat::Rgba16Float,
        (PassStage::Fragment, false) => TextureFormat::Rgba8UnormSrgb,
        (PassStage::Compute, false) => TextureFormat::Rgba8Unorm,
    };
    let target = TargetSchema {
        name,
        persistent,
        float,
        format,
        width,
        height,
    };
    Ok((Some(target), dispatch))
}

/// A `width`/`height`/`dispatch` value as an expression string.
fn expr_string(value: &Value) -> String {
    match value {
        Value::Number(n) => n.to_string(),
        Value::Str(s) | Value::Ident(s) => s.clone(),
    }
}

fn dispatch_exprs(value: &Value, context: &str) -> Result<[String; 3], SchemaError> {
    let exprs: Vec<String> = expr_string(value)
        .split(',')
        .map(|s| s.trim().to_string())
        .collect();
    if exprs.is_empty() || exprs.len() > 3 || exprs.iter().any(String::is_empty) {
        return Err(SchemaError::DispatchArity {
            context: context.to_string(),
        });
    }
    let mut iter = exprs
        .into_iter()
        .chain(std::iter::repeat(String::from("1")));
    Ok(std::array::from_fn(|_| {
        iter.next().expect("repeat iterator is infinite")
    }))
}

fn globals_schema(
    module: &naga::Module,
    struct_ty: naga::Handle<naga::Type>,
    members: &[naga::StructMember],
    span: u32,
    docs: Option<&DocComments>,
) -> Result<GlobalsSchema, SchemaError> {
    let mut fields = Vec::new();
    for (index, member) in members.iter().enumerate() {
        let name = member.name.clone().unwrap_or_default();
        let valid = || GlobalKind::ALL.map(GlobalKind::name).join(", ");
        let kind = GlobalKind::from_name(&name).ok_or_else(|| SchemaError::UnknownGlobal {
            name: name.clone(),
            valid: valid(),
        })?;
        if param_type_of(module, member.ty) != Some(kind.param_ty()) {
            return Err(SchemaError::GlobalType {
                name,
                expected: kind.param_ty().wgsl_name(),
            });
        }
        // Annotations carry no meaning on globals members - reject them so typos
        // (e.g. a misplaced `@default`) don't pass silently.
        if let Some(raw) = docs.and_then(|d| d.struct_members.get(&(struct_ty, index))) {
            let context = format!("globals member `{name}`");
            let parsed = annot::parse_docs(raw).map_err(|err| SchemaError::Annot {
                context: context.clone(),
                err,
            })?;
            check_known(&parsed, &[], &context)?;
        }
        fields.push((kind, member.offset));
    }
    Ok(GlobalsSchema { size: span, fields })
}

fn params_schema(
    module: &naga::Module,
    struct_ty: naga::Handle<naga::Type>,
    var_name: &str,
    members: &[naga::StructMember],
    span: u32,
    docs: Option<&DocComments>,
) -> Result<ParamsSchema, SchemaError> {
    let mut fields = Vec::new();
    for (index, member) in members.iter().enumerate() {
        let name = member.name.clone().unwrap_or_default();
        let context = format!("param `{name}`");
        let raw = docs
            .and_then(|d| d.struct_members.get(&(struct_ty, index)))
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        let parsed = annot::parse_docs(raw).map_err(|err| SchemaError::Annot {
            context: context.clone(),
            err,
        })?;
        check_known(&parsed, MEMBER_ANNOTATIONS, &context)?;
        let base = param_type_of(module, member.ty).ok_or_else(|| SchemaError::ParamType {
            field: name.clone(),
        })?;
        let ty = match parsed.get("bool") {
            None => base,
            Some(annotation) => {
                if !annotation.args.is_empty() {
                    return Err(SchemaError::AnnotationArgs {
                        context,
                        annotation: "bool",
                        expected: "no arguments",
                    });
                }
                if base != ParamType::U32 {
                    return Err(SchemaError::BoolType { context });
                }
                ParamType::Bool
            }
        };
        let ui = ui_hints(&parsed, ty, &context)?;
        fields.push(ParamField {
            name,
            ty,
            offset: member.offset,
            ui,
        });
    }
    Ok(ParamsSchema {
        name: var_name.to_string(),
        size: span,
        fields,
    })
}

fn ui_hints(parsed: &Docs, ty: ParamType, context: &str) -> Result<UiHints, SchemaError> {
    let scalar_hint = |name: &'static str| -> Result<Option<f64>, SchemaError> {
        let Some(annotation) = parsed.get(name) else {
            return Ok(None);
        };
        if !ty.is_scalar() {
            return Err(SchemaError::ScalarHint {
                context: context.to_string(),
            });
        }
        annotation
            .single_number()
            .map(Some)
            .ok_or_else(|| SchemaError::AnnotationArgs {
                context: context.to_string(),
                annotation: name,
                expected: "a single number",
            })
    };
    let min = scalar_hint("min")?;
    let max = scalar_hint("max")?;
    let step = scalar_hint("step")?;
    let label = match parsed.get("label") {
        None => None,
        Some(annotation) => {
            Some(
                annotation
                    .single_string()
                    .ok_or_else(|| SchemaError::AnnotationArgs {
                        context: context.to_string(),
                        annotation: "label",
                        expected: "a single string",
                    })?,
            )
        }
    };
    let color = match parsed.get("color") {
        None => false,
        Some(annotation) => {
            if !annotation.args.is_empty() {
                return Err(SchemaError::AnnotationArgs {
                    context: context.to_string(),
                    annotation: "color",
                    expected: "no arguments",
                });
            }
            if !matches!(ty, ParamType::Vec3 | ParamType::Vec4) {
                return Err(SchemaError::ColorType {
                    context: context.to_string(),
                });
            }
            true
        }
    };
    let default = match parsed.get("default") {
        None => None,
        Some(annotation) => {
            let components =
                annotation
                    .pos_numbers()
                    .ok_or_else(|| SchemaError::AnnotationArgs {
                        context: context.to_string(),
                        annotation: "default",
                        expected: "numbers",
                    })?;
            if components.len() != ty.arity() {
                return Err(SchemaError::DefaultArity {
                    context: context.to_string(),
                    expected: ty.arity(),
                    found: components.len(),
                });
            }
            Some(components)
        }
    };
    let values: Vec<i64> = match parsed.get("values") {
        None => Vec::new(),
        Some(annotation) => {
            if !matches!(ty, ParamType::I32 | ParamType::U32) {
                return Err(SchemaError::ValuesType {
                    context: context.to_string(),
                });
            }
            annotation
                .pos_numbers()
                .ok_or_else(|| SchemaError::AnnotationArgs {
                    context: context.to_string(),
                    annotation: "values",
                    expected: "numbers",
                })?
                .into_iter()
                .map(|n| n as i64)
                .collect()
        }
    };
    let labels = match parsed.get("labels") {
        None => Vec::new(),
        Some(annotation) => {
            let labels = annotation
                .pos_strings()
                .ok_or_else(|| SchemaError::AnnotationArgs {
                    context: context.to_string(),
                    annotation: "labels",
                    expected: "strings",
                })?;
            if values.is_empty() || labels.len() != values.len() {
                return Err(SchemaError::LabelsMismatch {
                    context: context.to_string(),
                });
            }
            labels
        }
    };
    Ok(UiHints {
        description: parsed.description.clone(),
        label,
        min,
        max,
        step,
        default,
        color,
        values,
        labels,
    })
}

fn texture_role(
    name: &str,
    raw: &[String],
    target_passes: &BTreeMap<String, usize>,
) -> Result<TextureRole, SchemaError> {
    let context = format!("texture `{name}`");
    let parsed = annot::parse_docs(raw).map_err(|err| SchemaError::Annot {
        context: context.clone(),
        err,
    })?;
    check_known(&parsed, TEXTURE_ANNOTATIONS, &context)?;
    match (parsed.get("audio"), parsed.get("audio_fft")) {
        (Some(_), Some(_)) => Err(SchemaError::AnnotationArgs {
            context,
            annotation: "audio",
            expected: "either `@audio` or `@audio_fft`, not both",
        }),
        (Some(annotation), None) => Ok(TextureRole::AudioWaveform {
            samples: named_u32(annotation, "samples", 512, &context)?,
        }),
        (None, Some(annotation)) => Ok(TextureRole::AudioFft {
            bins: named_u32(annotation, "bins", 256, &context)?,
        }),
        (None, None) => match target_passes.get(name) {
            Some(&pass) => Ok(TextureRole::PassTarget { pass }),
            None => Ok(TextureRole::ImageInput),
        },
    }
}

/// The sole `key = <positive integer>` argument of an annotation, or a default
/// when no arguments are given.
fn named_u32(
    annotation: &Annotation,
    key: &'static str,
    default: u32,
    context: &str,
) -> Result<u32, SchemaError> {
    let err = || SchemaError::AnnotationArgs {
        context: context.to_string(),
        annotation: key,
        expected: "a positive integer",
    };
    if annotation.args.is_empty() {
        return Ok(default);
    }
    if annotation.args.len() != 1 {
        return Err(err());
    }
    let n = annotation
        .named(key)
        .and_then(Value::as_number)
        .ok_or_else(err)?;
    if n < 1.0 || n.fract() != 0.0 || n > u32::MAX as f64 {
        return Err(err());
    }
    Ok(n as u32)
}

fn storage_target(
    name: &str,
    format: naga::StorageFormat,
    access: naga::StorageAccess,
    target_passes: &BTreeMap<String, usize>,
    passes: &mut [PassSchema],
) -> Result<(usize, TextureFormat), SchemaError> {
    let target = name
        .strip_suffix("_out")
        .ok_or_else(|| SchemaError::StorageTextureName {
            name: name.to_string(),
        })?;
    let orphan = || SchemaError::StorageTextureOrphan {
        name: name.to_string(),
        target: target.to_string(),
    };
    let &pass = target_passes.get(target).ok_or_else(orphan)?;
    if passes[pass].stage != PassStage::Compute {
        return Err(orphan());
    }
    if access != naga::StorageAccess::STORE {
        return Err(SchemaError::StorageAccess {
            name: name.to_string(),
        });
    }
    let Some(target_schema) = passes[pass].target.as_mut() else {
        return Err(orphan());
    };
    let (expected, expected_name) = match target_schema.float {
        true => (TextureFormat::Rgba16Float, "rgba16float"),
        false => (TextureFormat::Rgba8Unorm, "rgba8unorm"),
    };
    let declared = match format {
        naga::StorageFormat::Rgba8Unorm => TextureFormat::Rgba8Unorm,
        naga::StorageFormat::Rgba16Float => TextureFormat::Rgba16Float,
        _ => {
            return Err(SchemaError::StorageFormat {
                name: name.to_string(),
                expected: expected_name,
            });
        }
    };
    if declared != expected {
        return Err(SchemaError::StorageFormat {
            name: name.to_string(),
            expected: expected_name,
        });
    }
    Ok((pass, expected))
}

fn check_known(parsed: &Docs, known: &[&str], context: &str) -> Result<(), SchemaError> {
    for annotation in &parsed.annotations {
        if !known.contains(&annotation.name.as_str()) {
            return Err(SchemaError::UnknownAnnotation {
                context: context.to_string(),
                name: annotation.name.clone(),
                valid: match known.is_empty() {
                    true => String::from("none"),
                    false => known.join(", "),
                },
            });
        }
    }
    Ok(())
}

fn param_type_of(module: &naga::Module, ty: naga::Handle<naga::Type>) -> Option<ParamType> {
    match module.types[ty].inner {
        naga::TypeInner::Scalar(scalar) => scalar_param_type(scalar),
        naga::TypeInner::Vector { size, scalar }
            if scalar_param_type(scalar) == Some(ParamType::F32) =>
        {
            match size {
                naga::VectorSize::Bi => Some(ParamType::Vec2),
                naga::VectorSize::Tri => Some(ParamType::Vec3),
                naga::VectorSize::Quad => Some(ParamType::Vec4),
            }
        }
        _ => None,
    }
}

fn scalar_param_type(scalar: naga::Scalar) -> Option<ParamType> {
    match (scalar.kind, scalar.width) {
        (naga::ScalarKind::Float, 4) => Some(ParamType::F32),
        (naga::ScalarKind::Sint, 4) => Some(ParamType::I32),
        (naga::ScalarKind::Uint, 4) => Some(ParamType::U32),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reflect::parse_and_validate;

    fn schema(src: &str) -> WispSchema {
        schema_from_module(&parse_and_validate(src).unwrap()).unwrap()
    }

    fn schema_err(src: &str) -> SchemaError {
        schema_from_module(&parse_and_validate(src).unwrap()).unwrap_err()
    }

    const FINAL_PASS: &str = "
        @fragment
        fn fragment(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
            return vec4<f32>(uv, 0.0, 1.0);
        }
    ";

    #[test]
    fn params_offsets_and_size() {
        let src = format!(
            "
            struct Params {{
                a: f32,
                b: vec2<f32>,
                c: vec4<f32>,
            }}
            @group(1) @binding(0) var<uniform> params: Params;
            {FINAL_PASS}
            "
        );
        let s = schema(&src);
        let params = s.params.unwrap();
        assert_eq!(params.size, 32);
        let layout: Vec<_> = params
            .fields
            .iter()
            .map(|f| (f.name.as_str(), f.ty, f.offset))
            .collect();
        assert_eq!(
            layout,
            vec![
                ("a", ParamType::F32, 0),
                ("b", ParamType::Vec2, 8),
                ("c", ParamType::Vec4, 16),
            ]
        );
        assert_eq!(s.passes.len(), 1);
        assert_eq!(s.passes[0].stage, PassStage::Fragment);
        assert!(s.passes[0].target.is_none());
        assert_eq!(
            s.bindings,
            vec![BindingDesc {
                group: 1,
                binding: 0,
                visibility: ShaderStages::FRAGMENT,
                ty: BindingTy::Params { size: 32 },
            }]
        );
    }

    #[test]
    fn member_annotations() {
        let src = format!(
            r#"
            struct Params {{
                /// Overall strength. @min(0.0) @max(1.0) @step(0.01) @default(0.5)
                level: f32,
                /// @color @default(1.0, 0.0, 0.0, 1.0) @label("Tint")
                tint: vec4<f32>,
                /// @bool @default(1)
                enabled: u32,
                /// @values(1, 2, 4) @labels("one", "two", "four")
                subdiv: i32,
            }}
            @group(1) @binding(0) var<uniform> params: Params;
            {FINAL_PASS}
            "#
        );
        let params = schema(&src).params.unwrap();
        let level = &params.fields[0];
        assert_eq!(level.ui.description, "Overall strength.");
        assert_eq!(
            (level.ui.min, level.ui.max, level.ui.step),
            (Some(0.0), Some(1.0), Some(0.01))
        );
        assert_eq!(level.ui.default, Some(vec![0.5]));
        let tint = &params.fields[1];
        assert!(tint.ui.color);
        assert_eq!(tint.ui.label.as_deref(), Some("Tint"));
        assert_eq!(tint.ui.default, Some(vec![1.0, 0.0, 0.0, 1.0]));
        let enabled = &params.fields[2];
        assert_eq!(enabled.ty, ParamType::Bool);
        let subdiv = &params.fields[3];
        assert_eq!(subdiv.ui.values, vec![1, 2, 4]);
        assert_eq!(subdiv.ui.labels, vec!["one", "two", "four"]);
    }

    #[test]
    fn annotation_errors() {
        let unknown = format!(
            "
            struct Params {{
                /// @mni(0.0)
                level: f32,
            }}
            @group(1) @binding(0) var<uniform> params: Params;
            {FINAL_PASS}
            "
        );
        assert!(matches!(
            schema_err(&unknown),
            SchemaError::UnknownAnnotation { name, .. } if name == "mni"
        ));

        let arity = format!(
            "
            struct Params {{
                /// @default(1.0, 2.0)
                level: f32,
            }}
            @group(1) @binding(0) var<uniform> params: Params;
            {FINAL_PASS}
            "
        );
        assert!(matches!(
            schema_err(&arity),
            SchemaError::DefaultArity {
                expected: 1,
                found: 2,
                ..
            }
        ));

        let color = format!(
            "
            struct Params {{
                /// @color
                level: f32,
            }}
            @group(1) @binding(0) var<uniform> params: Params;
            {FINAL_PASS}
            "
        );
        assert!(matches!(schema_err(&color), SchemaError::ColorType { .. }));

        let boolean = format!(
            "
            struct Params {{
                /// @bool
                level: f32,
            }}
            @group(1) @binding(0) var<uniform> params: Params;
            {FINAL_PASS}
            "
        );
        assert!(matches!(schema_err(&boolean), SchemaError::BoolType { .. }));
    }

    #[test]
    fn globals_subset_and_errors() {
        let src = format!(
            "
            struct Globals {{
                resolution: vec2<f32>,
                time: f32,
            }}
            @group(0) @binding(0) var<uniform> globals: Globals;
            {FINAL_PASS}
            "
        );
        let globals = schema(&src).globals.unwrap();
        assert_eq!(
            globals.fields,
            vec![(GlobalKind::Resolution, 0), (GlobalKind::Time, 8)]
        );

        let unknown = format!(
            "
            struct Globals {{ vibes: f32, }}
            @group(0) @binding(0) var<uniform> globals: Globals;
            {FINAL_PASS}
            "
        );
        assert!(matches!(
            schema_err(&unknown),
            SchemaError::UnknownGlobal { name, .. } if name == "vibes"
        ));

        let wrong_ty = format!(
            "
            struct Globals {{ time: vec2<f32>, }}
            @group(0) @binding(0) var<uniform> globals: Globals;
            {FINAL_PASS}
            "
        );
        assert!(matches!(
            schema_err(&wrong_ty),
            SchemaError::GlobalType { name, expected } if name == "time" && expected == "f32"
        ));
    }

    #[test]
    fn texture_classification_and_multipass() {
        let src = r#"
            @group(0) @binding(1) var samp: sampler;
            @group(1) @binding(1) var input_image: texture_2d<f32>;
            @group(1) @binding(2) var buffer_a: texture_2d<f32>;

            /// Feedback accumulator.
            /// @pass(target = "buffer_a", persistent, float, width = "$WIDTH/2", height = "$HEIGHT/2")
            @fragment
            fn feedback(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
                return textureSample(buffer_a, samp, uv) * 0.95
                    + textureSample(input_image, samp, uv) * 0.05;
            }

            @fragment
            fn fragment(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
                return textureSample(buffer_a, samp, uv);
            }
        "#;
        let s = schema(src);
        assert_eq!(s.passes.len(), 2);
        let feedback = &s.passes[0];
        assert_eq!(feedback.entry, "feedback");
        assert_eq!(feedback.description, "Feedback accumulator.");
        let target = feedback.target.as_ref().unwrap();
        assert_eq!(target.name, "buffer_a");
        assert!(target.persistent);
        assert!(target.float);
        assert_eq!(target.format, TextureFormat::Rgba16Float);
        assert_eq!(target.width.as_deref(), Some("$WIDTH/2"));
        assert_eq!(target.height.as_deref(), Some("$HEIGHT/2"));
        assert!(
            feedback.self_feedback,
            "feedback pass samples its own target"
        );
        let final_pass = &s.passes[1];
        assert!(final_pass.target.is_none());
        assert!(!final_pass.self_feedback);

        let roles: Vec<_> = s
            .textures
            .iter()
            .map(|t| (t.name.as_str(), t.role.clone()))
            .collect();
        assert_eq!(
            roles,
            vec![
                ("input_image", TextureRole::ImageInput),
                ("buffer_a", TextureRole::PassTarget { pass: 0 }),
            ]
        );
        assert_eq!(s.samplers, vec![(0, 1)]);
    }

    #[test]
    fn compute_pass_and_storage_target() {
        let src = r#"
            @group(0) @binding(1) var samp: sampler;
            @group(1) @binding(1) var sim: texture_2d<f32>;
            @group(1) @binding(2) var sim_out: texture_storage_2d<rgba16float, write>;

            /// @pass(target = "sim", float, dispatch = "$WIDTH/8, $HEIGHT/8")
            @compute @workgroup_size(8, 8, 1)
            fn step_sim(@builtin(global_invocation_id) id: vec3<u32>) {
                let c = textureLoad(sim, vec2<i32>(id.xy), 0);
                textureStore(sim_out, vec2<i32>(id.xy), c);
            }

            @fragment
            fn present(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
                return textureSample(sim, samp, uv);
            }
        "#;
        let s = schema(src);
        let sim = &s.passes[0];
        assert_eq!(sim.stage, PassStage::Compute);
        assert_eq!(sim.workgroup_size, [8, 8, 1]);
        assert_eq!(
            sim.dispatch,
            Some([
                "$WIDTH/8".to_string(),
                "$HEIGHT/8".to_string(),
                "1".to_string()
            ])
        );
        assert!(sim.self_feedback, "compute pass reads its own target");
        assert_eq!(
            sim.target.as_ref().unwrap().format,
            TextureFormat::Rgba16Float
        );
        assert_eq!(s.textures[1].role, TextureRole::StorageTarget { pass: 0 });
        let storage = s
            .bindings
            .iter()
            .find(|b| (b.group, b.binding) == (1, 2))
            .unwrap();
        assert_eq!(
            storage.ty,
            BindingTy::StorageTexture2d {
                format: TextureFormat::Rgba16Float
            }
        );
        assert_eq!(storage.visibility, ShaderStages::COMPUTE);
        // `sim` is read by both stages.
        let sim_binding = s
            .bindings
            .iter()
            .find(|b| (b.group, b.binding) == (1, 1))
            .unwrap();
        assert_eq!(
            sim_binding.visibility,
            ShaderStages::FRAGMENT | ShaderStages::COMPUTE
        );
    }

    #[test]
    fn compute_errors() {
        let no_target = "
            @compute @workgroup_size(8, 8, 1)
            fn step_sim() {}
        ";
        assert!(matches!(
            schema_err(&format!("{no_target}{FINAL_PASS}")),
            SchemaError::ComputeWithoutTarget { .. }
        ));

        let missing_storage = format!(
            r#"
            /// @pass(target = "sim")
            @compute @workgroup_size(8, 8, 1)
            fn step_sim() {{}}
            {FINAL_PASS}
            "#
        );
        assert!(matches!(
            schema_err(&missing_storage),
            SchemaError::MissingStorageTexture { target, .. } if target == "sim"
        ));

        let bad_format = format!(
            r#"
            @group(1) @binding(1) var sim_out: texture_storage_2d<rgba8unorm, write>;
            /// @pass(target = "sim", float)
            @compute @workgroup_size(8, 8, 1)
            fn step_sim(@builtin(global_invocation_id) id: vec3<u32>) {{
                textureStore(sim_out, vec2<i32>(id.xy), vec4<f32>(0.0));
            }}
            {FINAL_PASS}
            "#
        );
        assert!(matches!(
            schema_err(&bad_format),
            SchemaError::StorageFormat { expected, .. } if expected == "rgba16float"
        ));
    }

    #[test]
    fn pass_errors() {
        assert!(matches!(
            schema_err(
                "
                @fragment
                fn a() -> @location(0) vec4<f32> { return vec4<f32>(0.0); }
                @fragment
                fn b() -> @location(0) vec4<f32> { return vec4<f32>(0.0); }
                "
            ),
            SchemaError::MultipleFinalPasses { .. }
        ));

        assert!(matches!(
            schema_err(
                r#"
                /// @pass(target = "a")
                @fragment
                fn only() -> @location(0) vec4<f32> { return vec4<f32>(0.0); }
                "#
            ),
            SchemaError::NoFinalPass
        ));

        assert!(matches!(
            schema_err(
                "
                /// @pass(persistent)
                @fragment
                fn only() -> @location(0) vec4<f32> { return vec4<f32>(0.0); }
                "
            ),
            SchemaError::FinalPassConfig { .. }
        ));

        assert!(matches!(
            schema_err(&format!(
                "
                @vertex
                fn vert() -> @builtin(position) vec4<f32> {{ return vec4<f32>(0.0); }}
                {FINAL_PASS}
                "
            )),
            SchemaError::UnsupportedStage { .. }
        ));
    }

    #[test]
    fn group_conventions() {
        assert!(matches!(
            schema_err(&format!(
                "
                @group(2) @binding(0) var tex: texture_2d<f32>;
                {FINAL_PASS}
                "
            )),
            SchemaError::BadGroup { group: 2, .. }
        ));

        assert!(matches!(
            schema_err(&format!(
                "
                struct G {{ time: f32, }}
                @group(0) @binding(2) var<uniform> globals: G;
                {FINAL_PASS}
                "
            )),
            SchemaError::Group0Binding { binding: 2, .. }
        ));
    }

    #[test]
    fn audio_annotations() {
        let src = format!(
            "
            /// @audio(samples = 1024)
            @group(1) @binding(1) var waveform: texture_2d<f32>;
            /// @audio_fft
            @group(1) @binding(2) var spectrum: texture_2d<f32>;
            {FINAL_PASS}
            "
        );
        let s = schema(&src);
        assert_eq!(
            s.textures[0].role,
            TextureRole::AudioWaveform { samples: 1024 }
        );
        assert_eq!(s.textures[1].role, TextureRole::AudioFft { bins: 256 });
    }

    #[test]
    fn eval_size_expressions() {
        let base = UVec2::new(800, 600);
        assert_eq!(eval_size("$WIDTH", base).unwrap(), 800);
        assert_eq!(eval_size("$WIDTH/2", base).unwrap(), 400);
        assert_eq!(eval_size("$WIDTH/16.0", base).unwrap(), 50);
        assert_eq!(eval_size("512", base).unwrap(), 512);
        assert_eq!(eval_size("max($HEIGHT, 1024)", base).unwrap(), 1024);
        assert!(matches!(
            eval_size("0", base),
            Err(SizeExprError::NonPositive { .. })
        ));
        assert!(matches!(
            eval_size("$WIDTH /", base),
            Err(SizeExprError::Eval { .. })
        ));
    }
}
