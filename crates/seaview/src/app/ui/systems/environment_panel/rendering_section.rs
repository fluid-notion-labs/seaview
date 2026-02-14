//! Rendering section for the Environment Panel
//!
//! This module renders the rendering controls (frame window size, spawned
//! entity stats) inside the environment panel's accordion UI.

use bevy_egui::egui;

use crate::app::ui::state::{RenderConfig, UiState};
use crate::lib::sequence::SequenceAssets;

/// Renders the rendering section inside an existing egui `Ui` context.
///
/// Wrapped in its own collapsible header since it manages its own
/// open/closed state via `UiState::collapsible`.
pub fn render_rendering_section(
    ui: &mut egui::Ui,
    ui_state: &mut UiState,
    render_config: &mut RenderConfig,
    sequence_assets: &SequenceAssets,
) {
    egui::CollapsingHeader::new("\u{1f5a5} Rendering")
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
                            .unwrap_or_else(|| "\u{2014}".to_string()),
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
