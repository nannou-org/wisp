//! Render-world plumbing: pipeline specialization, uniform packing and the
//! fullscreen pass.
//!
//! Everything here works from the [`WispSchema`] alone. Bind group layouts are
//! built from the schema's reflected bindings, so the pipeline layout is exactly
//! what the shader declared; the vertex stage is always bevy's fullscreen
//! triangle ([`FullscreenShader`]).

use crate::asset::{Wisp, WispHandle};
use crate::error::WispErrors;
use crate::globals::{FrameGlobals, pack_globals};
use crate::inputs::{WispInputs, WispValue, pack_params};
use crate::schema::{BindingDesc, BindingTy, PassStage, TextureRole, WispSchema};
use crate::targets::WispPassTargets;
use bevy::core_pipeline::FullscreenShader;
use bevy::core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy::ecs::system::SystemParamItem;
use bevy::log::warn_once;
use bevy::prelude::*;
use bevy::render::MainWorld;
use bevy::render::extract_component::ExtractComponentPlugin;
use bevy::render::extract_resource::ExtractResourcePlugin;
use bevy::render::render_asset::{PrepareAssetError, RenderAsset, RenderAssetPlugin, RenderAssets};
use bevy::render::render_resource::binding_types::{
    sampler, texture_2d, texture_storage_2d, uniform_buffer_sized,
};
use bevy::render::render_resource::*;
use bevy::render::renderer::{RenderContext, RenderDevice, ViewQuery};
use bevy::render::texture::{DefaultImageSampler, GpuImage};
use bevy::render::view::ViewTarget;
use bevy::render::{Render, RenderApp, RenderSystems};
use std::num::NonZero;

pub struct WispRenderPlugin;

/// The render-world copy of a [`Wisp`] asset.
pub struct GpuWisp {
    pub schema: WispSchema,
    pub shader: Handle<Shader>,
}

/// One cached pipeline per pass; `None` for passes not yet supported.
#[derive(Component, Deref, DerefMut, Default)]
pub struct WispPipelineIds(Vec<Option<CachedRenderPipelineId>>);

/// Per-view uniform buffers, rewritten each frame.
#[derive(Component)]
pub struct WispUniforms {
    /// One aligned globals chunk per pass, indexed via `globals_offsets`.
    globals: Option<Buffer>,
    globals_offsets: Vec<u32>,
    params: Option<Buffer>,
}

/// Per-view bind groups: for each pass, one bind group per group index.
///
/// Bind groups are per-pass because pass-target bindings differ: a pass reading a
/// target written *earlier* this frame sees the fresh contents, while reading its
/// own target (or a *later* pass's) sees the previous frame's.
#[derive(Component)]
pub struct WispBindGroups {
    passes: Vec<Vec<BindGroup>>,
    /// The group bound with a per-pass dynamic offset (the globals group).
    dynamic_group: Option<usize>,
}

#[derive(Resource)]
pub struct WispPipelines {
    fullscreen: FullscreenShader,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct WispPipelineKey {
    pub shader: Handle<Shader>,
    pub bindings: Vec<BindingDesc>,
    pub entry: String,
    pub format: TextureFormat,
    pub samples: u32,
}

impl Plugin for WispRenderPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            RenderAssetPlugin::<GpuWisp>::default(),
            ExtractComponentPlugin::<WispHandle>::default(),
            ExtractComponentPlugin::<WispInputs>::default(),
            ExtractComponentPlugin::<WispPassTargets>::default(),
            ExtractResourcePlugin::<FrameGlobals>::default(),
        ));
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app
            .init_resource::<SpecializedRenderPipelines<WispPipelines>>()
            .add_systems(ExtractSchedule, sync_pipeline_errors)
            .add_systems(
                Render,
                (
                    queue_wisp.in_set(RenderSystems::Queue),
                    prepare_wisp_uniforms.in_set(RenderSystems::PrepareResources),
                    prepare_wisp_bind_groups.in_set(RenderSystems::PrepareBindGroups),
                ),
            )
            .add_systems(Core3d, wisp_render.in_set(Core3dSystems::MainPass));
    }

    fn finish(&self, app: &mut App) {
        let Some(render_app) = app.get_sub_app_mut(RenderApp) else {
            return;
        };
        render_app.init_resource::<WispPipelines>();
    }
}

