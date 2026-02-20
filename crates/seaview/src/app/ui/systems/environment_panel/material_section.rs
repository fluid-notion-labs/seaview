//! Material section for the Environment Panel
//!
//! This module renders the material controls (base color, opacity, roughness,
//! metallic, reflectance, emissive, double-sided) inside the environment
//! panel's accordion UI, with a material preset selector at the top.

use bevy::prelude::*;
use bevy_egui::egui;

use crate::app::ui::state::MaterialConfig;
use crate::lib::lighting::MaterialPreset;

/// Persistent state for the material preset selector, stored in egui temp data.
#[derive(Clone, Default)]
struct MaterialPresetState {
    /// Currently selected preset index, or `None` for "Custom".
    selected: Option<usize>,
}

/// Renders the material controls inside an existing egui `Ui` context.
pub fn render_material_controls(ui: &mut egui::Ui, material_config: &mut MaterialConfig) {
    // ── Preset selector ───────────────────────────────────────────────
    let preset_id = egui::Id::new("mat_preset_state");
    let mut preset_state: MaterialPresetState = ui
        .data(|d| d.get_temp::<MaterialPresetState>(preset_id))
        .unwrap_or_default();

    let active_preset = preset_state
        .selected
        .and_then(|i| MaterialPreset::ALL.get(i).copied());

    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new("Preset")
                .strong()
                .color(egui::Color32::from_gray(180)),
        );
        egui::ComboBox::from_id_salt("mat_preset_combo")
            .width(ui.available_width() - 4.0)
            .selected_text(active_preset.map_or("Custom", |p| p.label()))
            .show_ui(ui, |ui| {
                if ui
                    .selectable_label(active_preset.is_none(), "Custom")
                    .clicked()
                {
                    preset_state.selected = None;
                }
                ui.separator();
                for (idx, &preset) in MaterialPreset::ALL.iter().enumerate() {
                    let selected = preset_state.selected == Some(idx);
                    let resp = ui
                        .selectable_label(selected, preset.label())
                        .on_hover_text(preset.description());
                    if resp.clicked() && !selected {
                        *material_config = preset.config();
                        preset_state.selected = Some(idx);
                    }
                }
            });
    });

    ui.data_mut(|d| d.insert_temp(preset_id, preset_state));

    ui.add_space(5.0);
    ui.separator();
    ui.add_space(5.0);

    // ── Base color ────────────────────────────────────────────────────
    ui.heading("Base Color");
    ui.add_space(2.0);
    let srgba = material_config.base_color.to_srgba();
    let mut base_color = [srgba.red, srgba.green, srgba.blue];
    if ui.color_edit_button_rgb(&mut base_color).changed() {
        material_config.base_color = Color::srgb(base_color[0], base_color[1], base_color[2]);
    }

    ui.add_space(5.0);

    // Opacity
    ui.heading("Opacity");
    ui.add_space(2.0);
    ui.add(egui::Slider::new(&mut material_config.opacity, 0.0..=1.0).text("opacity"));

    ui.add_space(5.0);

    // Roughness
    ui.heading("Roughness");
    ui.add_space(2.0);
    ui.add(
        egui::Slider::new(&mut material_config.perceptual_roughness, 0.0..=1.0).text("roughness"),
    );

    ui.add_space(5.0);

    // Metallic
    ui.heading("Metallic");
    ui.add_space(2.0);
    ui.add(egui::Slider::new(&mut material_config.metallic, 0.0..=1.0).text("metallic"));

    ui.add_space(5.0);

    // Reflectance
    ui.heading("Reflectance");
    ui.add_space(2.0);
    ui.add(egui::Slider::new(&mut material_config.reflectance, 0.0..=1.0).text("reflectance"));

    ui.add_space(5.0);

    // Emissive
    ui.heading("Emissive");
    ui.add_space(2.0);
    let em_srgba = material_config.emissive.to_srgba();
    let mut emissive_color = [em_srgba.red, em_srgba.green, em_srgba.blue];
    if ui.color_edit_button_rgb(&mut emissive_color).changed() {
        material_config.emissive =
            Color::srgb(emissive_color[0], emissive_color[1], emissive_color[2]);
    }
    ui.add(
        egui::Slider::new(&mut material_config.emissive_intensity, 0.0..=50_000.0)
            .text("intensity")
            .logarithmic(true),
    );

    ui.add_space(5.0);

    // Double-sided
    ui.checkbox(&mut material_config.double_sided, "Double Sided");

    ui.add_space(5.0);

    // Reset button
    if ui.button("Reset to Defaults").clicked() {
        *material_config = MaterialConfig::default();
    }
}
