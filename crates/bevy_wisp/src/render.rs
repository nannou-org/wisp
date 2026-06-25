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
use crate::schema::{BindingDesc, BindingTy, PassStage, TextureRole, WispSchema, requires_compute};
use crate::targets::WispPassTargets;
use bevy_app::prelude::*;
use bevy_asset::prelude::*;
use bevy_core_pipeline::FullscreenShader;
use bevy_core_pipeline::schedule::{Core3d, Core3dSystems};
use bevy_ecs::prelude::*;
use bevy_ecs::system::SystemParamItem;
use bevy_image::prelude::Image;
use bevy_log::error;
use bevy_math::Vec2;
use bevy_platform::collections::HashMap;
use bevy_render::camera::ExtractedCamera;
use bevy_render::extract_component::ExtractComponentPlugin;
use bevy_render::extract_resource::ExtractResourcePlugin;
use bevy_render::render_asset::{PrepareAssetError, RenderAsset, RenderAssetPlugin, RenderAssets};
use bevy_render::render_resource::binding_types::{
    sampler, texture_2d, texture_storage_2d, uniform_buffer_sized,
};
use bevy_render::render_resource::*;
use bevy_render::renderer::{RenderAdapter, RenderContext, RenderDevice, ViewQuery};
use bevy_render::texture::{DefaultImageSampler, GpuImage};
use bevy_render::view::{Msaa, ViewTarget};
use bevy_render::{ExtractSchedule, MainWorld, Render, RenderApp, RenderSystems};
use bevy_shader::Shader;
use bevy_utils::default;
use std::num::NonZero;

pub struct WispRenderPlugin;

/// The render-world copy of a [`Wisp`] asset.
pub struct GpuWisp {
    pub schema: WispSchema,
    pub shader: Handle<Shader>,
}

/// One cached pipeline per pass, stamped with the asset they were built for.
#[derive(Component)]
pub struct WispPipelineIds {
    asset: AssetId<Wisp>,
    ids: Vec<WispPassPipelineId>,
}

#[derive(Clone, Copy, Debug)]
pub enum WispPassPipelineId {
    Render(CachedRenderPipelineId),
    Compute(CachedComputePipelineId),
}

/// Lazily-created 1x1 storage textures, bound in place of a pass target's
/// storage view in every pass except the one that writes it (binding the real
/// view there would conflict with sampling it).
#[derive(Resource, Default)]
struct WispDummyStorage(HashMap<TextureFormat, TextureView>);

/// Per-view uniform buffers, rewritten each frame.
#[derive(Component)]
pub struct WispUniforms {
    asset: AssetId<Wisp>,
    /// One aligned globals chunk per pass, indexed via `globals_offsets`.
    globals: Option<Buffer>,
    globals_offsets: Vec<u32>,
    params: Option<Buffer>,
}

/// Per-view bind groups and attachment snapshots, one entry per pass.
///
/// Bind groups are per-pass because pass-target bindings differ: a pass reading a
/// target written *earlier* this frame sees the fresh contents, while reading its
/// own target (or a *later* pass's) sees the previous frame's.
///
/// The attachment each pass writes is snapshotted *here*, from the same
/// [`WispPassTargets`] the bind groups were built from, so the sampled and
/// attached images can never disagree - even if the component outlives its
/// frame (e.g. while a newly selected shader is still loading). The render
/// system additionally checks `asset` against the view's current handle so
/// components built for another shader are never mixed.
#[derive(Component)]
pub struct WispBindGroups {
    asset: AssetId<Wisp>,
    passes: Vec<PassBindings>,
    /// The group bound with a per-pass dynamic offset (the globals group).
    dynamic_group: Option<usize>,
}

struct PassBindings {
    groups: Vec<BindGroup>,
    attachment: PassAttachment,
}