impl RenderAsset for GpuWisp {
    type SourceAsset = Wisp;
    type Param = ();

    fn prepare_asset(
        wisp: Self::SourceAsset,
        _asset_id: AssetId<Self::SourceAsset>,
        _param: &mut SystemParamItem<Self::Param>,
        _previous_asset: Option<&Self>,
    ) -> Result<Self, PrepareAssetError<Self::SourceAsset>> {
        Ok(GpuWisp {
            schema: wisp.schema,
            shader: wisp.shader,
        })
    }
}

impl FromWorld for WispPipelines {
    fn from_world(world: &mut World) -> Self {
        Self {
            fullscreen: world.resource::<FullscreenShader>().clone(),
        }
    }
}

impl SpecializedRenderPipeline for WispPipelines {
    type Key = WispPipelineKey;

    fn specialize(&self, key: Self::Key) -> RenderPipelineDescriptor {
        RenderPipelineDescriptor {
            label: Some("wisp_pipeline".into()),
            layout: bind_group_layout_descriptors(&key.bindings),
            immediate_size: 0,
            vertex: self.fullscreen.to_vertex_state(),
            fragment: Some(FragmentState {
                shader: key.shader,
                shader_defs: vec![],
                entry_point: Some(key.entry.into()),
                targets: vec![Some(ColorTargetState {
                    format: key.format,
                    blend: None,
                    write_mask: ColorWrites::ALL,
                })],
            }),
            primitive: PrimitiveState::default(),
            depth_stencil: None,
            multisample: MultisampleState {
                count: key.samples,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            zero_initialize_workgroup_memory: false,
        }
    }
}

/// The bind group layout descriptors for a schema's bindings, one per group.
///
/// Used for both pipeline specialization and bind group creation - the
/// `PipelineCache` deduplicates layouts by descriptor equality.
pub fn bind_group_layout_descriptors(bindings: &[BindingDesc]) -> Vec<BindGroupLayoutDescriptor> {
    let group_count = bindings.iter().map(|b| b.group + 1).max().unwrap_or(0) as usize;
    (0..group_count)
        .map(|group| {
            let entries: Vec<BindGroupLayoutEntry> = bindings
                .iter()
                .filter(|b| b.group as usize == group)
                .map(|b| match &b.ty {
                    BindingTy::Globals { size } => {
                        uniform_buffer_sized(true, NonZero::new(*size as u64))
                            .build(b.binding, b.visibility)
                    }
                    BindingTy::Params { size } => {
                        uniform_buffer_sized(false, NonZero::new(*size as u64))
                            .build(b.binding, b.visibility)
                    }
                    BindingTy::Texture2d => {
                        texture_2d(TextureSampleType::Float { filterable: true })
                            .build(b.binding, b.visibility)
                    }
                    BindingTy::Sampler => {
                        sampler(SamplerBindingType::Filtering).build(b.binding, b.visibility)
                    }
                    BindingTy::StorageTexture2d { format } => {
                        texture_storage_2d(*format, StorageTextureAccess::WriteOnly)
                            .build(b.binding, b.visibility)
                    }
                })
                .collect();
            BindGroupLayoutDescriptor::new("wisp_bind_group_layout", &entries)
        })
        .collect()
}

fn queue_wisp(
    mut commands: Commands,
    pipeline_cache: Res<PipelineCache>,
    pipeline: Res<WispPipelines>,
    mut specialized: ResMut<SpecializedRenderPipelines<WispPipelines>>,
    wisps: Res<RenderAssets<GpuWisp>>,
    views: Query<(Entity, &WispHandle, &Msaa, &ViewTarget)>,
) {
    for (entity, wisp, msaa, view_target) in views.iter() {
        let Some(wisp) = wisps.get(&**wisp) else {
            continue;
        };
        let ids: Vec<Option<CachedRenderPipelineId>> = wisp
            .schema
            .passes
            .iter()
            .map(|pass| {
                // Compute passes land in a later milestone.
                if pass.stage != PassStage::Fragment {
                    return None;
                }
                // Intermediate targets render at their own format, unsampled;
                // the final pass matches the view.
                let (format, samples) = match &pass.target {
                    Some(target) => (target.format, 1),
                    None => (view_target.main_texture_format(), msaa.samples()),
                };
                let key = WispPipelineKey {
                    shader: wisp.shader.clone(),
                    bindings: wisp.schema.bindings.clone(),
                    entry: pass.entry.clone(),
                    format,
                    samples,
                };
                Some(specialized.specialize(&pipeline_cache, &pipeline, key))
            })
            .collect();
        commands.entity(entity).insert(WispPipelineIds(ids));
    }
}

fn prepare_wisp_uniforms(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    frame_globals: Res<FrameGlobals>,
    wisps: Res<RenderAssets<GpuWisp>>,
    views: Query<(
        Entity,
        &WispHandle,
        &WispInputs,
        &ViewTarget,
        Option<&WispPassTargets>,
    )>,
) {
    for (entity, wisp, inputs, view_target, pass_targets) in views.iter() {
        let Some(wisp) = wisps.get(&**wisp) else {
            continue;
        };
        let schema = &wisp.schema;
        let extent = view_target.main_texture().size();
        let view_size = Vec2::new(extent.width as f32, extent.height as f32);

        let (globals, globals_offsets) = match &schema.globals {
            None => (None, Vec::new()),
            Some(globals_schema) => {
                let align = render_device.limits().min_uniform_buffer_offset_alignment as usize;
                let mut bytes = Vec::new();
                let mut offsets = Vec::new();
                for (pass_index, _pass) in schema.passes.iter().enumerate() {
                    // Each pass sees its own target's resolution; the final pass
                    // sees the view's.
                    let resolution = pass_targets
                        .and_then(|targets| targets.0.get(pass_index)?.as_ref())
                        .map(|target| target.size.as_vec2())
                        .unwrap_or(view_size);
                    let values = frame_globals.values(resolution, pass_index as u32);
                    offsets.push(bytes.len() as u32);
                    bytes.extend(pack_globals(globals_schema, &values));
                    bytes.resize(bytes.len().next_multiple_of(align), 0);
                }
                let buffer = render_device.create_buffer_with_data(&BufferInitDescriptor {
                    label: Some("wisp_globals"),
                    contents: &bytes,
                    usage: BufferUsages::UNIFORM,
                });
                (Some(buffer), offsets)
            }
        };

        let params = schema.params.as_ref().map(|params_schema| {
            render_device.create_buffer_with_data(&BufferInitDescriptor {
                label: Some("wisp_params"),
                contents: &pack_params(params_schema, inputs),
                usage: BufferUsages::UNIFORM,
            })
        });

        commands.entity(entity).insert(WispUniforms {
            globals,
            globals_offsets,
            params,
        });
    }
}

fn prepare_wisp_bind_groups(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    pipeline_cache: Res<PipelineCache>,
    default_sampler: Res<DefaultImageSampler>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    wisps: Res<RenderAssets<GpuWisp>>,
    views: Query<(
        Entity,
        &WispHandle,
        &WispInputs,
        &WispUniforms,
        Option<&WispPassTargets>,
    )>,
) {
    for (entity, wisp, inputs, uniforms, pass_targets) in views.iter() {
        let Some(wisp) = wisps.get(&**wisp) else {
            continue;
        };
        if wisp
            .schema
            .bindings
            .iter()
            .any(|b| matches!(b.ty, BindingTy::StorageTexture2d { .. }))
        {
            warn_once!("wisp: compute passes are not supported yet; skipping");
            continue;
        }
        let Some(dummy) = gpu_images.get(&Handle::<Image>::default()) else {
            continue;
        };
        let mut dynamic_group = None;
        let passes: Option<Vec<Vec<BindGroup>>> = (0..wisp.schema.passes.len())
            .map(|pass_index| {
                create_pass_bind_groups(
                    &wisp.schema,
                    pass_index,
                    uniforms,
                    inputs,
                    pass_targets,
                    dummy,
                    &gpu_images,
                    &default_sampler,
                    &pipeline_cache,
                    &render_device,
                    &mut dynamic_group,
                )
            })
            .collect();
        let Some(passes) = passes else {
            continue;
        };
        commands.entity(entity).insert(WispBindGroups {
            passes,
            dynamic_group,
        });
    }
}

#[allow(clippy::too_many_arguments)]
fn create_pass_bind_groups(
    schema: &WispSchema,
    pass_index: usize,
    uniforms: &WispUniforms,
    inputs: &WispInputs,
    pass_targets: Option<&WispPassTargets>,
    dummy: &GpuImage,
    gpu_images: &RenderAssets<GpuImage>,
    default_sampler: &DefaultImageSampler,
    pipeline_cache: &PipelineCache,
    render_device: &RenderDevice,
    dynamic_group: &mut Option<usize>,
) -> Option<Vec<BindGroup>> {
    let descriptors = bind_group_layout_descriptors(&schema.bindings);
    let mut groups = Vec::with_capacity(descriptors.len());
    for (group_index, descriptor) in descriptors.iter().enumerate() {
        let mut entries = Vec::new();
        let bindings = schema
            .bindings
            .iter()
            .filter(|b| b.group as usize == group_index);
        for binding in bindings {
            let resource = match &binding.ty {
                BindingTy::Globals { size } => {
                    *dynamic_group = Some(group_index);
                    BindingResource::Buffer(BufferBinding {
                        buffer: uniforms.globals.as_ref()?,
                        offset: 0,
                        size: NonZero::new(*size as u64),
                    })
                }
                BindingTy::Params { .. } => uniforms.params.as_ref()?.as_entire_binding(),
                BindingTy::Sampler => default_sampler.into_binding(),
                BindingTy::Texture2d => {
                    texture_image(schema, binding, inputs, pass_index, pass_targets)
                        .and_then(|handle| gpu_images.get(&handle))
                        .unwrap_or(dummy)
                        .texture_view
                        .into_binding()
                }
                BindingTy::StorageTexture2d { .. } => return None,
            };
            entries.push(BindGroupEntry {
                binding: binding.binding,
                resource,
            });
        }
        let layout = pipeline_cache.get_bind_group_layout(descriptor);
        groups.push(render_device.create_bind_group("wisp_bind_group", &layout, &entries));
    }
    Some(groups)
}

/// The image handle bound for a texture binding from the given pass, if any.
fn texture_image(
    schema: &WispSchema,
    binding: &BindingDesc,
    inputs: &WispInputs,
    pass_index: usize,
    pass_targets: Option<&WispPassTargets>,
) -> Option<Handle<Image>> {
    let texture = schema
        .textures
        .iter()
        .find(|t| (t.group, t.binding) == (binding.group, binding.binding))?;
    match &texture.role {
        TextureRole::ImageInput => match inputs.get(&texture.name) {
            Some(WispValue::Image(handle)) => Some(handle.clone()),
            _ => None,
        },
        TextureRole::PassTarget { pass } => {
            let target = pass_targets?.0.get(*pass)?.as_ref()?;
            // Passes already rendered this frame are read fresh; the reading
            // pass's own target (and later passes') read the previous frame.
            if *pass < pass_index {
                Some(target.write().clone())
            } else if *pass == pass_index && target.read() == target.write() {
                // Without ping-pong (no self-feedback), a pass's own target is
                // its colour attachment and must not also be sampled - bind the
                // placeholder; the entry point never reads it anyway.
                None
            } else {
                Some(target.read().clone())
            }
        }
        // Storage and audio textures land in later milestones - the dummy image
        // keeps the bind group valid meanwhile.
        TextureRole::StorageTarget { .. }
        | TextureRole::AudioWaveform { .. }
        | TextureRole::AudioFft { .. } => None,
    }
}

/// Mirror pipeline compilation errors into the main world's [`WispErrors`].
///
/// Runs during extraction (the only point where the render world can reach the
/// main world), following the pattern of nannou's compute `sync_pipeline_cache`.
fn sync_pipeline_errors(
    mut main_world: ResMut<MainWorld>,
    pipeline_cache: Res<PipelineCache>,
    wisps: Res<RenderAssets<GpuWisp>>,
    views: Query<(&WispHandle, &WispPipelineIds)>,
) {
    let mut errors = std::collections::BTreeMap::new();
    for (wisp, pipeline_ids) in views.iter() {
        let Some(wisp) = wisps.get(&**wisp) else {
            continue;
        };
        let passes = wisp.schema.passes.iter().zip(pipeline_ids.iter());
        for (pass, pipeline_id) in passes {
            let Some(pipeline_id) = pipeline_id else {
                continue;
            };
            if let CachedPipelineState::Err(err) =
                pipeline_cache.get_render_pipeline_state(*pipeline_id)
            {
                errors.insert(pass.entry.clone(), err.to_string());
            }
        }
    }
    let Some(mut wisp_errors) = main_world.get_resource_mut::<WispErrors>() else {
        return;
    };
    if wisp_errors.pipeline != errors {
        for (entry, err) in &errors {
            if wisp_errors.pipeline.get(entry) != Some(err) {
                error!("wisp pipeline error in pass `{entry}`:\n{err}");
            }
        }
        wisp_errors.pipeline = errors;
    }
}

fn wisp_render(
    view: ViewQuery<(
        &ViewTarget,
        &WispBindGroups,
        &WispPipelineIds,
        &WispUniforms,
        Option<&WispPassTargets>,
    )>,
    mut ctx: RenderContext,
    pipeline_cache: Res<PipelineCache>,
    gpu_images: Res<RenderAssets<GpuImage>>,
) {
    let (view_target, bind_groups, pipeline_ids, uniforms, pass_targets) = view.into_inner();
    // All-or-nothing: rendering a partial pass chain (e.g. while pipelines
    // recompile after a hot reload) would feed stale targets downstream.
    let pipelines: Option<Vec<Option<&RenderPipeline>>> = pipeline_ids
        .iter()
        .map(|pipeline_id| match pipeline_id {
            // Unsupported pass (e.g. compute, for now): skipped, not blocking.
            None => Some(None),
            Some(id) => pipeline_cache.get_render_pipeline(*id).map(Some),
        })
        .collect();
    let Some(pipelines) = pipelines else {
        return;
    };
    for (pass_index, pipeline) in pipelines.into_iter().enumerate() {
        let Some(pipeline) = pipeline else {
            continue;
        };
        let target = pass_targets.and_then(|targets| targets.0.get(pass_index)?.as_ref());
        let color_attachment = match target {
            None => view_target.get_color_attachment(),
            Some(target) => {
                let Some(gpu_image) = gpu_images.get(target.write()) else {
                    continue;
                };
                RenderPassColorAttachment {
                    view: &gpu_image.texture_view,
                    resolve_target: None,
                    ops: Operations {
                        load: match target.clear {
                            true => LoadOp::Clear(default()),
                            false => LoadOp::Load,
                        },
                        store: StoreOp::Store,
                    },
                    depth_slice: None,
                }
            }
        };
        let mut render_pass = ctx.begin_tracked_render_pass(RenderPassDescriptor {
            label: Some("wisp_pass"),
            color_attachments: &[Some(color_attachment)],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        render_pass.set_render_pipeline(pipeline);
        let Some(groups) = bind_groups.passes.get(pass_index) else {
            continue;
        };
        for (group_index, group) in groups.iter().enumerate() {
            let offset = match bind_groups.dynamic_group {
                Some(dynamic) if dynamic == group_index => {
                    uniforms.globals_offsets.get(pass_index).copied()
                }
                _ => None,
            };
            match offset {
                Some(offset) => render_pass.set_bind_group(group_index, group, &[offset]),
                None => render_pass.set_bind_group(group_index, group, &[]),
            }
        }
        render_pass.draw(0..3, 0..1);
    }
}
