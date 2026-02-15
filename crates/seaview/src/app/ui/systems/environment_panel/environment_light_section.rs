//! Environment Light section for the Environment Panel
//!
//! Renders controls for the global scene lighting (directional + ambient)
//! independently of the spot-light rig.  This section appears above the
//! spot-lights section in the environment panel accordion.

use bevy_egui::egui;

use crate::lib::lighting::EnvironmentLightConfig;

/// Renders the environment light controls inside an existing egui `Ui` context.
///
/// Includes:
/// - Master enable/disable toggle
/// - Ambient light colour and brightness
/// - Directional light illuminance and colour
pub fn render_environment_light_controls(ui: &mut egui::Ui, config: &mut EnvironmentLightConfig) {
    // Master toggle
    ui.horizontal(|ui| {
        ui.checkbox(&mut config.enabled, "Enabled");
    });

    if !config.enabled {
        ui.label(
            egui::RichText::new("Environment lighting is disabled")
                .small()
                .color(egui::Color32::from_gray(120)),
        );
        return;
    }

    ui.add_space(5.0);

    // ── Ambient Light ─────────────────────────────────────────────────────
    ui.heading("Ambient Light");
    ui.add_space(2.0);

    ui.label(
        egui::RichText::new("Uniform light that fills the entire scene")
            .small()
            .color(egui::Color32::from_gray(140)),
    );
    ui.add_space(2.0);

    // Ambient colour
    ui.horizontal(|ui| {
        ui.label("Color");
        let srgba = config.ambient_color.to_srgba();
        let mut color_arr = [srgba.red, srgba.green, srgba.blue];
        if ui.color_edit_button_rgb(&mut color_arr).changed() {
            config.ambient_color =
                bevy::prelude::Color::srgb(color_arr[0], color_arr[1], color_arr[2]);
        }
    });

    ui.add_space(2.0);

    // Ambient brightness
    ui.add(
        egui::Slider::new(&mut config.ambient_brightness, 0.0..=5000.0)
            .text("brightness")
            .logarithmic(true)
            .clamping(egui::SliderClamping::Always),
    );

    ui.add_space(8.0);

    // ── Directional Light ─────────────────────────────────────────────────
    ui.heading("Directional Light");
    ui.add_space(2.0);

    ui.label(
        egui::RichText::new("Sun-like light with shadows")
            .small()
            .color(egui::Color32::from_gray(140)),
    );
    ui.add_space(2.0);

    // Directional colour
    ui.horizontal(|ui| {
        ui.label("Color");
        let srgba = config.directional_color.to_srgba();
        let mut color_arr = [srgba.red, srgba.green, srgba.blue];
        if ui.color_edit_button_rgb(&mut color_arr).changed() {
            config.directional_color =
                bevy::prelude::Color::srgb(color_arr[0], color_arr[1], color_arr[2]);
        }
    });

    ui.add_space(2.0);

    // Directional illuminance
    ui.add(
        egui::Slider::new(&mut config.directional_illuminance, 0.0..=100_000.0)
            .text("lux")
            .logarithmic(true)
            .clamping(egui::SliderClamping::Always),
    );
}
