//! A minimal live-coding editor for wisp shaders.
//!
//! The UI is a re-arrangeable `egui_tiles` layout of three panes: a monospace
//! code editor with syntax highlighting (and the file controls), the shader
//! param widgets, and the shader view itself. Drag a tab header to re-arrange
//! the panes; the shader pane hides its tab when it sits alone so its view
//! stays clean (the camera viewport follows that pane). Pick one of the
//! bundled shaders (compiled into the binary, so no assets dir is needed at
//! runtime) or create your own. Saving (button or ctrl/cmd+S) persists to a
//! key-value store ([`bevy_pkv`]) that works the same on native (a file in the
//! platform data dir) and on the web (browser local storage) - editing a
//! bundled shader saves a copy that shadows it - and reloads the shader in
//! place; broken edits keep the last working version on screen while the error
//! shows in the params pane.
//!
//! Stored shaders load through the same `embedded://` asset source as the
//! bundled ones: their source is (re)inserted into the embedded registry, so no
//! filesystem is involved and the create/save controls work on the web too.

use bevy::asset::RenderAssetUsages;
use bevy::asset::io::embedded::EmbeddedAssetRegistry;
use bevy::camera::Viewport;
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};
use bevy_egui::{EguiContexts, EguiPlugin, egui};
use bevy_pkv::PkvStore;
use bevy_wisp::prelude::*;
use bevy_wisp::ui::{errors_ui, params_ui};
use std::path::PathBuf;

mod audio;

/// Seed contents for a freshly created shader.
const NEW_SHADER_TEMPLATE: &str = r#"//! A fresh wisp - edit me.

struct Globals {
    resolution: vec2<f32>,
    time: f32,
}
@group(0) @binding(0) var<uniform> globals: Globals;

struct Params {
    /// @min(0.0) @max(1.0) @default(0.35)
    thickness: f32,
    /// @color @default(0.1, 0.6, 0.9, 1.0)
    tint: vec4<f32>,
}
@group(1) @binding(0) var<uniform> params: Params;

@fragment
fn fragment(@location(0) uv: vec2<f32>) -> @location(0) vec4<f32> {
    let wave = 0.5 + 0.3 * sin(uv.x * 6.2832 + globals.time);
    let band = smoothstep(params.thickness, 0.0, abs(uv.y - wave));
    return vec4<f32>(params.tint.rgb * (0.05 + band), 1.0);
}
"#;

/// The shaders compiled into the binary. They are registered with the
/// `embedded://` asset source at startup (see [`register_bundled`]) and shown in
/// the picker unless a user shader of the same name shadows them.
const BUNDLED: &[(&str, &str)] = &[
    (
        "test_audio",
        include_str!("../../assets/wisp/test_audio.wgsl"),
    ),
    (
        "test_audio_fft",
        include_str!("../../assets/wisp/test_audio_fft.wgsl"),
    ),
    (
        "test_color",
        include_str!("../../assets/wisp/test_color.wgsl"),
    ),
    (
        "test_compute",
        include_str!("../../assets/wisp/test_compute.wgsl"),
    ),
    (
        "test_float",
        include_str!("../../assets/wisp/test_float.wgsl"),
    ),
    (
        "test_image",
        include_str!("../../assets/wisp/test_image.wgsl"),
    ),
    (
        "test_inputs",
        include_str!("../../assets/wisp/test_inputs.wgsl"),
    ),
    (
        "test_multi_pass_rendering",
        include_str!("../../assets/wisp/test_multi_pass_rendering.wgsl"),
    ),
    (
        "test_persistent_buffer",
        include_str!("../../assets/wisp/test_persistent_buffer.wgsl"),
    ),
];

/// The pkv key holding the index of user shader names. The store cannot
/// enumerate keys, so the names are kept in this list alongside the per-shader
/// source entries (see [`shader_source_key`]).
const SHADER_INDEX_KEY: &str = "shaders";

/// Persistent shader storage, wrapping a [`PkvStore`]. The store backs the same
/// on native (a `redb` file in the platform data dir) and on the web (browser
/// local storage), so saving and loading work identically on both. bevy_pkv's
/// own `bevy` feature is disabled to avoid a second `bevy_ecs`, so this newtype
/// supplies the [`Resource`] impl.
#[derive(Resource)]
struct Pkv(PkvStore);

