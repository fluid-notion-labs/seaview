//! Performance section for the Environment Panel
//!
//! This module renders a collapsible performance section inside the environment
//! panel showing real-time FPS, frame time statistics, rendering stats (vertices,
//! triangles, mesh counts), and entity counts.

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use bevy_egui::egui;

use crate::app::systems::diagnostics::RenderingStats;
use crate::app::ui::state::UiState;
use crate::app::ui::widgets::section_header_tracked;

/// Renders the performance section inside an existing egui `Ui` context.
///
/// This is called from `environment_panel_system` as another collapsible accordion
/// entry so that it lives in the environment panel alongside lighting, material, etc.
pub fn render_perf_section(
    ui: &mut egui::Ui,
    ui_state: &mut UiState,
    diagnostics: &DiagnosticsStore,
    rendering_stats: &RenderingStats,
) {
    if !ui_state.show_perf_panel {
        return;
    }

    section_header_tracked(
        ui,
        "⏱ Performance",
        "perf",
        &mut ui_state.collapsible.perf_open,
        |ui| {
            // ── FPS & Frame Time ───────────────────────────────────
            ui.heading("Frame Rate");
            ui.add_space(2.0);

            let (fps_val, fps_color) = fps_from_diagnostics(diagnostics);
            let frame_time_val = frame_time_from_diagnostics(diagnostics);

            egui::Grid::new("perf_fps_grid")
                .num_columns(2)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    ui.label("FPS:");
                    ui.colored_label(fps_color, format!("{:.1}", fps_val));
                    ui.end_row();

                    ui.label("Frame time:");
                    ui.label(format!("{:.2} ms", frame_time_val));
                    ui.end_row();
                });

            // ── Frame Time Stats (from RenderingStats history) ─────
            let frame_stats = rendering_stats.frame_stats();
            if frame_stats.avg > 0.0 {
                ui.add_space(6.0);
                ui.heading("Frame Time Stats");
                ui.add_space(2.0);

                egui::Grid::new("perf_frametime_grid")
                    .num_columns(2)
                    .spacing([12.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Avg:");
                        ui.label(format!("{:.2} ms", frame_stats.avg));
                        ui.end_row();

                        ui.label("Min:");
                        ui.label(format!("{:.2} ms", frame_stats.min));
                        ui.end_row();

                        ui.label("Max:");
                        ui.label(format!("{:.2} ms", frame_stats.max));
                        ui.end_row();

                        ui.label("P95:");
                        ui.label(format!("{:.2} ms", frame_stats.p95));
                        ui.end_row();

                        ui.label("P99:");
                        ui.label(format!("{:.2} ms", frame_stats.p99));
                        ui.end_row();

                        ui.label("Std dev:");
                        ui.label(format!("{:.2} ms", frame_stats.std_dev));
                        ui.end_row();

                        if frame_stats.spikes > 0 {
                            ui.label("Spikes (>33ms):");
                            ui.colored_label(
                                egui::Color32::from_rgb(255, 160, 60),
                                format!("{}", frame_stats.spikes),
                            );
                            ui.end_row();
                        }
                    });

                // Mini frame-time bar sparkline
                render_frame_time_sparkline(ui, rendering_stats);
            }

            // ── Rendering Stats ────────────────────────────────────
            ui.add_space(6.0);
            ui.heading("Rendering");
            ui.add_space(2.0);

            egui::Grid::new("perf_render_grid")
                .num_columns(2)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Meshes:");
                    ui.label(format!(
                        "{} / {}",
                        rendering_stats.visible_mesh_count, rendering_stats.mesh_count
                    ));
                    ui.end_row();

                    ui.label("Vertices:");
                    ui.label(format_count(rendering_stats.total_vertices));
                    ui.end_row();

                    ui.label("Triangles:");
                    ui.label(format_count(rendering_stats.total_triangles));
                    ui.end_row();

                    ui.label("Largest mesh:");
                    ui.label(format_count(rendering_stats.largest_mesh_vertices));
                    ui.end_row();

                    // Rough GPU memory estimate
                    let bytes_per_vertex: usize = 12 + 12 + 8; // pos + normal + uv
                    let gpu_mb =
                        (rendering_stats.total_vertices * bytes_per_vertex) as f64 / 1_048_576.0;
                    ui.label("Est. GPU mem:");
                    ui.label(format!("{:.1} MB", gpu_mb));
                    ui.end_row();
                });

            // ── Warnings ───────────────────────────────────────────
            let issues = collect_issues(rendering_stats, &frame_stats);
            if !issues.is_empty() {
                ui.add_space(6.0);
                ui.heading("⚠ Warnings");
                ui.add_space(2.0);
                for issue in &issues {
                    ui.colored_label(egui::Color32::from_rgb(255, 200, 80), issue);
                }
            }
        },
    );
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Read smoothed FPS from the diagnostics store and return (value, colour).
fn fps_from_diagnostics(diagnostics: &DiagnosticsStore) -> (f64, egui::Color32) {
    let fps = diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FPS)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0);

    let color = if fps >= 55.0 {
        egui::Color32::from_rgb(100, 255, 100) // green
    } else if fps >= 30.0 {
        egui::Color32::from_rgb(255, 255, 100) // yellow
    } else {
        egui::Color32::from_rgb(255, 100, 100) // red
    };

    (fps, color)
}

