//! Lighting section for the Environment Panel
//!
//! This module renders the lighting rig controls and light map visualization
//! inside the environment panel's accordion UI.

use bevy::prelude::*;
use bevy_egui::egui;

use crate::lib::lighting::{LightIntensityUnit, NightLightingConfig, PlacementAlgorithm};
use crate::lib::mesh_info::MeshDimensions;

/// Renders the lighting rig controls inside an existing egui `Ui` context.
///
/// Includes the light map visualization, enable/disable toggles, and all
/// lighting parameters (count, height, cone angle, intensity, range, color,
/// coverage, placement algorithm, debug markers).
pub fn render_lighting_controls(
    ui: &mut egui::Ui,
    lighting_config: &mut NightLightingConfig,
    mesh_dims: &MeshDimensions,
) {
    // Light map visualization — always visible at top
    render_light_map(ui, lighting_config, mesh_dims);

    ui.add_space(5.0);

    // Enable/disable toggle
    ui.horizontal(|ui| {
        ui.checkbox(&mut lighting_config.enabled, "Enabled");
    });

    if !lighting_config.enabled {
        ui.label("Spot lighting is disabled");
        return;
    }

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
    let cone_changed = ui
        .add(egui::Slider::new(&mut lighting_config.cone_angle, 5.0..=180.0).text("°"))
        .changed();

    if cone_changed {
        // Cone angle affects candela conversion - update display value
        lighting_config.sync_intensity_from_lumens();
    }

    ui.add_space(5.0);

    // Intensity
    ui.heading("Intensity");
    ui.add_space(2.0);

    // Unit selector
    let current_unit_name = lighting_config.intensity_unit.name();
    egui::ComboBox::from_label("Unit")
        .selected_text(current_unit_name)
        .show_ui(ui, |ui| {
            for unit in LightIntensityUnit::all() {
                if ui
                    .selectable_value(&mut lighting_config.intensity_unit, *unit, unit.name())
                    .clicked()
                {
                    lighting_config.set_intensity_unit(*unit);
                }
            }
        });

    ui.add_space(2.0);

    // Intensity slider (in selected unit)
    let min_val = match lighting_config.intensity_unit {
        LightIntensityUnit::Lumens => 1000.0,
        LightIntensityUnit::Candela => 1000.0,
        LightIntensityUnit::Kilolumens => 1.0,
    };
    let max_val = match lighting_config.intensity_unit {
        LightIntensityUnit::Lumens => 10_000_000.0,
        LightIntensityUnit::Candela => 20_000_000.0,
        LightIntensityUnit::Kilolumens => 10_000.0,
    };

    let changed = ui
        .add(
            egui::Slider::new(
                &mut lighting_config.intensity_display_value,
                min_val..=max_val,
            )
            .text(lighting_config.intensity_unit.symbol())
            .logarithmic(true),
        )
        .changed();

    if changed {
        lighting_config.sync_intensity_to_lumens();
    }

    // Show equivalent in lumens (Bevy's native unit)
    if lighting_config.intensity_unit != LightIntensityUnit::Lumens {
        ui.label(
            egui::RichText::new(format!("≈ {:.0} lm", lighting_config.intensity))
                .small()
                .color(egui::Color32::from_gray(140)),
        );
    }

    ui.add_space(5.0);

    // Range
    ui.heading("Range");
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new("Distance light travels before fading to zero")
            .small()
            .color(egui::Color32::from_gray(140)),
    );
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
    ui.add(egui::Slider::new(&mut lighting_config.coverage_pct, 10.0..=300.0).text("%"));

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
                    .selectable_value(&mut lighting_config.placement_algorithm, *alg, alg.name())
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
}

// ── Light Map Visualization ────────────────────────────────────────────────