/// A built-in image bound to every `@image` shader input the user has not set.
/// Without it those inputs keep bevy's 1x1 white placeholder, so an image
/// shader shows a flat, static frame; a real picture makes it visibly work.
#[derive(Resource)]
struct DefaultImage(Handle<Image>);

#[derive(Resource)]
struct Editor {
    files: Vec<ShaderFile>,
    selected: Option<usize>,
    buffer: String,
    dirty: bool,
    new_name: String,
    status: Option<String>,
}

struct ShaderFile {
    name: String,
    source: ShaderSource,
}

/// Where a shader's source comes from.
enum ShaderSource {
    /// Compiled into the binary; loaded via `embedded://wisp/{name}.wgsl`.
    Bundled(&'static str),
    /// A user shader whose source lives in the [`Pkv`] store under the file's
    /// name, loaded via the embedded source after [`register_user_source`].
    User,
}

/// What the UI asked for this frame, applied after the tree is drawn.
enum Action {
    Select(usize),
    Save,
    Create,
}

/// The re-arrangeable panes of the editor layout.
#[derive(PartialEq)]
enum Pane {
    /// The code editor and shader file controls.
    Editor,
    /// The reflected shader param widgets and any load/pipeline errors.
    Params,
    /// The shader view itself; painted nothing so the wisp camera shows
    /// through, and its tab is hidden whenever it sits alone.
    Shader,
}

/// Holds the tile layout for the process lifetime (no persistence).
#[derive(Resource)]
struct EditorTree(egui_tiles::Tree<Pane>);

/// Draws each pane and collects what the UI asked for this frame.
///
/// Borrows the editor state and shader inputs so the pane closures can mutate
/// them while `egui_tiles` owns the surrounding `Ui`.
struct TreeBehavior<'a> {
    editor: &'a mut Editor,
    errors: &'a WispErrors,
    wisp: Option<&'a Wisp>,
    inputs: Option<&'a mut WispInputs>,
    audio: &'a mut audio::AudioConfig,
    action: &'a mut Option<Action>,
    /// Set to the shader pane's rect so the camera viewport can follow it.
    shader_rect: &'a mut Option<egui::Rect>,
}

impl Editor {
    /// The bundled shaders plus any user shaders stored in pkv, sorted by name.
    /// A stored shader shadows the bundled shader of the same name.
    fn scan(pkv: &Pkv) -> Vec<ShaderFile> {
        let mut files: Vec<ShaderFile> = BUNDLED
            .iter()
            .map(|&(name, source)| ShaderFile {
                name: name.to_owned(),
                source: ShaderSource::Bundled(source),
            })
            .collect();
        for name in stored_shader_names(pkv) {
            match files.iter_mut().find(|file| file.name == name) {
                Some(file) => file.source = ShaderSource::User,
                None => files.push(ShaderFile {
                    name,
                    source: ShaderSource::User,
                }),
            }
        }
        files.sort_by(|a, b| a.name.cmp(&b.name));
        files
    }

    fn label(&self, index: usize) -> String {
        self.files[index].name.clone()
    }
}

