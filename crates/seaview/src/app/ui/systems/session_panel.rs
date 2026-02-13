//! Inspector panel system for Seaview
//!
//! This module implements the left side panel that displays scene controls
//! using an accordion-style UI: lighting rig, material, and rendering.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::app::ui::state::{MaterialConfig, RenderConfig, UiState};
use crate::lib::lighting::{NightLightingConfig, PlacementAlgorithm};
use crate::lib::mesh_info::MeshDimensions;
use crate::lib::sequence::SequenceAssets;

/// System that renders the inspector panel (left sidebar).
///
/// Contains collapsible sections for Lighting Rig, Material, and Rendering.
pub fn session_panel_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    mut lighting_config: ResMut<NightLightingConfig>,
    mut material_config: ResMut<MaterialConfig>,
    mut render_config: ResMut<RenderConfig>,
    mesh_dims: Res<MeshDimensions>,
    sequence_assets: Res<SequenceAssets>,
) {
    if !ui_state.show_session_panel {
        return;
    }

    let ctx = contexts.ctx_mut().unwrap();

    let panel_width = ui_state.panel_sizes.session_panel_width;

    egui::SidePanel::left("session_panel")
        .resizable(true)
        .default_width(panel_width)
        .width_range(200.0..=500.0)
        .show(ctx, |ui| {
            // Update panel width if resized
            ui_state.panel_sizes.session_panel_width = ui.available_width();

            // Main scroll area for all accordion sections
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // Lighting Rig Section (Collapsible)
                    egui::CollapsingHeader::new("Lighting Rig")
                        .default_open(ui_state.collapsible.lighting_rig_open)
                        .show(ui, |ui| {
                            ui_state.collapsible.lighting_rig_open = true;

                            render_lighting_rig_controls(ui, &mut lighting_config, &mesh_dims);
                        });

                    ui.add_space(5.0);

                    // Material Section (Collapsible)
                    egui::CollapsingHeader::new("Material")
                        .default_open(ui_state.collapsible.material_open)
                        .show(ui, |ui| {
                            ui_state.collapsible.material_open = true;

                            render_material_controls(ui, &mut material_config);
                        });

                    ui.add_space(5.0);

                    // Rendering Section (Collapsible)
                    render_rendering_section(
                        ui,
                        &mut ui_state,
                        &mut render_config,
                        &sequence_assets,
                    );
                });
        });
}

fn render_lighting_rig_controls(
    ui: &mut egui::Ui,
    lighting_config: &mut NightLightingConfig,
    mesh_dims: &MeshDimensions,
) {
    // Enable/disable toggle
    ui.horizontal(|ui| {
        ui.checkbox(&mut lighting_config.enabled, "Enabled");
    });

    if !lighting_config.enabled {
        ui.label("Night lighting is disabled");
        return;
    }

    ui.add_space(5.0);

    // Global lighting toggle
    ui.horizontal(|ui| {
        ui.checkbox(&mut lighting_config.global_lighting_enabled, "Global Lights");
    });

    ui.add_space(5.0);

    // Number of lights
    ui.heading("Light Count");
    ui.add_space(2.0);
    ui.add(
        egui::Slider::new(&mut lighting_config.num_lights, 1..=64)
            .text("lights")
            .logarithmic(true),
    );

    ui.add_space(5.0);

    // Height
    ui.heading("Height");
    ui.add_space(2.0);
    let max_height = mesh_dims
        .dimensions
        .map(|d| d.y * 5.0)
        .unwrap_or(500.0)
        .max(10.0);
    ui.add(
        egui::Slider::new(&mut lighting_config.height, 1.0..=max_height)
            .text("m")
            .logarithmic(true),
    );

    ui.add_space(5.0);

    // Cone angle
    ui.heading("Cone Angle");
    ui.add_space(2.0);
    ui.add(egui::Slider::new(&mut lighting_config.cone_angle, 5.0..=180.0).text("°"));

    ui.add_space(5.0);

    // Intensity
    ui.heading("Intensity");
    ui.add_space(2.0);
    ui.add(
        egui::Slider::new(&mut lighting_config.intensity, 1000.0..=10_000_000.0)
            .text("cd")
            .logarithmic(true),
    );

    ui.add_space(5.0);

    // Range
    ui.heading("Range");
    ui.add_space(2.0);
    ui.add(
        egui::Slider::new(&mut lighting_config.range, 10.0..=5000.0)
            .text("m")
            .logarithmic(true),
    );

    ui.add_space(5.0);

    // Color
    ui.heading("Light Color");
    ui.add_space(2.0);
    let srgba = lighting_config.color.to_srgba();
    let mut color_arr = [srgba.red, srgba.green, srgba.blue];
    if ui.color_edit_button_rgb(&mut color_arr).changed() {
        lighting_config.color = Color::srgb(color_arr[0], color_arr[1], color_arr[2]);
    }

    ui.add_space(5.0);

    // Coverage
    ui.heading("Coverage");
    ui.add_space(2.0);
    ui.add(egui::Slider::new(&mut lighting_config.coverage_pct, 50.0..=300.0).text("%"));

    ui.add_space(5.0);

    // Placement algorithm
    ui.heading("Placement");
    ui.add_space(2.0);
    let current_name = lighting_config.placement_algorithm.name();
    egui::ComboBox::from_label("Algorithm")
        .selected_text(current_name)
        .show_ui(ui, |ui| {
            for alg in PlacementAlgorithm::all() {
                if ui
                    .selectable_value(
                        &mut lighting_config.placement_algorithm,
                        *alg,
                        alg.name(),
                    )
                    .clicked()
                {
                    // Selection applied via selectable_value
                }
            }
        });

    ui.add_space(5.0);

    // Markers
    ui.heading("Debug");
    ui.add_space(2.0);
    ui.checkbox(&mut lighting_config.show_markers, "Show Light Markers");
    if lighting_config.show_markers {
        ui.add(
            egui::Slider::new(&mut lighting_config.marker_size, 0.1..=5.0)
                .text("marker size")
                .logarithmic(true),
        );
    }

    ui.add_space(5.0);

    // Light map visualization
    render_light_map(ui, lighting_config, mesh_dims);
}

