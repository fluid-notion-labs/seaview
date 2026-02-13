//! Playback controls system for Seaview
//!
//! This module implements the bottom panel with timeline and playback controls.
//!
//! **GUTTED** — Commit 1 of playback rebuild. Buttons and sliders render but
//! are inert (no state mutation, no events written). `playback_update_system`
//! has been removed entirely.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::app::systems::camera::CenterOnMeshEvent;
use crate::app::ui::state::UiState;
use crate::lib::sequence::SequenceAssets;
use crate::lib::settings::SaveViewEvent;

/// System that renders the playback controls panel
///
/// Currently inert — UI elements render but clicking buttons does nothing.
/// Will be rebuilt in commit 2 to write directly to `SequenceManager`.
pub fn playback_controls_system(
    mut contexts: EguiContexts,
    ui_state: Res<UiState>,
    mut center_events: MessageWriter<CenterOnMeshEvent>,
    mut save_view_events: MessageWriter<SaveViewEvent>,
    time: Res<Time>,
    sequence_assets: Res<SequenceAssets>,
) {
    if !ui_state.show_playback_controls {
        return;
    }

    let ctx = contexts.ctx_mut().unwrap();

    // NOTE: Sync block removed — no overwriting of ui_state from sequence_manager.
    // NOTE: Speed sync hack removed — no bidirectional sync between SM and ui_state.

    // Update loading state from assets (read-only copy for display)
    let loading_is_loading = sequence_assets.loading;
    let loading_loaded_frames = sequence_assets.loaded_count;
    let loading_total_frames = sequence_assets.total_frames;
    let loading_progress = if loading_total_frames == 0 {
        0.0
    } else {
        loading_loaded_frames as f32 / loading_total_frames as f32
    };

    let panel_height = ui_state.panel_sizes.playback_panel_height;

    egui::TopBottomPanel::bottom("playback_panel")
        .resizable(false)
        .exact_height(panel_height)
        .show(ctx, |ui| {
            // Add 5px margin at the top
            ui.add_space(5.0);

            ui.vertical(|ui| {
                // Loading progress bar (shown when loading)
                if loading_is_loading {
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);

                        let progress_bar = egui::ProgressBar::new(loading_progress)
                            .text(format!(
                                "Loading: {}/{} frames ({:.0}%)",
                                loading_loaded_frames,
                                loading_total_frames,
                                loading_progress * 100.0
                            ))
                            .animate(true);
                        ui.add(progress_bar);

                        ui.add_space(10.0);
                    });
                    ui.add_space(5.0);
                }

                // Playback controls row
                ui.horizontal(|ui| {
                    ui.add_space(10.0);

                    // Previous frame button (INERT)
                    let _prev = ui.button("⏮").on_hover_text("Previous frame (inert)");

                    // Play/Pause button (INERT)
                    let play_pause_text = if ui_state.playback.is_playing {
                        "⏸"
                    } else {
                        "▶"
                    };
                    let _play = ui
                        .button(play_pause_text)
                        .on_hover_text(if ui_state.playback.is_playing {
                            "Pause (inert)"
                        } else {
                            "Play (inert)"
                        });

                    // Next frame button (INERT)
                    let _next = ui.button("⏭").on_hover_text("Next frame (inert)");

                    ui.separator();

                    // Center camera button (still functional — not part of playback)
                    if ui
                        .button("🎯")
                        .on_hover_text("Center camera on mesh")
                        .clicked()
                    {
                        debug!("🎯 Center camera button clicked!");
                        center_events.write(CenterOnMeshEvent);
                        debug!("CenterOnMeshEvent sent");
                    }

                    // Save View button (still functional — not part of playback)
                    if ui
                        .button("💾")
                        .on_hover_text("Save view (camera + playback) to seaview.toml")
                        .clicked()
                    {
                        debug!("💾 Save view button clicked!");
                        save_view_events.write(SaveViewEvent);
                    }

                    ui.separator();

                    // Frame counter (display only)
                    ui.label(format!(
                        "Frame: {} / {}",
                        ui_state.playback.current_frame, ui_state.playback.total_frames
                    ));

                    ui.separator();

                    // Speed control (display only — slider renders but changes are not propagated)
                    ui.label("Speed:");
                    let mut speed_display = ui_state.playback.speed;
                    ui.add(
                        egui::Slider::new(&mut speed_display, 0.1..=5.0)
                            .text("x")
                            .fixed_decimals(1)
                            .custom_formatter(|n, _| format!("{:.1}x", n))
                            .custom_parser(|s| s.trim_end_matches('x').parse::<f64>().ok()),
                    );

                    ui.separator();

                    // Loop toggle (display only — checkbox renders but changes are not propagated)
                    let mut loop_display = ui_state.playback.loop_enabled;
                    ui.checkbox(&mut loop_display, "Loop");

                    // Add some spacing before the right side
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // FPS display
                        let fps = 1.0 / time.delta_secs();
                        ui.label(format!("FPS: {:.0}", fps));
                        ui.separator();

                        // Time display
                        if ui_state.playback.total_frames > 0 {
                            let current_time = ui_state.playback.current_frame as f32 / 30.0;
                            let total_time = ui_state.playback.total_frames as f32 / 30.0;
                            ui.label(format!("Time: {:.1}s / {:.1}s", current_time, total_time));
                        }
                    });
                });

                ui.add_space(5.0);

                // Timeline scrubber (display only — slider renders but changes are not propagated)
                ui.horizontal(|ui| {
                    let total_frames = ui_state.playback.total_frames.saturating_sub(1);
                    let mut frame_display = ui_state.playback.current_frame;
                    ui.add_sized(
                        [ui.available_width(), 20.0],
                        egui::Slider::new(&mut frame_display, 0..=total_frames)
                            .show_value(false),
                    );
                });

                // Progress bar visualization
                ui.horizontal(|ui| {
                    ui.add_space(10.0);

                    let progress = if ui_state.playback.total_frames > 0 {
                        ui_state.playback.current_frame as f32
                            / ui_state.playback.total_frames as f32
                    } else {
                        0.0
                    };

                    let desired_size = egui::Vec2::new(ui.available_width() - 20.0, 10.0);
                    let (rect, _response) =
                        ui.allocate_exact_size(desired_size, egui::Sense::hover());

                    if ui.is_rect_visible(rect) {
                        let painter = ui.painter();

                        // Background
                        painter.rect_filled(rect, 2.0, egui::Color32::from_gray(50));

                        // Progress
                        if progress > 0.0 {
                            let progress_rect = egui::Rect::from_min_size(
                                rect.min,
                                egui::Vec2::new(rect.width() * progress, rect.height()),
                            );
                            painter.rect_filled(
                                progress_rect,
                                2.0,
                                egui::Color32::from_rgb(100, 150, 255),
                            );
                        }

                        // Frame markers (every 10% of the timeline)
                        for i in 1..10 {
                            let marker_x = rect.min.x + (rect.width() * (i as f32 / 10.0));
                            painter.line_segment(
                                [
                                    egui::pos2(marker_x, rect.min.y),
                                    egui::pos2(marker_x, rect.max.y),
                                ],
                                egui::Stroke::new(1.0, egui::Color32::from_gray(80)),
                            );
                        }
                    }
                });
            });
        });
}