impl egui_tiles::Behavior<Pane> for TreeBehavior<'_> {
    fn tab_title_for_pane(&mut self, pane: &Pane) -> egui::WidgetText {
        match pane {
            Pane::Editor => "editor".into(),
            Pane::Params => "params".into(),
            // Only ever visible while a tile is being dragged.
            Pane::Shader => "shader".into(),
        }
    }

    // The panes paint themselves the dark text-edit colour, so the lighter
    // panel fill here reads as a header strip that separates stacked panes.
    fn tab_bar_color(&self, visuals: &egui::Visuals) -> egui::Color32 {
        visuals.panel_fill
    }

    fn resize_stroke(
        &self,
        style: &egui::Style,
        _resize_state: egui_tiles::ResizeState,
    ) -> egui::Stroke {
        egui::Stroke::new(2.0, style.visuals.extreme_bg_color)
    }

    fn tab_outline_stroke(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<Pane>,
        _tile_id: egui_tiles::TileId,
        _state: &egui_tiles::TabState,
    ) -> egui::Stroke {
        egui::Stroke::NONE
    }

    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        // We simplify manually before `tree.ui`, so `tree.ui` should not.
        egui_tiles::SimplificationOptions::OFF
    }

    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        _tile_id: egui_tiles::TileId,
        pane: &mut Pane,
    ) -> egui_tiles::UiResponse {
        // Reborrow the fields so the pane closures can capture them
        // individually rather than borrowing all of `self` at once.
        let editor = &mut *self.editor;
        let action = &mut *self.action;
        let errors = self.errors;
        let wisp = self.wisp;
        let inputs = self.inputs.as_deref_mut();
        let audio = &mut *self.audio;
        match pane {
            // Painted nothing: the wisp camera renders through this pane, and
            // the camera viewport is fitted to its rect after the tree draws.
            Pane::Shader => *self.shader_rect = Some(ui.max_rect()),
            // The shader params, with any load/pipeline errors below them.
            Pane::Params => {
                let frame =
                    egui::Frame::central_panel(ui.style()).fill(ui.visuals().text_edit_bg_color());
                egui::CentralPanel::default()
                    .frame(frame)
                    .show_inside(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .auto_shrink([false; 2])
                            .show(ui, |ui| {
                                if let (Some(wisp), Some(inputs)) = (wisp, inputs)
                                    && wisp.schema.params.is_some()
                                {
                                    params_ui(ui, &wisp.schema, inputs);
                                }
                                // The audio capture controls, for shaders that
                                // declare `@audio`/`@audio_fft` inputs.
                                if let Some(wisp) = wisp
                                    && audio::schema_has_audio(&wisp.schema)
                                {
                                    ui.separator();
                                    audio::audio_ui(ui, audio);
                                }
                                if !errors.is_empty() {
                                    ui.separator();
                                    errors_ui(ui, errors);
                                }
                            });
                    });
            }
            // The file controls sit above the code editor. The pane paints its
            // own opaque background with no outer margin, so only the controls
            // are inset; the code editor reaches the pane edges.
            Pane::Editor => {
                let frame = egui::Frame::new().fill(ui.visuals().text_edit_bg_color());
                egui::CentralPanel::default()
                    .frame(frame)
                    .show_inside(ui, |ui| {
                        egui::Frame::new().inner_margin(8).show(ui, |ui| {
                            ui.horizontal(|ui| {
                                let selected_label = editor
                                    .selected
                                    .map(|index| editor.label(index))
                                    .unwrap_or_else(|| String::from("select a shader"));
                                egui::ComboBox::from_id_salt("shader_select")
                                    .selected_text(selected_label)
                                    .show_ui(ui, |ui| {
                                        for index in 0..editor.files.len() {
                                            let checked = editor.selected == Some(index);
                                            if ui
                                                .selectable_label(checked, editor.label(index))
                                                .clicked()
                                            {
                                                *action = Some(Action::Select(index));
                                            }
                                        }
                                    });
                                // Saving persists to the pkv store, which works
                                // on native and the web alike.
                                if ui
                                    .add_enabled(editor.dirty, egui::Button::new("save (ctrl+S)"))
                                    .clicked()
                                {
                                    *action = Some(Action::Save);
                                }
                            });

                            ui.horizontal(|ui| {
                                let name = egui::TextEdit::singleline(&mut editor.new_name)
                                    .hint_text("new_shader_name")
                                    .desired_width(180.0);
                                ui.add(name);
                                if ui.button("create").clicked()
                                    && !editor.new_name.trim().is_empty()
                                {
                                    *action = Some(Action::Create);
                                }
                            });
                            if let Some(status) = &editor.status {
                                ui.colored_label(egui::Color32::LIGHT_RED, status);
                            }
                        });

                        // The code editor fills the rest of the pane, flush to the
                        // left, right and bottom edges; its own inner text margin is
                        // enough, so an extra panel margin just looks noisy.
                        let theme = egui_extras::syntax_highlighting::CodeTheme::from_memory(
                            ui.ctx(),
                            ui.style(),
                        );
                        let mut layouter =
                            |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
                                // The simple built-in highlighter has no WGSL grammar;
                                // Rust's is close enough (fn/let/var/struct/return/comments).
                                let mut job = egui_extras::syntax_highlighting::highlight(
                                    ui.ctx(),
                                    ui.style(),
                                    &theme,
                                    buf.as_str(),
                                    "rs",
                                );
                                job.wrap.max_width = wrap_width;
                                ui.fonts_mut(|fonts| fonts.layout_job(job))
                            };
                        egui::ScrollArea::vertical().show(ui, |ui| {
                            // Freeze the text edit's frame at its resting look.
                            // Left to itself the ~1px outline brightens on hover
                            // and turns the accent colour on focus (egui picks
                            // `widgets.hovered`/`selection.stroke`); passing an
                            // explicit `Frame` takes it off that state-driven
                            // path, so the code editor stops flickering as the
                            // pointer crosses it or it gains focus.
                            let resting = ui.visuals().widgets.inactive;
                            let frame = egui::Frame::new()
                                .fill(ui.visuals().text_edit_bg_color())
                                .stroke(resting.bg_stroke)
                                .corner_radius(resting.corner_radius)
                                .inner_margin(ui.spacing().window_margin);
                            let response = ui.add_sized(
                                ui.available_size(),
                                egui::TextEdit::multiline(&mut editor.buffer)
                                    .code_editor()
                                    .frame(frame)
                                    .lock_focus(true)
                                    .layouter(&mut layouter),
                            );
                            if response.changed() {
                                editor.dirty = true;
                            }
                        });
                    });
            }
        }
        egui_tiles::UiResponse::None
    }
}