fn render_light_map(
    ui: &mut egui::Ui,
    lighting_config: &NightLightingConfig,
    mesh_dims: &MeshDimensions,
) {
    egui::CollapsingHeader::new("Light Map Preview")
        .default_open(false)
        .show(ui, |ui| {
            let (min_bounds, max_bounds) = if let (Some(mn), Some(mx)) = (mesh_dims.min, mesh_dims.max) {
                let coverage_scale = lighting_config.coverage_pct / 100.0;
                let center_x = (mn.x + mx.x) / 2.0;
                let center_z = (mn.z + mx.z) / 2.0;
                let half_x = (mx.x - mn.x) / 2.0 * coverage_scale;
                let half_z = (mx.z - mn.z) / 2.0 * coverage_scale;

                (
                    bevy::math::Vec2::new(center_x - half_x, center_z - half_z),
                    bevy::math::Vec2::new(center_x + half_x, center_z + half_z),
                )
            } else {
                (
                    bevy::math::Vec2::new(-50.0, -50.0),
                    bevy::math::Vec2::new(50.0, 50.0),
                )
            };

            let positions = lighting_config.placement_algorithm.calculate_positions(
                lighting_config.num_lights,
                min_bounds,
                max_bounds,
            );

            let map_size = 180.0_f32;
            let (rect, _response) =
                ui.allocate_exact_size(egui::Vec2::splat(map_size), egui::Sense::hover());

            if ui.is_rect_visible(rect) {
                let painter = ui.painter_at(rect);

                // Background
                painter.rect_filled(rect, 4.0, egui::Color32::from_gray(30));

                // Mesh outline (in map space)
                if let (Some(mn), Some(mx)) = (mesh_dims.min, mesh_dims.max) {
                    let world_w = max_bounds.x - min_bounds.x;
                    let world_h = max_bounds.y - min_bounds.y;

                    if world_w > 0.0 && world_h > 0.0 {
                        let mesh_min_x =
                            rect.min.x + ((mn.x - min_bounds.x) / world_w) * map_size;
                        let mesh_min_y =
                            rect.min.y + ((mn.z - min_bounds.y) / world_h) * map_size;
                        let mesh_max_x =
                            rect.min.x + ((mx.x - min_bounds.x) / world_w) * map_size;
                        let mesh_max_y =
                            rect.min.y + ((mx.z - min_bounds.y) / world_h) * map_size;

                        let mesh_rect = egui::Rect::from_min_max(
                            egui::pos2(mesh_min_x, mesh_min_y),
                            egui::pos2(mesh_max_x, mesh_max_y),
                        );
                        painter.rect_stroke(
                            mesh_rect,
                            2.0,
                            egui::Stroke::new(1.0, egui::Color32::from_gray(80)),
                            egui::StrokeKind::Outside,
                        );
                    }
                }

                // Light positions
                let world_w = max_bounds.x - min_bounds.x;
                let world_h = max_bounds.y - min_bounds.y;

                if world_w > 0.0 && world_h > 0.0 {
                    let srgba = lighting_config.color.to_srgba();
                    let light_color = egui::Color32::from_rgb(
                        (srgba.red * 255.0) as u8,
                        (srgba.green * 255.0) as u8,
                        (srgba.blue * 255.0) as u8,
                    );

                    for pos in &positions {
                        let px =
                            rect.min.x + ((pos.x - min_bounds.x) / world_w) * map_size;
                        let py =
                            rect.min.y + ((pos.y - min_bounds.y) / world_h) * map_size;

                        // Glow circle (represents cone coverage area)
                        let cone_radius = (lighting_config.height
                            * (lighting_config.cone_angle / 2.0).to_radians().tan())
                            / world_w
                            * map_size;
                        let glow_color = egui::Color32::from_rgba_unmultiplied(
                            (srgba.red * 255.0) as u8,
                            (srgba.green * 255.0) as u8,
                            (srgba.blue * 255.0) as u8,
                            30,
                        );
                        painter.circle_filled(
                            egui::pos2(px, py),
                            cone_radius.min(map_size * 0.3),
                            glow_color,
                        );

                        // Light dot
                        painter.circle_filled(egui::pos2(px, py), 3.0, light_color);
                    }
                }

                // Border
                painter.rect_stroke(
                    rect,
                    4.0,
                    egui::Stroke::new(1.0, egui::Color32::from_gray(60)),
                    egui::StrokeKind::Outside,
                );
            }

            ui.add_space(4.0);
            ui.label(format!(
                "{} lights, {:.0}m high",
                positions.len(),
                lighting_config.height
            ));
        });
}

