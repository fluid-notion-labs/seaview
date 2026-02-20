//! Rendering section for the Environment Panel
//!
//! This module renders the rendering controls (wireframe overlay, frame window
//! size, spawned entity stats, and screen-space reflections) inside the
//! environment panel's accordion UI.

use bevy_egui::egui;

use crate::app::ui::state::{RenderConfig, SsrConfig, UiState};
use crate::app::ui::widgets::section_header_tracked;
use crate::lib::sequence::FrameStore;

/// Renders the rendering section inside an existing egui `Ui` context.
///
/// Wrapped in its own collapsible header since it manages its own
/// open/closed state via `UiState::collapsible`.
pub fn render_rendering_section(
    ui: &mut egui::Ui,
    ui_state: &mut UiState,
    render_config: &mut RenderConfig,
    frame_store: &FrameStore,
    ssr_config: &mut SsrConfig,
) {
    section_header_tracked(
        ui,
        "\u{1f5a5} Rendering",
        "rendering",
        &mut ui_state.collapsible.rendering_open,
        |ui| {
            // Wireframe overlay toggle
            ui.heading("Wireframe");
            ui.add_space(2.0);
            ui.checkbox(
                &mut render_config.wireframe_enabled,
                "Show wireframe overlay",
            )
            .on_hover_text("Overlay triangle edges on the mesh (Ctrl+Shift+W)");
            ui.add_space(8.0);

            // ── Screen-Space Reflections ───────────────────────────────
            ui.heading("Screen-Space Reflections");
            ui.add_space(2.0);

            ui.checkbox(&mut ssr_config.enabled, "Enable SSR")
                .on_hover_text(
                    "Trace reflections in screen space. \
                     Requires deferred rendering. Best on smooth surfaces \
                     (water, glass, chrome).",
                );

            if ssr_config.enabled {
                ui.add_space(4.0);

                ui.add(
                    egui::Slider::new(
                        &mut ssr_config.perceptual_roughness_threshold,
                        0.0..=1.0,
                    )
                    .text("Roughness threshold")
                    .fixed_decimals(2),
                )
                .on_hover_text(
                    "Maximum roughness that receives reflections. \
                     0.0 = only perfect mirrors, 1.0 = everything reflects.",
                );

                ui.add(
                    egui::Slider::new(&mut ssr_config.thickness, 0.01..=10.0)
                        .text("Thickness (m)")
                        .logarithmic(true)
                        .fixed_decimals(2),
                )
                .on_hover_text(
                    "Assumed surface thickness for depth comparison. \
                     Thicker = more forgiving but more artefacts on thin geometry.",
                );

                ui.add(
                    egui::Slider::new(&mut ssr_config.linear_steps, 1..=128)
                        .text("Linear steps")
                        .logarithmic(true),
                )
                .on_hover_text(
                    "Number of ray-march steps. Higher = better quality, \
                     more GPU cost. 16–64 is a good range.",
                );

                ui.add(
                    egui::Slider::new(&mut ssr_config.linear_march_exponent, 1.0..=4.0)
                        .text("March exponent")
                        .fixed_decimals(1),
                )
                .on_hover_text(
                    "Step distribution exponent. 1.0 = uniform spacing. \
                     Higher values put more samples near the surface.",
                );

                ui.add(
                    egui::Slider::new(&mut ssr_config.bisection_steps, 0..=16)
                        .text("Bisection steps"),
                )
                .on_hover_text(
                    "Binary-search refinement after the linear march finds a hit. \
                     More steps = sharper reflections at grazing angles.",
                );

                ui.checkbox(&mut ssr_config.use_secant, "Secant refinement")
                    .on_hover_text(
                        "Use the secant method for sub-step intersection accuracy. \
                         Slightly more GPU work for sharper reflections.",
                    );

                ui.add_space(2.0);

                // Reset to defaults button
                if ui.small_button("Reset SSR defaults").clicked() {
                    let defaults = SsrConfig::default();
                    ssr_config.perceptual_roughness_threshold =
                        defaults.perceptual_roughness_threshold;
                    ssr_config.thickness = defaults.thickness;
                    ssr_config.linear_steps = defaults.linear_steps;
                    ssr_config.linear_march_exponent = defaults.linear_march_exponent;
                    ssr_config.bisection_steps = defaults.bisection_steps;
                    ssr_config.use_secant = defaults.use_secant;
                }
            } else {
                ui.label(
                    egui::RichText::new("SSR disabled — no GPU cost")
                        .small()
                        .color(egui::Color32::from_gray(120)),
                );
            }

            ui.add_space(8.0);

            // ── Frame Window ──────────────────────────────────────────
            ui.heading("Frame Window");
            ui.add_space(2.0);
            ui.label("Spawned entities around current frame:");
            ui.add_space(2.0);

            let max_window = frame_store.total_frames.max(1);
            ui.add(
                egui::Slider::new(
                    &mut render_config.frame_window_size,
                    1..=max_window.min(100),
                )
                .text("frames")
                .logarithmic(true),
            );

            ui.add_space(4.0);

            // Show current window stats
            let spawned = frame_store
                .frame_entities
                .iter()
                .filter(|e| e.is_some())
                .count();
            let total = frame_store.total_frames;

            egui::Grid::new("render_window_grid")
                .num_columns(2)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Spawned:");
                    ui.label(format!("{} / {}", spawned, total));
                    ui.end_row();

                    ui.label("Displayed:");
                    ui.label(
                        frame_store
                            .displayed_frame
                            .map(|f| format!("Frame {}", f))
                            .unwrap_or_else(|| "\u{2014}".to_string()),
                    );
                    ui.end_row();

                    if total > 0 {
                        let current = frame_store.displayed_frame.unwrap_or(0);
                        let (lo, hi) =
                            frame_store.window_range(current, render_config.frame_window_size);
                        ui.label("Window:");
                        ui.label(format!("[{}, {})", lo, hi));
                        ui.end_row();
                    }
                });
        },
    );
}