fn main() {
    App::new()
        .add_plugins((
            DefaultPlugins.set(WindowPlugin {
                primary_window: Some(Window {
                    title: String::from("bevy_wisp - editor"),
                    // Let the canvas track its parent element on the web build;
                    // ignored on native.
                    fit_canvas_to_parent: true,
                    ..default()
                }),
                ..default()
            }),
            // Wisp's panel runs in `Update`, which needs egui's single-pass
            // mode (see the `ui` example).
            #[allow(deprecated)]
            EguiPlugin {
                enable_multipass_for_primary_context: false,
                ..default()
            },
            WispPlugin,
            audio::AudioPlugin,
        ))
        // The params/errors widgets are embedded in the editor panel instead
        // of wisp's floating window.
        .insert_resource(WispConfig {
            ui_window: false,
            ..default()
        })
        // Persistent shader storage, the same on native and the web.
        .insert_resource(Pkv(PkvStore::new("nannou-org", "wisp")))
        .add_systems(Startup, setup)
        .add_systems(Update, (editor_ui, apply_default_image_inputs))
        .run();
}

/// Point every still-unset `@image` input at the built-in [`DefaultImage`].
///
/// Library inputs start image bindings at `Handle::default()` (bevy's 1x1 white
/// placeholder); swapping those for a real image is what makes an image shader
/// show a picture. Inputs already set to another image are left untouched.
fn apply_default_image_inputs(image: Res<DefaultImage>, mut cameras: Query<&mut WispInputs>) {
    for mut inputs in &mut cameras {
        let unset = inputs
            .values()
            .any(|value| matches!(value, WispValue::Image(handle) if *handle == Handle::default()));
        // Touch the inputs (and so trigger change detection) only when there is
        // actually a placeholder to replace.
        if !unset {
            continue;
        }
        for value in inputs.values_mut() {
            if let WispValue::Image(handle) = value
                && *handle == Handle::default()
            {
                *handle = image.0.clone();
            }
        }
    }
}

/// A colourful checkerboard over a gradient, the default `@image` input.
fn checkerboard(size: u32, cell: u32) -> Image {
    let mut data = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let on = ((x / cell) + (y / cell)).is_multiple_of(2);
            let r = if on { 230u8 } else { 30 };
            let g = (x * 255 / size) as u8;
            let b = (y * 255 / size) as u8;
            data.extend_from_slice(&[r, g, b, 255]);
        }
    }
    Image::new(
        Extent3d {
            width: size,
            height: size,
            depth_or_array_layers: 1,
        },
        TextureDimension::D2,
        data,
        TextureFormat::Rgba8UnormSrgb,
        RenderAssetUsages::default(),
    )
}

