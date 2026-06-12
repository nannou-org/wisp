//! Auto-generated egui control panel (feature `ui`).
//!
//! A reflection-driven panel for every wisp camera's inputs: sliders for ranged
//! floats, checkboxes for `@bool`, dropdowns for `@values`, colour pickers for
//! `@color` and drag rows for vectors. [`WispErrors`] are shown in a collapsible
//! section so live-coding failures are visible on screen.
//!
//! Requires `bevy_egui`'s `EguiPlugin` to be added to the app (the
//! panel is inert without it).

use crate::asset::{Wisp, WispHandle};
use crate::error::WispErrors;
use crate::inputs::{WispInputs, WispValue};
use crate::schema::{ParamField, UiHints};
use bevy::prelude::*;
use bevy_egui::EguiContexts;
use bevy_egui::egui;

pub(crate) fn wisp_ui(
    mut contexts: EguiContexts,
    wisps: Res<Assets<Wisp>>,
    errors: Res<WispErrors>,
    asset_server: Res<AssetServer>,
    mut cameras: Query<(&WispHandle, &mut WispInputs)>,
) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };
    egui::Window::new("wisp")
        .default_width(300.0)
        .show(ctx, |ui| {
            errors_section(ui, &errors);
            for (handle, mut inputs) in cameras.iter_mut() {
                let Some(wisp) = wisps.get(&**handle) else {
                    continue;
                };
                if let Some(path) = asset_server.get_path(handle.id()) {
                    ui.strong(path.to_string());
                }
                if !wisp.schema.description.is_empty() {
                    ui.label(&wisp.schema.description);
                }
                if let Some(params) = &wisp.schema.params {
                    for field in &params.fields {
                        if let Some(value) = inputs.get_mut(&field.name) {
                            field_row(ui, field, value);
                        }
                    }
                }
                ui.separator();
            }
        });
}

fn errors_section(ui: &mut egui::Ui, errors: &WispErrors) {
    if errors.is_empty() {
        return;
    }
    let header = egui::RichText::new("errors")
        .color(egui::Color32::RED)
        .strong();
    egui::CollapsingHeader::new(header)
        .default_open(true)
        .show(ui, |ui| {
            for (source, message) in errors.load.iter().chain(&errors.pipeline) {
                ui.colored_label(egui::Color32::LIGHT_RED, source);
                ui.monospace(message);
                ui.separator();
            }
        });
}

fn field_row(ui: &mut egui::Ui, field: &ParamField, value: &mut WispValue) {
    let label = field.ui.label.as_deref().unwrap_or(&field.name);
    let hints = &field.ui;
    let response = match value {
        WispValue::F32(v) => float_row(ui, label, hints, v),
        WispValue::Bool(v) => ui.checkbox(v, label),
        WispValue::I32(v) => int_row(ui, label, hints, v),
        WispValue::U32(v) => int_row(ui, label, hints, v),
        WispValue::Vec2(v) => {
            let mut components = v.to_array();
            let response = drag_row(ui, label, hints, &mut components);
            *v = Vec2::from_array(components);
            response
        }
        WispValue::Vec3(v) if hints.color => {
            let mut rgb = v.to_array();
            let response = ui
                .horizontal(|ui| {
                    ui.color_edit_button_rgb(&mut rgb);
                    ui.label(label);
                })
                .response;
            *v = Vec3::from_array(rgb);
            response
        }
        WispValue::Vec3(v) => {
            let mut components = v.to_array();
            let response = drag_row(ui, label, hints, &mut components);
            *v = Vec3::from_array(components);
            response
        }
        WispValue::Vec4(v) if hints.color => {
            let mut rgba = v.to_array();
            let response = ui
                .horizontal(|ui| {
                    ui.color_edit_button_rgba_unmultiplied(&mut rgba);
                    ui.label(label);
                })
                .response;
            *v = Vec4::from_array(rgba);
            response
        }
        WispValue::Vec4(v) => {
            let mut components = v.to_array();
            let response = drag_row(ui, label, hints, &mut components);
            *v = Vec4::from_array(components);
            response
        }
        WispValue::Image(_) => ui.label(format!("{label} (image input)")),
    };
    if !hints.description.is_empty() {
        response.on_hover_text(&hints.description);
    }
}

fn float_row(ui: &mut egui::Ui, label: &str, hints: &UiHints, value: &mut f32) -> egui::Response {
    if let (Some(min), Some(max)) = (hints.min, hints.max) {
        let mut slider = egui::Slider::new(value, min as f32..=max as f32).text(label);
        if let Some(step) = hints.step {
            slider = slider.step_by(step);
        }
        return ui.add(slider);
    }
    ui.horizontal(|ui| {
        let mut drag = egui::DragValue::new(value).speed(hints.step.unwrap_or(0.01));
        if let (Some(min), Some(max)) = (hints.min, hints.max) {
            drag = drag.range(min..=max);
        }
        ui.add(drag);
        ui.label(label);
    })
    .response
}

fn int_row<T: egui::emath::Numeric>(
    ui: &mut egui::Ui,
    label: &str,
    hints: &UiHints,
    value: &mut T,
) -> egui::Response {
    if hints.values.is_empty() {
        return ui
            .horizontal(|ui| {
                let mut drag = egui::DragValue::new(value).speed(hints.step.unwrap_or(1.0));
                if let (Some(min), Some(max)) = (hints.min, hints.max) {
                    drag = drag.range(min..=max);
                }
                ui.add(drag);
                ui.label(label);
            })
            .response;
    }
    let display = |index: usize, value: i64| {
        hints
            .labels
            .get(index)
            .cloned()
            .unwrap_or_else(|| value.to_string())
    };
    let current = value.to_f64() as i64;
    let selected_text = hints
        .values
        .iter()
        .position(|v| *v == current)
        .map(|index| display(index, current))
        .unwrap_or_else(|| current.to_string());
    egui::ComboBox::from_label(label)
        .selected_text(selected_text)
        .show_ui(ui, |ui| {
            for (index, v) in hints.values.iter().enumerate() {
                ui.selectable_value(value, T::from_f64(*v as f64), display(index, *v));
            }
        })
        .response
}

fn drag_row(
    ui: &mut egui::Ui,
    label: &str,
    hints: &UiHints,
    components: &mut [f32],
) -> egui::Response {
    ui.horizontal(|ui| {
        for component in components.iter_mut() {
            ui.add(egui::DragValue::new(component).speed(hints.step.unwrap_or(0.01)));
        }
        ui.label(label);
    })
    .response
}
