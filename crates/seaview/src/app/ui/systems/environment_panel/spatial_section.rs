//! Spatial section for the Environment Panel
//!
//! This module renders spatial mesh diagnostics (BVH build status, triangle
//! count, AABB dimensions, build time) and BVH wireframe visualization
//! controls inside the environment panel's accordion UI.

use bevy_egui::egui;

use crate::app::systems::bvh_gizmos::BvhGizmoConfig;
use crate::app::ui::state::UiState;
use crate::lib::sequence::loader::SpatialIndex;
use crate::lib::sequence::FrameStore;

/// Renders the spatial section inside an existing egui `Ui` context.
///
/// Shows BVH build progress, per-frame spatial mesh stats (triangles,
/// vertices, AABB), cumulative build time, and BVH wireframe visualization
/// controls.
pub fn render_spatial_section(
    ui: &mut egui::Ui,
    ui_state: &mut UiState,
    frame_store: &FrameStore,
    spatial_index: &SpatialIndex,
    bvh_config: &mut BvhGizmoConfig,
) {
    egui::CollapsingHeader::new("\u{1f4d0} Spatial")
        .default_open(ui_state.collapsible.spatial_open)
        .show(ui, |ui| {
            ui_state.collapsible.spatial_open = true;

            // ── Availability indicator ─────────────────────────────
            let built = spatial_index.built_count();
            let total = spatial_index.total_slots();

            let (status_text, status_color) = if total == 0 {
                ("No sequence loaded", egui::Color32::GRAY)
            } else if built == total {
                ("\u{2714} Available", egui::Color32::from_rgb(100, 255, 100))
            } else if built > 0 {
                (
                    "\u{231b} Building\u{2026}",
                    egui::Color32::from_rgb(255, 255, 100),
                )
            } else {
                ("Waiting for frames", egui::Color32::GRAY)
            };

            ui.horizontal(|ui| {
                ui.label("Status:");
                ui.colored_label(status_color, status_text);
            });

            if total > 0 {
                ui.add_space(2.0);

                // ── Build progress ─────────────────────────────────
                ui.heading("Build Progress");
                ui.add_space(2.0);

                let progress = if total > 0 {
                    built as f32 / total as f32
                } else {
                    0.0
                };

                ui.add(
                    egui::ProgressBar::new(progress)
                        .text(format!("{} / {} frames", built, total))
                        .animate(built < total),
                );

                let build_time = spatial_index.total_build_time_ms();
                if build_time > 0.0 {
                    ui.add_space(2.0);
                    egui::Grid::new("spatial_build_grid")
                        .num_columns(2)
                        .spacing([12.0, 4.0])
                        .show(ui, |ui| {
                            ui.label("Total build:");
                            ui.label(format_time(build_time));
                            ui.end_row();

                            if built > 0 {
                                ui.label("Avg per frame:");
                                ui.label(format_time(build_time / built as f64));
                                ui.end_row();
                            }
                        });
                }

                // ── Current frame stats ────────────────────────────
                if let Some(current) = frame_store.displayed_frame {
                    ui.add_space(6.0);
                    ui.heading("Current Frame");
                    ui.add_space(2.0);

                    if let Some(meshes) = spatial_index.get(current) {
                        let total_tris: usize = meshes.iter().map(|sm| sm.triangle_count()).sum();
                        let total_verts: usize = meshes.iter().map(|sm| sm.vertex_count()).sum();

                        egui::Grid::new("spatial_frame_grid")
                            .num_columns(2)
                            .spacing([12.0, 4.0])
                            .show(ui, |ui| {
                                ui.label("Frame:");
                                ui.label(format!("{}", current));
                                ui.end_row();

                                ui.label("Meshes:");
                                ui.label(format!("{}", meshes.len()));
                                ui.end_row();

                                ui.label("Vertices:");
                                ui.label(format_count(total_verts));
                                ui.end_row();

                                ui.label("Triangles:");
                                ui.label(format_count(total_tris));
                                ui.end_row();
                            });

                        // ── AABB ───────────────────────────────────
                        if let Some(first) = meshes.first() {
                            let (min, max) = first.aabb();
                            let size = [max[0] - min[0], max[1] - min[1], max[2] - min[2]];

                            ui.add_space(4.0);
                            ui.label(egui::RichText::new("Bounding Box").strong());
                            ui.add_space(2.0);

                            egui::Grid::new("spatial_aabb_grid")
                                .num_columns(2)
                                .spacing([12.0, 4.0])
                                .show(ui, |ui| {
                                    ui.label("Min:");
                                    ui.label(format!(
                                        "({:.3}, {:.3}, {:.3})",
                                        min[0], min[1], min[2]
                                    ));
                                    ui.end_row();

                                    ui.label("Max:");
                                    ui.label(format!(
                                        "({:.3}, {:.3}, {:.3})",
                                        max[0], max[1], max[2]
                                    ));
                                    ui.end_row();

                                    ui.label("Size:");
                                    ui.label(format!(
                                        "{:.3} \u{00d7} {:.3} \u{00d7} {:.3}",
                                        size[0], size[1], size[2]
                                    ));
                                    ui.end_row();
                                });
                        }

                        // ── BVH visualization controls ─────────────
                        ui.add_space(6.0);
                        ui.heading("BVH Visualization");
                        ui.add_space(2.0);

                        ui.checkbox(&mut bvh_config.enabled, "Show BVH wireframes")
                            .on_hover_text(
                                "Draw wireframe boxes for the bounding volume hierarchy nodes",
                            );

                        if bvh_config.enabled {
                            ui.add_space(4.0);

                            // Depth slider
                            let max_bvh_depth =
                                meshes.first().map(|sm| sm.bvh_depth()).unwrap_or(10);
                            let slider_max = max_bvh_depth.max(1);
                            ui.add(
                                egui::Slider::new(&mut bvh_config.max_depth, 0..=slider_max)
                                    .text("Depth")
                                    .clamping(egui::SliderClamping::Always),
                            )
                            .on_hover_text(
                                "Maximum BVH tree depth to display (0 = root children only)",
                            );

                            ui.add_space(2.0);

                            // Node type filters
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut bvh_config.show_internal, "Internal");
                                ui.checkbox(&mut bvh_config.show_leaves, "Leaves");
                            });

                            ui.add_space(2.0);

                            // Opacity slider
                            ui.add(
                                egui::Slider::new(&mut bvh_config.opacity, 0.05..=1.0)
                                    .text("Opacity")
                                    .fixed_decimals(2),
                            );

                            // Node count preview
                            if let Some(first) = meshes.first() {
                                let nodes = first.bvh_node_aabbs(bvh_config.max_depth);
                                let shown = nodes
                                    .iter()
                                    .filter(|n| {
                                        (n.is_leaf && bvh_config.show_leaves)
                                            || (!n.is_leaf && bvh_config.show_internal)
                                    })
                                    .count();
                                ui.add_space(2.0);
                                ui.label(
                                    egui::RichText::new(format!(
                                        "{} boxes drawn ({} internal, {} leaves)",
                                        shown,
                                        nodes.iter().filter(|n| !n.is_leaf).count(),
                                        nodes.iter().filter(|n| n.is_leaf).count(),
                                    ))
                                    .small()
                                    .color(egui::Color32::GRAY),
                                );
                            }
                        }
                    } else {
                        ui.colored_label(
                            egui::Color32::GRAY,
                            format!("Frame {} \u{2014} not yet built", current),
                        );
                    }
                }
            }
        });
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Format a large count with K / M suffixes.
fn format_count(n: usize) -> String {
    if n >= 1_000_000 {
        format!("{:.2}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}K", n as f64 / 1_000.0)
    } else {
        format!("{}", n)
    }
}

/// Format a time in milliseconds with appropriate units.
fn format_time(ms: f64) -> String {
    if ms >= 1000.0 {
        format!("{:.2} s", ms / 1000.0)
    } else if ms >= 1.0 {
        format!("{:.1} ms", ms)
    } else {
        format!("{:.0} \u{00b5}s", ms * 1000.0)
    }
}