/// The pkv key holding a single user shader's source.
fn shader_source_key(name: &str) -> String {
    format!("shader/{name}")
}

/// The names of all user-saved shaders, in save order (empty if none yet).
fn stored_shader_names(pkv: &Pkv) -> Vec<String> {
    pkv.0.get::<Vec<String>>(SHADER_INDEX_KEY).unwrap_or_default()
}

/// A user shader's stored source, if present.
fn stored_shader_source(pkv: &Pkv, name: &str) -> Option<String> {
    pkv.0.get::<String>(shader_source_key(name)).ok()
}

/// Persist a shader's source, adding its name to the index if not already there.
fn store_shader(pkv: &mut Pkv, name: &str, source: &str) -> Result<(), bevy_pkv::SetError> {
    pkv.0.set_string(shader_source_key(name), source)?;
    let mut names = stored_shader_names(pkv);
    if !names.iter().any(|n| n == name) {
        names.push(name.to_owned());
        pkv.0.set(SHADER_INDEX_KEY, &names)?;
    }
    Ok(())
}

/// The `embedded://` asset path a bundled shader of the given name loads from.
fn bundled_asset_path(name: &str) -> String {
    format!("embedded://wisp/{name}.wgsl")
}

/// The `embedded://` asset path a user shader of the given name loads from. The
/// `user/` sub-path keeps it from colliding with a bundled shader's path, so a
/// stored shader can shadow a bundled one in the picker while both remain
/// loadable.
fn user_asset_path(name: &str) -> String {
    format!("embedded://wisp/user/{name}.wgsl")
}

/// Register the bundled shader sources with the `embedded://` asset source so
/// they load through [`Wisp`]'s loader without relying on the assets dir.
fn register_bundled(embedded: &EmbeddedAssetRegistry) {
    for &(name, source) in BUNDLED {
        let asset_path = PathBuf::from(format!("wisp/{name}.wgsl"));
        // `full_path` is only consulted by the embedded-file watcher (off here).
        embedded.insert_asset(PathBuf::new(), &asset_path, source.as_bytes());
    }
}

/// (Re)insert a user shader's source into the embedded registry so it is
/// loadable - and, after an in-place edit, re-readable on `reload` - via
/// [`user_asset_path`]. The registry's in-memory dir is shared with its reader,
/// so overwriting an entry here is what a subsequent `reload` picks up.
fn register_user_source(embedded: &EmbeddedAssetRegistry, name: &str, source: &str) {
    let asset_path = PathBuf::from(format!("wisp/user/{name}.wgsl"));
    embedded.insert_asset(PathBuf::new(), &asset_path, source.as_bytes().to_vec());
}

fn setup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    embedded: Res<EmbeddedAssetRegistry>,
    pkv: Res<Pkv>,
    mut images: ResMut<Assets<Image>>,
    mut egui_settings: ResMut<bevy_egui::EguiGlobalSettings>,
) {
    register_bundled(&embedded);
    commands.insert_resource(DefaultImage(images.add(checkerboard(512, 32))));
    // The egui UI gets its own full-window camera: bevy_egui sizes a context
    // to its camera's viewport, so hosting the UI on the wisp camera (whose
    // viewport follows the panel) would shrink the UI along with it.
    egui_settings.auto_create_primary_context = false;
    commands.spawn((
        Camera2d,
        bevy_egui::PrimaryEguiContext,
        Camera {
            order: 1,
            clear_color: bevy::camera::ClearColorConfig::None,
            ..default()
        },
    ));
    // Black behind the shader: the wisp camera clears the shader pane's
    // viewport before rendering into it.
    let camera = commands
        .spawn((
            Camera3d::default(),
            Camera {
                clear_color: bevy::camera::ClearColorConfig::Custom(Color::BLACK),
                ..default()
            },
        ))
        .id();
    let mut editor = Editor {
        files: Editor::scan(&pkv),
        selected: None,
        buffer: String::new(),
        dirty: false,
        new_name: String::new(),
        status: None,
    };
    let initial = editor
        .files
        .iter()
        .position(|file| file.name == "test_inputs")
        .or(match editor.files.is_empty() {
            true => None,
            false => Some(0),
        });
    if let Some(index) = initial {
        select(
            &mut editor,
            index,
            camera,
            &mut commands,
            &asset_server,
            &embedded,
            &pkv,
        );
    }
    commands.insert_resource(editor);
    commands.insert_resource(EditorTree(create_tree()));
}

