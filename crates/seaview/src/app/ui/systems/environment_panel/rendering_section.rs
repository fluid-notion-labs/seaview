//! Rendering section for the Environment Panel
//!
//! This module renders the rendering controls (wireframe overlay, frame window
//! size, spawned entity stats) inside the environment panel's accordion UI.

use bevy_egui::egui;

use crate::app::ui::state::{RenderConfig, UiState};
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

            // Frame window size slider
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
