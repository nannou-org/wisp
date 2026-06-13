//! A minimal live-coding editor for wisp shaders.
//!
//! The UI is a re-arrangeable `egui_tiles` layout of three panes: a monospace
//! code editor with syntax highlighting (and the file controls), the shader
//! param widgets, and the shader view itself. Drag a tab header to re-arrange
//! the panes; the shader pane hides its tab when it sits alone so its view
//! stays clean (the camera viewport follows that pane). Pick one of the
//! bundled shaders or create your own - user shaders live in the platform data
//! directory (e.g. `~/.local/share/wisp` on Linux). Saving (button or
//! ctrl/cmd+S) writes the file and reloads the shader in place; broken edits
//! keep the last working version on screen while the error shows in the
//! params pane.

use bevy::asset::UnapprovedPathMode;
use bevy::camera::Viewport;
use bevy::prelude::*;
use bevy_egui::{EguiContexts, EguiPlugin, egui};
use bevy_wisp::prelude::*;
use bevy_wisp::ui::{errors_ui, params_ui};
use std::path::{Path, PathBuf};

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
    path: PathBuf,
    user: bool,
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
    action: &'a mut Option<Action>,
    /// Set to the shader pane's rect so the camera viewport can follow it.
    shader_rect: &'a mut Option<egui::Rect>,
}

impl Editor {
    fn scan() -> Vec<ShaderFile> {
        let mut files = Vec::new();
        let bundled = Path::new(env!("CARGO_MANIFEST_DIR")).join("assets/wisp");
        for (dir, user) in [(bundled, false), (user_shader_dir(), true)] {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|ext| ext == "wgsl")
                    && let Some(stem) = path.file_stem()
                {
                    files.push(ShaderFile {
                        name: stem.to_string_lossy().into_owned(),
                        path,
                        user,
                    });
                }
            }
        }
        files.sort_by(|a, b| (a.user, &a.name).cmp(&(b.user, &b.name)));
        files
    }

    fn label(&self, index: usize) -> String {
        let file = &self.files[index];
        match file.user {
            true => format!("{} (user)", file.name),
            false => file.name.clone(),
        }
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

    // The tab bar matches the panel fill so it looks continuous with the panes.
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
        match pane {
            // Painted nothing: the wisp camera renders through this pane, and
            // the camera viewport is fitted to its rect after the tree draws.
            Pane::Shader => *self.shader_rect = Some(ui.max_rect()),
            // The shader params, with any load/pipeline errors below them.
            Pane::Params => {
                egui::CentralPanel::default().show_inside(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .auto_shrink([false; 2])
                        .show(ui, |ui| {
                            if let (Some(wisp), Some(inputs)) = (wisp, inputs)
                                && wisp.schema.params.is_some()
                            {
                                params_ui(ui, &wisp.schema, inputs);
                            }
                            if !errors.is_empty() {
                                ui.separator();
                                errors_ui(ui, errors);
                            }
                        });
                });
            }
            // The file controls above the syntax-highlighted code editor.
            Pane::Editor => {
                egui::CentralPanel::default().show_inside(ui, |ui| {
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
                                    if ui.selectable_label(checked, editor.label(index)).clicked() {
                                        *action = Some(Action::Select(index));
                                    }
                                }
                            });
                        let save = ui.add_enabled(editor.dirty, egui::Button::new("save (ctrl+S)"));
                        if save.clicked() {
                            *action = Some(Action::Save);
                        }
                    });

                    ui.horizontal(|ui| {
                        let name = egui::TextEdit::singleline(&mut editor.new_name)
                            .hint_text("new_shader_name")
                            .desired_width(180.0);
                        ui.add(name);
                        if ui.button("create").clicked() && !editor.new_name.trim().is_empty() {
                            *action = Some(Action::Create);
                        }
                    });
                    if let Some(status) = &editor.status {
                        ui.colored_label(egui::Color32::LIGHT_RED, status);
                    }

                    let theme =
                        egui_extras::syntax_highlighting::CodeTheme::from_memory(ui.ctx(), ui.style());
                    let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
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
                        let response = ui.add_sized(
                            ui.available_size(),
                            egui::TextEdit::multiline(&mut editor.buffer)
                                .code_editor()
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
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: String::from("bevy_wisp - editor"),
                        ..default()
                    }),
                    ..default()
                })
                // User shaders live outside the assets dir.
                .set(AssetPlugin {
                    unapproved_path_mode: UnapprovedPathMode::Allow,
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
        ))
        // The params/errors widgets are embedded in the editor panel instead
        // of wisp's floating window.
        .insert_resource(WispConfig {
            ui_window: false,
            ..default()
        })
        .add_systems(Startup, setup)
        .add_systems(Update, editor_ui)
        .run();
}

/// The directory user shaders are kept in, e.g. `~/.local/share/wisp`.
fn user_shader_dir() -> PathBuf {
    dirs::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("wisp")
}

fn setup(
    mut commands: Commands,
    asset_server: Res<AssetServer>,
    mut egui_settings: ResMut<bevy_egui::EguiGlobalSettings>,
) {
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
    let camera = commands.spawn(Camera3d::default()).id();
    let mut editor = Editor {
        files: Editor::scan(),
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
        select(&mut editor, index, camera, &mut commands, &asset_server);
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
) {
    let file = &editor.files[index];
    match std::fs::read_to_string(&file.path) {
        Ok(source) => {
            editor.buffer = source;
            editor.selected = Some(index);
            editor.dirty = false;
            editor.status = None;
            let wisp: Handle<Wisp> = asset_server.load(file.path.clone());
            commands.entity(camera).insert(WispHandle(wisp));
        }
        Err(err) => editor.status = Some(format!("failed to read {}: {err}", file.path.display())),
    }
}

#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn editor_ui(
    mut commands: Commands,
    mut contexts: EguiContexts,
    mut editor: ResMut<Editor>,
    mut tree: ResMut<EditorTree>,
    asset_server: Res<AssetServer>,
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
            let size = UVec2::new((rect.width() * scale) as u32, (rect.height() * scale) as u32);
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
            select(&mut editor, index, camera, &mut commands, &asset_server);
        }
        Some(Action::Save) => {
            let Some(index) = editor.selected else {
                return;
            };
            let path = editor.files[index].path.clone();
            match std::fs::write(&path, &editor.buffer) {
                Ok(()) => {
                    editor.dirty = false;
                    editor.status = None;
                    // Re-runs the loader; errors surface via wisp's panel while
                    // the previous working shader keeps rendering.
                    asset_server.reload(path);
                }
                Err(err) => {
                    editor.status = Some(format!("failed to write {}: {err}", path.display()));
                }
            }
        }
        Some(Action::Create) => {
            let name = editor.new_name.trim().replace(' ', "_");
            let dir = user_shader_dir();
            let path = dir.join(format!("{name}.wgsl"));
            // Never clobber: select the existing file of that name instead.
            if let Some(existing) = editor.files.iter().position(|file| file.path == path) {
                select(&mut editor, existing, camera, &mut commands, &asset_server);
                return;
            }
            let written = std::fs::create_dir_all(&dir)
                .and_then(|()| std::fs::write(&path, NEW_SHADER_TEMPLATE));
            match written {
                Ok(()) => {
                    editor.files.push(ShaderFile {
                        name,
                        path,
                        user: true,
                    });
                    editor.new_name.clear();
                    let index = editor.files.len() - 1;
                    select(&mut editor, index, camera, &mut commands, &asset_server);
                }
                Err(err) => {
                    editor.status = Some(format!("failed to create {}: {err}", path.display()));
                }
            }
        }
    }
}
