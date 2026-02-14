//! Material section for the Environment Panel
//!
//! This module renders the material controls (base color, opacity, roughness,
//! metallic, reflectance, emissive, double-sided) inside the environment
//! panel's accordion UI.

use bevy::prelude::*;
use bevy_egui::egui;

use crate::app::ui::state::MaterialConfig;

/// Renders the material controls inside an existing egui `Ui` context.
pub fn render_material_controls(ui: &mut egui::Ui, material_config: &mut MaterialConfig) {
    // Base color
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
        egui::Slider::new(&mut material_config.emissive_intensity, 0.0..=100.0)
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