/// Read smoothed frame time (ms) from the diagnostics store.
fn frame_time_from_diagnostics(diagnostics: &DiagnosticsStore) -> f64 {
    diagnostics
        .get(&FrameTimeDiagnosticsPlugin::FRAME_TIME)
        .and_then(|d| d.smoothed())
        .unwrap_or(0.0)
}

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

/// Draw a tiny sparkline of recent frame times using egui painter.
fn render_frame_time_sparkline(ui: &mut egui::Ui, stats: &RenderingStats) {
    let history = &stats.frame_time_history;
    if history.len() < 2 {
        return;
    }

    // Take the last N samples that fit the available width
    let max_samples = 120;
    let samples: &[f32] = if history.len() > max_samples {
        &history[history.len() - max_samples..]
    } else {
        history
    };

    let desired_width = ui.available_width().min(280.0);
    let desired_height: f32 = 40.0;

    let (rect, _response) = ui.allocate_exact_size(
        egui::vec2(desired_width, desired_height),
        egui::Sense::hover(),
    );

    if !ui.is_rect_visible(rect) {
        return;
    }

    let painter = ui.painter_at(rect);

    // Background
    painter.rect_filled(rect, 2.0, egui::Color32::from_gray(30));

    // Determine y-axis range: 0 .. max(33.33, actual_max)
    let max_val = samples.iter().cloned().fold(33.33_f32, f32::max).max(1.0);

    let n = samples.len();
    let bar_width = rect.width() / n as f32;

    for (i, &val) in samples.iter().enumerate() {
        let t = (val / max_val).clamp(0.0, 1.0);
        let x = rect.left() + i as f32 * bar_width;
        let bar_height = t * rect.height();
        let y_top = rect.bottom() - bar_height;

        let color = if val <= 16.67 {
            egui::Color32::from_rgb(80, 200, 80) // ≥60 fps green
        } else if val <= 33.33 {
            egui::Color32::from_rgb(220, 220, 60) // 30-60 fps yellow
        } else {
            egui::Color32::from_rgb(220, 70, 70) // <30 fps red
        };

        painter.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(x, y_top),
                egui::pos2(x + bar_width - 0.5, rect.bottom()),
            ),
            0.0,
            color,
        );
    }

    // 60 fps reference line (16.67 ms)
    let y_60 = rect.bottom() - (16.67 / max_val).clamp(0.0, 1.0) * rect.height();
    painter.line_segment(
        [
            egui::pos2(rect.left(), y_60),
            egui::pos2(rect.right(), y_60),
        ],
        egui::Stroke::new(
            1.0,
            egui::Color32::from_rgba_premultiplied(100, 255, 100, 100),
        ),
    );

    // 30 fps reference line (33.33 ms)
    let y_30 = rect.bottom() - (33.33 / max_val).clamp(0.0, 1.0) * rect.height();
    if (y_30 - y_60).abs() > 4.0 {
        painter.line_segment(
            [
                egui::pos2(rect.left(), y_30),
                egui::pos2(rect.right(), y_30),
            ],
            egui::Stroke::new(
                1.0,
                egui::Color32::from_rgba_premultiplied(255, 100, 100, 100),
            ),
        );
    }
}

/// Collect human-readable perf warnings.
fn collect_issues(
    stats: &RenderingStats,
    frame_stats: &crate::app::systems::diagnostics::FrameStats,
) -> Vec<String> {
    let mut issues = Vec::new();

    if frame_stats.avg > 20.0 {
        issues.push(format!(
            "Avg frame time {:.1}ms ({:.0} FPS)",
            frame_stats.avg,
            1000.0 / frame_stats.avg
        ));
    }

    if frame_stats.std_dev > 10.0 {
        issues.push(format!("Unstable: ±{:.1}ms std dev", frame_stats.std_dev));
    }

    if stats.total_vertices > 10_000_000 {
        issues.push(format!(
            "High vertex count: {}",
            format_count(stats.total_vertices)
        ));
    }

    if stats.largest_mesh_vertices > 5_000_000 {
        issues.push(format!(
            "Large single mesh: {}",
            format_count(stats.largest_mesh_vertices)
        ));
    }

    issues
}