fn render_material_controls(ui: &mut egui::Ui, material_config: &mut MaterialConfig) {
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

pub fn delete_confirmation_dialog_system(
    mut contexts: EguiContexts,
    mut _ui_state: ResMut<UiState>,
) {
    // Stub — session deletion is no longer exposed in the UI.
    // Kept as a no-op to satisfy the system registration in UiSystemsPlugin.
    let _ctx = contexts.ctx_mut();
}

fn render_rendering_section(
    ui: &mut egui::Ui,
    ui_state: &mut UiState,
    render_config: &mut RenderConfig,
    sequence_assets: &SequenceAssets,
) {
    egui::CollapsingHeader::new("🖥 Rendering")
        .default_open(ui_state.collapsible.rendering_open)
        .show(ui, |ui| {
            ui_state.collapsible.rendering_open = true;

            // Frame window size slider
            ui.heading("Frame Window");
            ui.add_space(2.0);
            ui.label("Spawned entities around current frame:");
            ui.add_space(2.0);

            let max_window = sequence_assets.total_frames.max(1);
            ui.add(
                egui::Slider::new(&mut render_config.frame_window_size, 1..=max_window.min(100))
                    .text("frames")
                    .logarithmic(true),
            );

            ui.add_space(4.0);

            // Show current window stats
            let spawned = sequence_assets
                .frame_entities
                .iter()
                .filter(|e| e.is_some())
                .count();
            let total = sequence_assets.total_frames;

            egui::Grid::new("render_window_grid")
                .num_columns(2)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Spawned:");
                    ui.label(format!("{} / {}", spawned, total));
                    ui.end_row();

                    ui.label("Displayed:");
                    ui.label(
                        sequence_assets
                            .displayed_frame
                            .map(|f| format!("Frame {}", f))
                            .unwrap_or_else(|| "—".to_string()),
                    );
                    ui.end_row();

                    if total > 0 {
                        let current = sequence_assets.displayed_frame.unwrap_or(0);
                        let (lo, hi) =
                            sequence_assets.window_range(current, render_config.frame_window_size);
                        ui.label("Window:");
                        ui.label(format!("[{}, {})", lo, hi));
                        ui.end_row();
                    }
                });
        });
}
