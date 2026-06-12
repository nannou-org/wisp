//! A minimal live-coding editor for wisp shaders.
//!
//! The left panel holds the param widgets and a monospace code editor with
//! syntax highlighting; the shader renders in the space to the right (the
//! camera viewport follows the panel). Pick one of the bundled shaders or
//! create your own - user shaders live in the platform data directory (e.g.
//! `~/.local/share/wisp` on Linux). Saving (button or ctrl/cmd+S) writes the
//! file and reloads the shader in place; broken edits keep the last working
//! version on screen while the error shows in the panel.

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

/// What the UI asked for this frame, applied after the panel closure.
enum Action {
    Select(usize),
    Save,
    Create,
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
    // Top-level `Panel::show` is deprecated mid-transition in egui 0.34 (the
    // replacement `show_inside` wants a root `Ui` that bevy_egui's single-pass
    // mode doesn't expose); revisit alongside the multi-pass migration.
    #[allow(deprecated)]
    let panel = egui::Panel::left("wisp_editor")
        .resizable(true)
        .default_size(460.0)
        .show(ctx, |ui| {
            // ctrl/cmd+S saves.
            let save_shortcut = egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, egui::Key::S);
            if ui.input_mut(|input| input.consume_shortcut(&save_shortcut)) {
                action = Some(Action::Save);
            }

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
                                action = Some(Action::Select(index));
                            }
                        }
                    });
                let save = ui.add_enabled(editor.dirty, egui::Button::new("save (ctrl+S)"));
                if save.clicked() {
                    action = Some(Action::Save);
                }
            });

            ui.horizontal(|ui| {
                let name = egui::TextEdit::singleline(&mut editor.new_name)
                    .hint_text("new_shader_name")
                    .desired_width(180.0);
                ui.add(name);
                if ui.button("create").clicked() && !editor.new_name.trim().is_empty() {
                    action = Some(Action::Create);
                }
            });
            if let Some(status) = &editor.status {
                ui.colored_label(egui::Color32::LIGHT_RED, status);
            }
            ui.separator();

            // Wisp's params/errors widgets, embedded above the code editor.
            errors_ui(ui, &errors);
            if let (Some(handle), Some(inputs)) = (handle, inputs.as_mut())
                && let Some(wisp) = wisps.get(&**handle)
                && wisp.schema.params.is_some()
            {
                egui::CollapsingHeader::new("params")
                    .default_open(true)
                    .show(ui, |ui| params_ui(ui, &wisp.schema, inputs));
                ui.separator();
            }

            let theme = egui_extras::syntax_highlighting::CodeTheme::from_memory(ctx, ui.style());
            let mut layouter = |ui: &egui::Ui, buf: &dyn egui::TextBuffer, wrap_width: f32| {
                // The simple built-in highlighter has no WGSL grammar; Rust's
                // is close enough (fn/let/var/struct/return/comments).
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

    // The shader renders in whatever space the panel leaves over.
    let content = ctx.content_rect();
    let scale = ctx.pixels_per_point();
    let panel_edge = panel
        .response
        .rect
        .max
        .x
        .clamp(content.min.x, content.max.x);
    let position = UVec2::new((panel_edge * scale) as u32, (content.min.y * scale) as u32);
    let size = UVec2::new(
        ((content.max.x - panel_edge) * scale) as u32,
        (content.height() * scale) as u32,
    );
    if size.x > 0 && size.y > 0 {
        camera_config.viewport = Some(Viewport {
            physical_position: position,
            physical_size: size,
            ..default()
        });
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