/// Renders a top-down light map showing the domain AABB, coverage boundary,
/// cone footprints, and light positions.
fn render_light_map(
    ui: &mut egui::Ui,
    lighting_config: &NightLightingConfig,
    mesh_dims: &MeshDimensions,
) {
    // Compute mesh AABB (domain) in XZ
    let (domain_min, domain_max) = if let (Some(mn), Some(mx)) = (mesh_dims.min, mesh_dims.max) {
        (
            bevy::math::Vec2::new(mn.x, mn.z),
            bevy::math::Vec2::new(mx.x, mx.z),
        )
    } else {
        (
            bevy::math::Vec2::new(-50.0, -50.0),
            bevy::math::Vec2::new(50.0, 50.0),
        )
    };

    // Coverage bounds (where lights are placed)
    let coverage_scale = lighting_config.coverage_pct / 100.0;
    let domain_center = (domain_min + domain_max) * 0.5;
    let domain_half = (domain_max - domain_min) * 0.5;
    let coverage_min = domain_center - domain_half * coverage_scale;
    let coverage_max = domain_center + domain_half * coverage_scale;

    // View bounds — padded slightly beyond whichever is larger (domain or coverage)
    let view_min = bevy::math::Vec2::new(
        coverage_min.x.min(domain_min.x),
        coverage_min.y.min(domain_min.y),
    );
    let view_max = bevy::math::Vec2::new(
        coverage_max.x.max(domain_max.x),
        coverage_max.y.max(domain_max.y),
    );
    let view_extent = view_max - view_min;
    let padding_frac = 0.08;
    let pad = bevy::math::Vec2::new(view_extent.x * padding_frac, view_extent.y * padding_frac);
    let view_min = view_min - pad;
    let view_max = view_max + pad;
    let view_w = view_max.x - view_min.x;
    let view_h = view_max.y - view_min.y;

    let positions = lighting_config.placement_algorithm.calculate_positions(
        lighting_config.num_lights,
        coverage_min,
        coverage_max,
    );

    // Use full available width for the map
    let map_size = ui.available_width().min(280.0);
    let (rect, _response) =
        ui.allocate_exact_size(egui::Vec2::splat(map_size), egui::Sense::hover());

    // Helper: world XZ → pixel position in the map rect
    let to_px = |wx: f32, wz: f32| -> egui::Pos2 {
        egui::pos2(
            rect.min.x + ((wx - view_min.x) / view_w) * map_size,
            rect.min.y + ((wz - view_min.y) / view_h) * map_size,
        )
    };

    if ui.is_rect_visible(rect) {
        let painter = ui.painter_at(rect);

        // Background
        painter.rect_filled(rect, 4.0, egui::Color32::from_gray(20));

        // --- Coverage boundary (dashed-style: thin bright rect) ---
        let cov_tl = to_px(coverage_min.x, coverage_min.y);
        let cov_br = to_px(coverage_max.x, coverage_max.y);
        let cov_rect = egui::Rect::from_min_max(cov_tl, cov_br);
        // Subtle fill to distinguish the coverage zone
        painter.rect_filled(
            cov_rect,
            2.0,
            egui::Color32::from_rgba_unmultiplied(60, 90, 120, 25),
        );
        painter.rect_stroke(
            cov_rect,
            2.0,
            egui::Stroke::new(
                1.0,
                egui::Color32::from_rgba_unmultiplied(80, 140, 200, 140),
            ),
            egui::StrokeKind::Outside,
        );
        // Label
        painter.text(
            egui::pos2(cov_tl.x + 3.0, cov_tl.y + 2.0),
            egui::Align2::LEFT_TOP,
            format!("Coverage {:.0}%", lighting_config.coverage_pct),
            egui::FontId::proportional(7.5),
            egui::Color32::from_rgba_unmultiplied(80, 140, 200, 160),
        );

        // --- Domain AABB (mesh footprint) ---
        let dom_tl = to_px(domain_min.x, domain_min.y);
        let dom_br = to_px(domain_max.x, domain_max.y);
        let dom_rect = egui::Rect::from_min_max(dom_tl, dom_br);
        // Subtle domain fill
        painter.rect_filled(
            dom_rect,
            1.0,
            egui::Color32::from_rgba_unmultiplied(180, 180, 180, 15),
        );
        painter.rect_stroke(
            dom_rect,
            1.0,
            egui::Stroke::new(
                1.5,
                egui::Color32::from_rgba_unmultiplied(200, 200, 200, 200),
            ),
            egui::StrokeKind::Outside,
        );
        // Domain label
        if let Some(dims) = mesh_dims.dimensions {
            painter.text(
                egui::pos2(dom_br.x - 3.0, dom_br.y - 2.0),
                egui::Align2::RIGHT_BOTTOM,
                format!("Domain {:.0}×{:.0}", dims.x, dims.z),
                egui::FontId::proportional(7.5),
                egui::Color32::from_rgba_unmultiplied(200, 200, 200, 160),
            );
        } else {
            painter.text(
                egui::pos2(dom_br.x - 3.0, dom_br.y - 2.0),
                egui::Align2::RIGHT_BOTTOM,
                "Domain",
                egui::FontId::proportional(7.5),
                egui::Color32::from_rgba_unmultiplied(200, 200, 200, 160),
            );
        }

        // --- Cone ground projections + light dots ---
        if view_w > 0.0 && view_h > 0.0 {
            let srgba = lighting_config.color.to_srgba();
            let light_color = egui::Color32::from_rgb(
                (srgba.red * 255.0) as u8,
                (srgba.green * 255.0) as u8,
                (srgba.blue * 255.0) as u8,
            );

            // Cone ground radius in world units
            let cone_world_radius =
                lighting_config.height * (lighting_config.cone_angle / 2.0).to_radians().tan();
            // Convert to pixels (use average of x/z scale)
            let px_per_unit = map_size / ((view_w + view_h) * 0.5);
            let cone_px = (cone_world_radius * px_per_unit).min(map_size * 0.4);

            for pos in &positions {
                let center = to_px(pos.x, pos.y);

                // Glow circle (cone footprint estimation)
                let glow_color = egui::Color32::from_rgba_unmultiplied(
                    (srgba.red * 255.0) as u8,
                    (srgba.green * 255.0) as u8,
                    (srgba.blue * 255.0) as u8,
                    25,
                );
                painter.circle_filled(center, cone_px, glow_color);

                // Cone edge ring
                painter.circle_stroke(
                    center,
                    cone_px,
                    egui::Stroke::new(
                        0.5,
                        egui::Color32::from_rgba_unmultiplied(
                            (srgba.red * 255.0) as u8,
                            (srgba.green * 255.0) as u8,
                            (srgba.blue * 255.0) as u8,
                            50,
                        ),
                    ),
                );

                // Light dot
                painter.circle_filled(center, 3.0, light_color);
            }
        }

        // Border
        painter.rect_stroke(
            rect,
            4.0,
            egui::Stroke::new(1.0, egui::Color32::from_gray(50)),
            egui::StrokeKind::Outside,
        );
    }

    // Info line below the map
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 8.0;
        ui.label(
            egui::RichText::new(format!(
                "{} lights · {:.0}m · {}",
                positions.len(),
                lighting_config.height,
                lighting_config.placement_algorithm.name(),
            ))
            .small()
            .color(egui::Color32::from_gray(160)),
        );
    });
}
