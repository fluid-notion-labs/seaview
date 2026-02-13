//! Playback controls system for Seaview
//!
//! This module implements the bottom panel with timeline and playback controls.
//!
//! **Rebuilt** — Commit 2 of playback rebuild. UI reads from `UiState` for
//! display (one-way mirror of `SequenceManager`). Buttons, sliders, and
//! checkboxes write directly to `SequenceManager` — no events, no indirection.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::app::systems::camera::CenterOnMeshEvent;
use crate::app::ui::state::UiState;
use crate::lib::sequence::SequenceManager;
use crate::lib::settings::SaveViewEvent;

/// System that renders the playback controls panel.
///
/// Reads `UiState` for display values (frame counter, loading progress, etc.).
/// Writes directly to `SequenceManager` on user interaction (play/pause, step,
/// speed slider, loop checkbox, timeline scrubber).
pub fn playback_controls_system(
    mut contexts: EguiContexts,
    ui_state: Res<UiState>,
    mut center_events: MessageWriter<CenterOnMeshEvent>,
    mut save_view_events: MessageWriter<SaveViewEvent>,
    time: Res<Time>,
    mut sequence_manager: ResMut<SequenceManager>,
) {
    if !ui_state.show_playback_controls {
        return;
    }

    let ctx = contexts.ctx_mut().unwrap();

    // Read display values from UiState (read-only mirror of SequenceManager).
    // These were synced by `sync_ui_from_sequence_manager` earlier this tick.
    let display_current_frame = ui_state.playback.current_frame;
    let display_total_frames = ui_state.playback.total_frames;
    let display_is_playing = ui_state.playback.is_playing;

    // Loading state (also synced via sync_ui_from_sequence_manager)
    let loading_is_loading = ui_state.loading.is_loading;
    let loading_loaded_frames = ui_state.loading.loaded_frames;
    let loading_total_frames = ui_state.loading.total_frames;
    let loading_progress = ui_state.loading.progress();

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

                    // Previous frame button → writes directly to SequenceManager
                    if ui.button("⏮").on_hover_text("Previous frame").clicked() {
                        sequence_manager.previous_frame();
                    }

                    // Play/Pause button → writes directly to SequenceManager
                    let play_pause_text = if display_is_playing {
                        "⏸"
                    } else {
                        "▶"
                    };
                    if ui
                        .button(play_pause_text)
                        .on_hover_text(if display_is_playing {
                            "Pause"
                        } else {
                            "Play"
                        })
                        .clicked()
                    {
                        sequence_manager.toggle_playback();
                    }

                    // Next frame button → writes directly to SequenceManager
                    if ui.button("⏭").on_hover_text("Next frame").clicked() {
                        sequence_manager.next_frame();
                    }

                    ui.separator();

                    // Center camera button (not part of playback — still uses events)
                    if ui
                        .button("🎯")
                        .on_hover_text("Center camera on mesh")
                        .clicked()
                    {
                        debug!("🎯 Center camera button clicked!");
                        center_events.write(CenterOnMeshEvent);
                        debug!("CenterOnMeshEvent sent");
                    }

                    // Save View button (not part of playback — still uses events)
                    if ui
                        .button("💾")
                        .on_hover_text("Save view (camera + playback) to seaview.toml")
                        .clicked()
                    {
                        debug!("💾 Save view button clicked!");
                        save_view_events.write(SaveViewEvent);
                    }

                    ui.separator();

                    // Frame counter (display from UiState mirror)
                    ui.label(format!(
                        "Frame: {} / {}",
                        display_current_frame, display_total_frames
                    ));

                    ui.separator();

                    // Speed slider → bound directly to SequenceManager.speed
                    ui.label("Speed:");
                    ui.add(
                        egui::Slider::new(&mut sequence_manager.speed, 0.1..=5.0)
                            .text("x")
                            .fixed_decimals(1)
                            .custom_formatter(|n, _| format!("{:.1}x", n))
                            .custom_parser(|s| s.trim_end_matches('x').parse::<f64>().ok()),
                    );

                    ui.separator();

                    // Loop toggle → bound directly to SequenceManager.loop_enabled
                    ui.checkbox(&mut sequence_manager.loop_enabled, "Loop");

                    // Add some spacing before the right side
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // FPS display
                        let fps = 1.0 / time.delta_secs();
                        ui.label(format!("FPS: {:.0}", fps));
                        ui.separator();

                        // Time display
                        if display_total_frames > 0 {
                            let current_time = display_current_frame as f32 / 30.0;
                            let total_time = display_total_frames as f32 / 30.0;
                            ui.label(format!("Time: {:.1}s / {:.1}s", current_time, total_time));
                        }
                    });
                });

                ui.add_space(5.0);

                // Timeline scrubber (full width) → writes to SequenceManager on change
                ui.horizontal(|ui| {
                    let total_frames = display_total_frames.saturating_sub(1);
                    let mut frame = sequence_manager.current_frame;
                    let timeline_response = ui.add_sized(
                        [ui.available_width(), 20.0],
                        egui::Slider::new(&mut frame, 0..=total_frames)
                            .show_value(false),
                    );

                    if timeline_response.changed() {
                        sequence_manager.jump_to_frame(frame);
                    }
                });

                // Progress bar visualization
                ui.horizontal(|ui| {
                    ui.add_space(10.0);

                    let progress = if display_total_frames > 0 {
                        display_current_frame as f32 / display_total_frames as f32
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
