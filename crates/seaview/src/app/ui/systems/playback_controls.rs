//! Playback controls system for Seaview
//!
//! This module implements the bottom panel with timeline and playback controls.
//!
//! **Rebuilt** — Commit 2 of playback rebuild. UI reads from `UiState` for
//! display (one-way mirror of `SequenceManager`). Buttons, sliders, and
//! checkboxes write directly to `SequenceManager` — no events, no indirection.
//!
//! **Phase 4** — Camera switching UI: dropdown of named cameras near the 💾
//! button, click to switch, "Save as…" to name a new camera preset.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::app::systems::camera::{CenterOnMeshEvent, FpsCamera};
use crate::app::ui::state::UiState;
use crate::lib::sequence::{LoadingPhase, LoadingPhaseState, SequenceManager, WatchConfig};
use crate::lib::settings::{CameraSettings, SaveViewEvent, SavedCameras};

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
    mut watch_config: ResMut<WatchConfig>,
    mut loading_phase: ResMut<LoadingPhase>,
    mut saved_cameras: Option<ResMut<SavedCameras>>,
    mut camera_query: Query<&mut Transform, (With<Camera3d>, With<FpsCamera>)>,
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

    // Collect camera-switch actions to apply after the UI closure
    let mut switch_to_camera: Option<String> = None;
    let mut save_as_name: Option<String> = None;

    // Build a sorted list of camera names for the dropdown
    let camera_names: Vec<String> = saved_cameras
        .as_ref()
        .map(|sc| {
            let mut names: Vec<String> = sc.cameras.keys().cloned().collect();
            names.sort();
            names
        })
        .unwrap_or_default();

    let active_camera_name = saved_cameras
        .as_ref()
        .map(|sc| sc.active.clone())
        .unwrap_or_else(|| "default".to_string());

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
                        .on_hover_text("Save view (camera + environment) to seaview.toml")
                        .clicked()
                    {
                        debug!("💾 Save view button clicked!");
                        save_view_events.write(SaveViewEvent);
                    }

                    // -- Camera switching dropdown ---------------------------------
                    if !camera_names.is_empty() {
                        ui.separator();

                        ui.label("📷");

                        let mut selected = active_camera_name.clone();
                        let combo = egui::ComboBox::from_id_salt("camera_selector")
                            .selected_text(&selected)
                            .width(100.0);

                        combo.show_ui(ui, |ui| {
                            for name in &camera_names {
                                if ui
                                    .selectable_value(&mut selected, name.clone(), name)
                                    .clicked()
                                {
                                    switch_to_camera = Some(name.clone());
                                }
                            }
                        });
                    }

                    // "Save camera as…" button — opens a tiny popup to name the preset
                    let save_as_btn = ui
                        .button("📷+")
                        .on_hover_text("Save current camera as a named preset");

                    // Popup for entering a new camera name (modern Popup API)
                    egui::Popup::from_toggle_button_response(&save_as_btn)
                        .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
                        .show(|ui| {
                            ui.set_min_width(160.0);
                            ui.label("Camera name:");

                            // Use a temporary data store in egui memory for the text input
                            let buf_id = ui.id().with("save_as_name_buf");
                            let mut name_buf = ui
                                .data_mut(|d| d.get_temp::<String>(buf_id))
                                .unwrap_or_default();

                            let response = ui.text_edit_singleline(&mut name_buf);

                            // Store the buffer back
                            ui.data_mut(|d| d.insert_temp(buf_id, name_buf.clone()));

                            let enter_pressed = response.lost_focus()
                                && ui.input(|i| i.key_pressed(egui::Key::Enter));

                            ui.horizontal(|ui| {
                                let popup_id = egui::Popup::default_response_id(&save_as_btn);
                                let ctx = ui.ctx().clone();
                                let save_clicked = ui.button("Save").clicked();
                                if (save_clicked || enter_pressed) && !name_buf.trim().is_empty()
                                {
                                    let trimmed = name_buf.trim().to_string();
                                    save_as_name = Some(trimmed);
                                    // Clear the buffer and close popup
                                    ui.data_mut(|d| d.insert_temp(buf_id, String::new()));
                                    egui::Popup::close_id(&ctx, popup_id);
                                }
                                if ui.button("Cancel").clicked() {
                                    ui.data_mut(|d| d.insert_temp(buf_id, String::new()));
                                    egui::Popup::close_id(&ctx, popup_id);
                                }
                            });
                        });

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

                    ui.separator();

                    // Watch toggle → enables directory watching for new frames (Ctrl+W)
                    let watch_label = if loading_phase.state.is_watching() {
                        "👁 Watch"
                    } else {
                        "Watch"
                    };
                    let mut watch_enabled = watch_config.enabled;
                    if ui.checkbox(&mut watch_enabled, watch_label)
                        .on_hover_text("Watch directory for new frames (Ctrl+W)")
                        .changed()
                    {
                        watch_config.enabled = watch_enabled;
                        // Update loading phase state
                        if watch_enabled && loading_phase.state == LoadingPhaseState::Idle {
                            loading_phase.state = LoadingPhaseState::Watching;
                        } else if !watch_enabled && loading_phase.state == LoadingPhaseState::Watching {
                            loading_phase.state = LoadingPhaseState::Idle;
                        }
                    }

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

    // -- Apply deferred camera actions (outside the UI closure) ----------------

    // Switch to a named camera
    if let Some(ref name) = switch_to_camera {
        if let Some(ref mut sc) = saved_cameras {
            if let Some(cam_settings) = sc.cameras.get(name) {
                let new_transform = cam_settings.to_transform();
                if let Ok(mut transform) = camera_query.single_mut() {
                    *transform = new_transform;
                    sc.active = name.clone();
                    debug!("📷 Switched to camera '{}'", name);
                }
            } else {
                warn!("Camera '{}' not found in saved cameras", name);
            }
        }
    }

    // "Save as…" — snapshot the current camera under a new name
    if let Some(ref name) = save_as_name {
        if let Ok(transform) = camera_query.single() {
            let cam_settings = CameraSettings::from_transform(&transform);
            if let Some(ref mut sc) = saved_cameras {
                sc.cameras.insert(name.clone(), cam_settings);
                sc.active = name.clone();
                debug!("📷+ Saved current camera as '{}'", name);
                // Also trigger a full save so it persists to disk
                save_view_events.write(SaveViewEvent);
            }
        } else {
            warn!("No camera found to save as '{}'", name);
        }
    }
}