/// Where a pass writes, captured alongside its bind groups.
enum PassAttachment {
    /// The final pass renders to the camera's view target.
    View,
    /// An intermediate fragment pass renders to its target's write image.
    Target { image: Handle<Image>, clear: bool },
    /// A compute pass writes through its storage binding.
    Compute { dispatch: [u32; 3] },
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

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct WispComputePipelineKey {
    pub shader: Handle<Shader>,
    pub bindings: Vec<BindingDesc>,
    pub entry: String,
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
            .init_resource::<SpecializedComputePipelines<WispPipelines>>()
            .init_resource::<WispDummyStorage>()
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

impl SpecializedComputePipeline for WispPipelines {
    type Key = WispComputePipelineKey;

    fn specialize(&self, key: Self::Key) -> ComputePipelineDescriptor {
        ComputePipelineDescriptor {
            label: Some("wisp_compute_pipeline".into()),
            layout: bind_group_layout_descriptors(&key.bindings),
            immediate_size: 0,
            shader: key.shader,
            shader_defs: vec![],
            entry_point: Some(key.entry.into()),
            zero_initialize_workgroup_memory: false,
        }
    }
}

/// Whether the device supports compute shaders.
///
/// WebGL2 reports this downlevel flag unset; a wisp with a compute pass (see
/// [`requires_compute`]) then declares pipelines, layouts and storage textures
/// the backend cannot build, which would otherwise trip wgpu validation and
/// take down the app via the default render-error handler. The render systems
/// skip such a wisp on these devices and report it through [`WispErrors`].
pub(crate) fn supports_compute(adapter: &RenderAdapter) -> bool {
    adapter
        .get_downlevel_capabilities()
        .flags
        .contains(DownlevelFlags::COMPUTE_SHADERS)
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

// Bevy systems legitimately take one parameter per resource/query.
#[allow(clippy::too_many_arguments)]
fn queue_wisp(
    mut commands: Commands,
    pipeline_cache: Res<PipelineCache>,
    pipeline: Res<WispPipelines>,
    mut specialized: ResMut<SpecializedRenderPipelines<WispPipelines>>,
    mut specialized_compute: ResMut<SpecializedComputePipelines<WispPipelines>>,
    render_adapter: Res<RenderAdapter>,
    wisps: Res<RenderAssets<GpuWisp>>,
    views: Query<(Entity, &WispHandle, &Msaa, &ViewTarget)>,
) {
    for (entity, handle, msaa, view_target) in views.iter() {
        let Some(wisp) = wisps.get(&**handle) else {
            continue;
        };
        // A compute wisp needs compute-shader support; on a device without it
        // (e.g. WebGL2) queuing its pipelines and storage-texture layouts would
        // trip wgpu validation, so skip it. `sync_pipeline_errors` reports why.
        if requires_compute(&wisp.schema) && !supports_compute(&render_adapter) {
            continue;
        }
        let ids: Vec<WispPassPipelineId> = wisp
            .schema
            .passes
            .iter()
            .map(|pass| match pass.stage {
                PassStage::Compute => {
                    let key = WispComputePipelineKey {
                        shader: wisp.shader.clone(),
                        bindings: wisp.schema.bindings.clone(),
                        entry: pass.entry.clone(),
                    };
                    let id = specialized_compute.specialize(&pipeline_cache, &pipeline, key);
                    WispPassPipelineId::Compute(id)
                }
                PassStage::Fragment => {
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
                    let id = specialized.specialize(&pipeline_cache, &pipeline, key);
                    WispPassPipelineId::Render(id)
                }
            })
            .collect();
        commands.entity(entity).insert(WispPipelineIds {
            asset: handle.id(),
            ids,
        });
    }
}

fn prepare_wisp_uniforms(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    render_adapter: Res<RenderAdapter>,
    frame_globals: Res<FrameGlobals>,
    wisps: Res<RenderAssets<GpuWisp>>,
    views: Query<(
        Entity,
        &WispHandle,
        &WispInputs,
        &ViewTarget,
        Option<&ExtractedCamera>,
        Option<&WispPassTargets>,
    )>,
) {
    for (entity, handle, inputs, view_target, camera, pass_targets) in views.iter() {
        let Some(wisp) = wisps.get(&**handle) else {
            continue;
        };
        // Skip compute wisps the device cannot build (see `queue_wisp`); their
        // bind groups and pass targets are skipped too, so the uniforms would
        // go unused.
        if requires_compute(&wisp.schema) && !supports_compute(&render_adapter) {
            continue;
        }
        let schema = &wisp.schema;
        let extent = view_target.main_texture().size();
        // The final pass reports the camera's viewport size where one is set.
        let view_size = camera
            .and_then(|camera| camera.physical_viewport_size)
            .map(|size| size.as_vec2())
            .unwrap_or_else(|| Vec2::new(extent.width as f32, extent.height as f32));

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
            asset: handle.id(),
            globals,
            globals_offsets,
            params,
        });
    }
}

// Bevy systems legitimately take one parameter per resource/query.
#[allow(clippy::too_many_arguments)]
fn prepare_wisp_bind_groups(
    mut commands: Commands,
    render_device: Res<RenderDevice>,
    render_adapter: Res<RenderAdapter>,
    pipeline_cache: Res<PipelineCache>,
    default_sampler: Res<DefaultImageSampler>,
    gpu_images: Res<RenderAssets<GpuImage>>,
    wisps: Res<RenderAssets<GpuWisp>>,
    mut dummy_storage: ResMut<WispDummyStorage>,
    views: Query<(
        Entity,
        &WispHandle,
        &WispInputs,
        &WispUniforms,
        Option<&WispPassTargets>,
    )>,
) {
    for (entity, handle, inputs, uniforms, pass_targets) in views.iter() {
        let Some(wisp) = wisps.get(&**handle) else {
            continue;
        };
        // Skip compute wisps the device cannot build (see `queue_wisp`):
        // creating the placeholder storage textures and storage-texture bind
        // groups below would trip wgpu validation on e.g. WebGL2.
        if requires_compute(&wisp.schema) && !supports_compute(&render_adapter) {
            continue;
        }
        let Some(dummy) = gpu_images.get(&Handle::<Image>::default()) else {
            continue;
        };
        // Ensure a placeholder storage view exists for every storage format
        // before bind group creation borrows the map immutably.
        for binding in &wisp.schema.bindings {
            if let BindingTy::StorageTexture2d { format } = binding.ty {
                dummy_storage.entry(format, &render_device);
            }
        }
        let mut dynamic_group = None;
        let passes: Option<Vec<PassBindings>> = (0..wisp.schema.passes.len())
            .map(|pass_index| {
                create_pass_bind_groups(
                    &wisp.schema,
                    pass_index,
                    uniforms,
                    inputs,
                    pass_targets,
                    dummy,
                    &dummy_storage,
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
            asset: handle.id(),
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
    dummy_storage: &WispDummyStorage,
    gpu_images: &RenderAssets<GpuImage>,
    default_sampler: &DefaultImageSampler,
    pipeline_cache: &PipelineCache,
    render_device: &RenderDevice,
    dynamic_group: &mut Option<usize>,
) -> Option<PassBindings> {
    // Snapshot where the pass writes alongside its bind groups, so the
    // attachment and the sampled images always come from the same frame's
    // `WispPassTargets`. A targeted pass whose image is not built yet aborts
    // the whole set - rendering it to the view instead would be wrong.
    let pass = &schema.passes[pass_index];
    let target = pass_targets.and_then(|targets| targets.0.get(pass_index)?.as_ref());
    let attachment = match (pass.stage, target) {
        (PassStage::Compute, target) => PassAttachment::Compute {
            dispatch: target?.dispatch?,
        },
        (PassStage::Fragment, Some(target)) => PassAttachment::Target {
            image: target.write().clone(),
            clear: target.clear,
        },
        (PassStage::Fragment, None) => match pass.target.is_some() {
            true => return None,
            false => PassAttachment::View,
        },
    };

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
                BindingTy::StorageTexture2d { format } => storage_view(
                    schema,
                    binding,
                    pass_index,
                    pass_targets,
                    gpu_images,
                    dummy_storage,
                    *format,
                )?
                .into_binding(),
            };
            entries.push(BindGroupEntry {
                binding: binding.binding,
                resource,
            });
        }
        let layout = pipeline_cache.get_bind_group_layout(descriptor);
        groups.push(render_device.create_bind_group("wisp_bind_group", &layout, &entries));
    }
    Some(PassBindings { groups, attachment })
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
        // Audio textures are maintained as image inputs by the `audio` feature;
        // without it (or before any samples arrive) the dummy image is bound.
        TextureRole::ImageInput
        | TextureRole::AudioWaveform { .. }
        | TextureRole::AudioFft { .. } => match inputs.get(&texture.name) {
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
        // `StorageTarget` only classifies storage bindings, which never reach
        // this sampled-texture path.
        TextureRole::StorageTarget { .. } => None,
    }
}

impl WispDummyStorage {
    /// The placeholder storage view for a format, creating it on first use.
    fn entry(&mut self, format: TextureFormat, render_device: &RenderDevice) {
        self.0.entry(format).or_insert_with(|| {
            let texture = render_device.create_texture(&TextureDescriptor {
                label: Some("wisp_dummy_storage"),
                size: Extent3d {
                    width: 1,
                    height: 1,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: TextureDimension::D2,
                format,
                usage: TextureUsages::STORAGE_BINDING,
                view_formats: &[],
            });
            texture.create_view(&TextureViewDescriptor::default())
        });
    }
}

/// The view bound for a storage-texture binding from the given pass.
///
/// Only the pass that owns the target binds its real write view; every other
/// pass gets the placeholder so the image can be sampled elsewhere in the same
/// usage scope.
fn storage_view<'a>(
    schema: &WispSchema,
    binding: &BindingDesc,
    pass_index: usize,
    pass_targets: Option<&'a WispPassTargets>,
    gpu_images: &'a RenderAssets<GpuImage>,
    dummy_storage: &'a WispDummyStorage,
    format: TextureFormat,
) -> Option<&'a TextureView> {
    let dummy = dummy_storage.0.get(&format);
    let texture = schema
        .textures
        .iter()
        .find(|t| (t.group, t.binding) == (binding.group, binding.binding));
    let real = texture.and_then(|texture| match texture.role {
        TextureRole::StorageTarget { pass } if pass == pass_index => {
            let target = pass_targets?.0.get(pass)?.as_ref()?;
            gpu_images
                .get(target.write())
                .map(|gpu_image| &gpu_image.texture_view)
        }
        _ => None,
    });
    real.or(dummy)
}

/// Mirror pipeline compilation errors into the main world's [`WispErrors`].
///
/// Runs during extraction (the only point where the render world can reach the
/// main world).
fn sync_pipeline_errors(
    mut main_world: ResMut<MainWorld>,
    pipeline_cache: Res<PipelineCache>,
    render_adapter: Res<RenderAdapter>,
    wisps: Res<RenderAssets<GpuWisp>>,
    views: Query<(&WispHandle, Option<&WispPipelineIds>)>,
) {
    let mut errors = std::collections::BTreeMap::new();
    for (wisp, pipeline_ids) in views.iter() {
        let Some(wisp) = wisps.get(&**wisp) else {
            continue;
        };
        // A compute wisp on a device without compute support is never queued
        // (see `queue_wisp`), so it has no pipeline state to inspect; report the
        // unsupported backend here instead, keyed by each compute pass.
        if requires_compute(&wisp.schema) && !supports_compute(&render_adapter) {
            for pass in wisp.schema.passes.iter() {
                if matches!(pass.stage, PassStage::Compute) {
                    errors.insert(
                        pass.entry.clone(),
                        "compute shaders are not supported by this device (e.g. under \
                         WebGL2); run with a WebGPU or native backend to use compute passes"
                            .to_string(),
                    );
                }
            }
            continue;
        }
        let Some(pipeline_ids) = pipeline_ids else {
            continue;
        };
        let passes = wisp.schema.passes.iter().zip(pipeline_ids.ids.iter());
        for (pass, pipeline_id) in passes {
            let state = match pipeline_id {
                WispPassPipelineId::Render(id) => pipeline_cache.get_render_pipeline_state(*id),
                WispPassPipelineId::Compute(id) => pipeline_cache.get_compute_pipeline_state(*id),
            };
            if let CachedPipelineState::Err(err) = state {
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
        &WispHandle,
        &WispBindGroups,
        &WispPipelineIds,
        &WispUniforms,
        Option<&ExtractedCamera>,
    )>,
    mut ctx: RenderContext,
    pipeline_cache: Res<PipelineCache>,
    gpu_images: Res<RenderAssets<GpuImage>>,
) {
    let (view_target, handle, bind_groups, pipeline_ids, uniforms, camera) = view.into_inner();
    // Render only a coherent generation: every component must have been built
    // for the shader the view currently points at. While a newly selected
    // shader is still loading/compiling, components from the previous one
    // linger - mixing them (e.g. old bind groups with new pass targets) binds
    // images inconsistently and trips wgpu's usage validation.
    let asset = handle.id();
    if bind_groups.asset != asset
        || pipeline_ids.asset != asset
        || uniforms.asset != asset
        || bind_groups.passes.len() != pipeline_ids.ids.len()
    {
        return;
    }
    // All-or-nothing: rendering a partial pass chain (e.g. while pipelines
    // recompile after a hot reload) would feed stale targets downstream.
    enum Ready<'a> {
        Render(&'a RenderPipeline),
        Compute(&'a ComputePipeline),
    }
    let pipelines: Option<Vec<Ready>> = pipeline_ids
        .ids
        .iter()
        .map(|pipeline_id| match pipeline_id {
            WispPassPipelineId::Render(id) => {
                pipeline_cache.get_render_pipeline(*id).map(Ready::Render)
            }
            WispPassPipelineId::Compute(id) => {
                pipeline_cache.get_compute_pipeline(*id).map(Ready::Compute)
            }
        })
        .collect();
    let Some(pipelines) = pipelines else {
        return;
    };
    for (pass_index, pipeline) in pipelines.into_iter().enumerate() {
        let Some(pass) = bind_groups.passes.get(pass_index) else {
            continue;
        };
        let groups = &pass.groups;
        let offsets: Vec<(usize, Option<u32>)> = (0..groups.len())
            .map(|group_index| {
                let offset = match bind_groups.dynamic_group {
                    Some(dynamic) if dynamic == group_index => {
                        uniforms.globals_offsets.get(pass_index).copied()
                    }
                    _ => None,
                };
                (group_index, offset)
            })
            .collect();
        match pipeline {
            Ready::Compute(pipeline) => {
                let PassAttachment::Compute { dispatch } = pass.attachment else {
                    continue;
                };
                let mut compute_pass =
                    ctx.command_encoder()
                        .begin_compute_pass(&ComputePassDescriptor {
                            label: Some("wisp_compute_pass"),
                            timestamp_writes: None,
                        });
                compute_pass.set_pipeline(pipeline);
                for (group_index, offset) in offsets {
                    let group = &groups[group_index];
                    match offset {
                        Some(offset) => {
                            compute_pass.set_bind_group(group_index as u32, &**group, &[offset]);
                        }
                        None => compute_pass.set_bind_group(group_index as u32, &**group, &[]),
                    }
                }
                let [x, y, z] = dispatch;
                compute_pass.dispatch_workgroups(x, y, z);
            }
            Ready::Render(pipeline) => {
                let color_attachment = match &pass.attachment {
                    PassAttachment::Compute { .. } => continue,
                    PassAttachment::View => view_target.get_color_attachment(),
                    PassAttachment::Target { image, clear } => {
                        let Some(gpu_image) = gpu_images.get(image) else {
                            continue;
                        };
                        RenderPassColorAttachment {
                            view: &gpu_image.texture_view,
                            resolve_target: None,
                            ops: Operations {
                                load: match clear {
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
                // The final pass honours the camera's viewport (e.g. sharing
                // the window with UI panels); intermediate targets are already
                // sized to it.
                if matches!(pass.attachment, PassAttachment::View)
                    && let Some(viewport) = camera.and_then(|camera| camera.viewport.as_ref())
                {
                    render_pass.set_camera_viewport(viewport);
                }
                render_pass.set_render_pipeline(pipeline);
                for (group_index, offset) in offsets {
                    let group = &groups[group_index];
                    match offset {
                        Some(offset) => render_pass.set_bind_group(group_index, group, &[offset]),
                        None => render_pass.set_bind_group(group_index, group, &[]),
                    }
                }
                render_pass.draw(0..3, 0..1);
            }
        }
    }
}
