//! Playback controls system for Seaview
//!
//! This module implements the bottom panel with timeline and playback controls.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::app::systems::camera::CenterOnMeshEvent;
use crate::app::ui::state::UiState;
use crate::lib::sequence::{SequenceAssets, SequenceEvent, SequenceManager};
use crate::lib::settings::SaveViewEvent;

/// System that renders the playback controls panel
pub fn playback_controls_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    mut center_events: MessageWriter<CenterOnMeshEvent>,
    mut sequence_events: MessageWriter<SequenceEvent>,
    mut save_view_events: MessageWriter<SaveViewEvent>,
    time: Res<Time>,
    mut sequence_manager: ResMut<SequenceManager>,
    sequence_assets: Res<SequenceAssets>,
) {
    if !ui_state.show_playback_controls {
        return;
    }

    let ctx = contexts.ctx_mut().unwrap();

    // Sync UI state with SequenceManager
    if let Some(sequence) = sequence_manager.current_sequence() {
        ui_state.playback.total_frames = sequence.frame_count();
        ui_state.playback.current_frame = sequence_manager.current_frame;
        ui_state.playback.is_playing = sequence_manager.is_playing;

        // Sync speed: keyboard +/- changes sequence_manager.playback_fps,
        // UI slider changes ui_state.playback.speed.  Keep them in sync
        // by treating sequence_manager as the source of truth for keyboard
        // changes and ui_state as the source of truth for slider changes.
        // We detect which one changed by comparing.
        let sm_speed = sequence_manager.playback_fps / 30.0; // normalise to multiplier
        if (sm_speed - ui_state.playback.speed).abs() > 0.001 {
            // sequence_manager was changed (keyboard +/-), propagate to UI
            ui_state.playback.speed = sm_speed;
        } else {
            // UI slider is authoritative, propagate to sequence_manager
            sequence_manager.playback_fps = ui_state.playback.speed * 30.0;
        }
    }

    // Update loading state from assets
    ui_state.loading.update_from_assets(&sequence_assets);

    let panel_height = ui_state.panel_sizes.playback_panel_height;

    egui::TopBottomPanel::bottom("playback_panel")
        .resizable(false)
        .exact_height(panel_height)
        .show(ctx, |ui| {
            // Add 5px margin at the top
            ui.add_space(5.0);

            ui.vertical(|ui| {
                // Loading progress bar (shown when loading)
                if ui_state.loading.is_loading {
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);

                        let progress = ui_state.loading.progress();
                        let progress_bar = egui::ProgressBar::new(progress)
                            .text(format!(
                                "Loading: {}/{} frames ({:.0}%)",
                                ui_state.loading.loaded_frames,
                                ui_state.loading.total_frames,
                                progress * 100.0
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

                    // Previous frame button
                    if ui.button("⏮").on_hover_text("Previous frame").clicked() {
                        ui_state.previous_frame();
                        sequence_events.write(SequenceEvent::FrameChanged(ui_state.playback.current_frame));
                    }

                    // Play/Pause button
                    let play_pause_text = if ui_state.playback.is_playing {
                        "⏸"
                    } else {
                        "▶"
                    };
                    if ui
                        .button(play_pause_text)
                        .on_hover_text(if ui_state.playback.is_playing {
                            "Pause"
                        } else {
                            "Play"
                        })
                        .clicked()
                    {
                        ui_state.toggle_playback();
                        if ui_state.playback.is_playing {
                            sequence_events.write(SequenceEvent::PlaybackStarted);
                        } else {
                            sequence_events.write(SequenceEvent::PlaybackStopped);
                        }
                    }

                    // Next frame button
                    if ui.button("⏭").on_hover_text("Next frame").clicked() {
                        ui_state.next_frame();
                        sequence_events.write(SequenceEvent::FrameChanged(ui_state.playback.current_frame));
                    }

                    ui.separator();

                    // Center camera button
                    if ui
                        .button("🎯")
                        .on_hover_text("Center camera on mesh")
                        .clicked()
                    {
                        debug!("🎯 Center camera button clicked!");
                        center_events.write(CenterOnMeshEvent);
                        debug!("CenterOnMeshEvent sent");
                    }

                    // Save View button
                    if ui
                        .button("💾")
                        .on_hover_text("Save view (camera + playback) to seaview.toml")
                        .clicked()
                    {
                        debug!("💾 Save view button clicked!");
                        save_view_events.write(SaveViewEvent);
                    }

                    ui.separator();

                    // Frame counter
                    ui.label(format!(
                        "Frame: {} / {}",
                        ui_state.playback.current_frame, ui_state.playback.total_frames
                    ));

                    ui.separator();

                    // Speed control
                    ui.label("Speed:");
                    ui.add(
                        egui::Slider::new(&mut ui_state.playback.speed, 0.1..=5.0)
                            .text("x")
                            .fixed_decimals(1)
                            .custom_formatter(|n, _| format!("{:.1}x", n))
                            .custom_parser(|s| s.trim_end_matches('x').parse::<f64>().ok()),
                    );

                    ui.separator();

                    // Loop toggle
                    ui.checkbox(&mut ui_state.playback.loop_enabled, "Loop");

                    // Add some spacing before the right side
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // FPS display
                        let fps = 1.0 / time.delta_secs();
                        ui.label(format!("FPS: {:.0}", fps));
                        ui.separator();

                        // Time display
                        if ui_state.playback.total_frames > 0 {
                            let current_time = ui_state.playback.current_frame as f32 / 30.0; // Assuming 30 fps
                            let total_time = ui_state.playback.total_frames as f32 / 30.0;
                            ui.label(format!("Time: {:.1}s / {:.1}s", current_time, total_time));
                        }
                    });
                });

                ui.add_space(5.0);

                // Timeline scrubber (full width)
                ui.horizontal(|ui| {
                    let total_frames = ui_state.playback.total_frames.saturating_sub(1);
                    let timeline_response = ui.add_sized(
                        [ui.available_width(), 20.0],
                        egui::Slider::new(&mut ui_state.playback.current_frame, 0..=total_frames)
                            .show_value(false),
                    );

                    if timeline_response.changed() {
                        let current_frame = ui_state.playback.current_frame;
                        ui_state.seek_to_frame(current_frame);
                        sequence_events.write(SequenceEvent::FrameChanged(current_frame));
                    }
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

/// System that handles automatic playback advancement
pub fn playback_update_system(
    mut ui_state: ResMut<UiState>,
    mut sequence_events: MessageWriter<SequenceEvent>,
    time: Res<Time>,
    mut last_update: Local<f32>,
) {
    if !ui_state.playback.is_playing || ui_state.playback.total_frames == 0 {
        return;
    }

    // Calculate frame duration based on speed
    let frame_duration = 1.0 / (30.0 * ui_state.playback.speed); // 30 FPS base rate

    *last_update += time.delta_secs();

    if *last_update >= frame_duration {
        *last_update -= frame_duration;
        ui_state.next_frame();
        sequence_events.write(SequenceEvent::FrameChanged(ui_state.playback.current_frame));

        // Stop playback if we reached the end and loop is disabled
        if !ui_state.playback.is_playing {
            sequence_events.write(SequenceEvent::PlaybackStopped);
        }
    }
}