/// The default tile layout: the code editor above the params on the left, with
/// the shader view filling the larger pane on the right.
fn create_tree() -> egui_tiles::Tree<Pane> {
    let mut tiles = egui_tiles::Tiles::default();
    let editor = tiles.insert_pane(Pane::Editor);
    let params = tiles.insert_pane(Pane::Params);
    let shader = tiles.insert_pane(Pane::Shader);
    let left = tiles.insert_container(egui_tiles::Linear::new_binary(
        egui_tiles::LinearDir::Vertical,
        [editor, params],
        0.65,
    ));
    let root = tiles.insert_container(egui_tiles::Linear::new_binary(
        egui_tiles::LinearDir::Horizontal,
        [left, shader],
        0.35,
    ));
    egui_tiles::Tree::new("wisp_editor_tree", root, tiles)
}

/// Keep every pane tabbed, but hide the shader pane's tab bar while it sits
/// alone so its view stays clean.
///
/// During a drag all tab bars stay visible (including the shader's) so a pane
/// can be dropped alongside it.
fn simplify_tree(tree: &mut egui_tiles::Tree<Pane>, ctx: &egui::Context) {
    tree.simplify(&egui_tiles::SimplificationOptions {
        all_panes_must_have_tabs: true,
        ..Default::default()
    });
    if tree.dragged_id(ctx).is_some() {
        return;
    }
    // Replace the lone shader tab container with the bare pane.
    let Some(shader_id) = tree.tiles.find_pane(&Pane::Shader) else {
        return;
    };
    let Some(parent_id) = tree.tiles.parent_of(shader_id) else {
        return;
    };
    let Some(parent) = tree.tiles.get_container(parent_id) else {
        return;
    };
    if parent.num_children() == 1 {
        tree.tiles.remove(shader_id);
        tree.tiles
            .insert(parent_id, egui_tiles::Tile::Pane(Pane::Shader));
    }
}

/// Load a shader into the editor buffer and onto the camera.
fn select(
    editor: &mut Editor,
    index: usize,
    camera: Entity,
    commands: &mut Commands,
    asset_server: &AssetServer,
    embedded: &EmbeddedAssetRegistry,
    pkv: &Pkv,
) {
    let file = &editor.files[index];
    let handle: Handle<Wisp>;
    let source = match &file.source {
        ShaderSource::Bundled(source) => {
            handle = asset_server.load(bundled_asset_path(&file.name));
            (*source).to_owned()
        }
        ShaderSource::User => {
            let Some(source) = stored_shader_source(pkv, &file.name) else {
                editor.status = Some(format!("stored shader `{}` is missing", file.name));
                return;
            };
            register_user_source(embedded, &file.name, &source);
            handle = asset_server.load(user_asset_path(&file.name));
            source
        }
    };
    editor.buffer = source;
    editor.selected = Some(index);
    editor.dirty = false;
    editor.status = None;
    commands.entity(camera).insert(WispHandle(handle));
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn editor_ui(
    mut commands: Commands,
    mut contexts: EguiContexts,
    mut editor: ResMut<Editor>,
    mut tree: ResMut<EditorTree>,
    asset_server: Res<AssetServer>,
    embedded: Res<EmbeddedAssetRegistry>,
    mut pkv: ResMut<Pkv>,
    mut audio_config: ResMut<audio::AudioConfig>,
    wisps: Res<Assets<Wisp>>,
    errors: Res<WispErrors>,
    mut cameras: Query<
        (
            Entity,
            &mut Camera,
            Option<&WispHandle>,
            Option<&mut WispInputs>,
        ),
        With<Camera3d>,
    >,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    let Ok((camera, mut camera_config, handle, mut inputs)) = cameras.single_mut() else {
        return;
    };
    let mut action = None;

    // ctrl/cmd+S saves. Consumed at the context level rather than inside a
    // pane, so it works no matter which pane has focus (or whether the editor
    // pane is the visible tab).
    let save_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::S);
    if ctx.input_mut(|input| input.consume_shortcut(&save_shortcut)) {
        action = Some(Action::Save);
    }

    let wisp = handle.and_then(|handle| wisps.get(&**handle));
    let mut shader_rect = None;
    simplify_tree(&mut tree.0, ctx);
    // Top-level `CentralPanel::show` is deprecated mid-transition in egui 0.34
    // (the replacement `show_inside` wants a root `Ui` that bevy_egui's
    // single-pass mode doesn't expose); revisit alongside the multi-pass
    // migration. The host frame is transparent so the shader pane reveals the
    // wisp camera and each other pane paints its own background.
    #[allow(deprecated)]
    egui::CentralPanel::no_frame().show(ctx, |ui| {
        let mut behavior = TreeBehavior {
            editor: &mut editor,
            errors: &errors,
            wisp,
            inputs: inputs.as_deref_mut(),
            audio: &mut audio_config,
            action: &mut action,
            shader_rect: &mut shader_rect,
        };
        tree.0.ui(&mut behavior, ui);
    });

    // The shader camera follows the shader pane. When that pane is hidden as an
    // inactive tab there is no rect, so the camera goes inactive rather than
    // leaving a stale viewport behind.
    match shader_rect {
        None => camera_config.is_active = false,
        Some(rect) => {
            camera_config.is_active = true;
            let rect = rect.intersect(ctx.content_rect());
            let scale = ctx.pixels_per_point();
            let position = UVec2::new((rect.min.x * scale) as u32, (rect.min.y * scale) as u32);
            let size = UVec2::new(
                (rect.width() * scale) as u32,
                (rect.height() * scale) as u32,
            );
            if size.x > 0 && size.y > 0 {
                camera_config.viewport = Some(Viewport {
                    physical_position: position,
                    physical_size: size,
                    ..default()
                });
            }
        }
    }

    match action {
        None => {}
        Some(Action::Select(index)) => {
            select(
                &mut editor,
                index,
                camera,
                &mut commands,
                &asset_server,
                &embedded,
                &pkv,
            );
        }
        Some(Action::Save) => {
            let Some(index) = editor.selected else {
                return;
            };
            // Always save to the store, never back to the bundled sources.
            let name = editor.files[index].name.clone();
            match store_shader(&mut pkv, &name, &editor.buffer) {
                Ok(()) => {
                    editor.dirty = false;
                    editor.status = None;
                    register_user_source(&embedded, &name, &editor.buffer);
                    if matches!(editor.files[index].source, ShaderSource::Bundled(_)) {
                        // First save of a bundled shader: the stored copy now
                        // shadows it, so point the camera at the user asset.
                        editor.files[index].source = ShaderSource::User;
                        let handle: Handle<Wisp> = asset_server.load(user_asset_path(&name));
                        commands.entity(camera).insert(WispHandle(handle));
                    } else {
                        // Re-runs the loader against the updated embedded source;
                        // errors surface via wisp's panel while the previous
                        // working shader keeps rendering.
                        asset_server.reload(user_asset_path(&name));
                    }
                }
                Err(err) => {
                    editor.status = Some(format!("failed to save `{name}`: {err}"));
                }
            }
        }
        Some(Action::Create) => {
            let name = editor.new_name.trim().replace(' ', "_");
            // Never clobber: select an existing shader of that name instead.
            if let Some(existing) = editor.files.iter().position(|file| file.name == name) {
                select(
                    &mut editor,
                    existing,
                    camera,
                    &mut commands,
                    &asset_server,
                    &embedded,
                    &pkv,
                );
                return;
            }
            match store_shader(&mut pkv, &name, NEW_SHADER_TEMPLATE) {
                Ok(()) => {
                    editor.new_name.clear();
                    editor.files.push(ShaderFile {
                        name,
                        source: ShaderSource::User,
                    });
                    let index = editor.files.len() - 1;
                    select(
                        &mut editor,
                        index,
                        camera,
                        &mut commands,
                        &asset_server,
                        &embedded,
                        &pkv,
                    );
                }
                Err(err) => {
                    editor.status = Some(format!("failed to create `{name}`: {err}"));
                }
            }
        }
    }
}
