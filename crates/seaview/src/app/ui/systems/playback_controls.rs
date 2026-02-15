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
use crate::app::ui::state::LoadSectionEdge;
use crate::app::ui::state::UiState;
use crate::lib::memory_budget::MemoryBudget;
use crate::lib::sequence::{
    LoadSection, LoadingPhase, LoadingPhaseState, SequenceManager, WatchConfig,
};
use crate::lib::settings::{CameraSettings, SaveViewEvent, SavedCameras};

/// System that renders the playback controls panel.
///
/// Reads `UiState` for display values (frame counter, loading progress, etc.).
/// Writes directly to `SequenceManager` on user interaction (play/pause, step,
/// speed slider, loop checkbox, timeline scrubber).
pub fn playback_controls_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    mut center_events: MessageWriter<CenterOnMeshEvent>,
    mut save_view_events: MessageWriter<SaveViewEvent>,
    time: Res<Time>,
    mut sequence_manager: ResMut<SequenceManager>,
    mut watch_config: ResMut<WatchConfig>,
    mut loading_phase: ResMut<LoadingPhase>,
    mut saved_cameras: Option<ResMut<SavedCameras>>,
    mut camera_query: Query<&mut Transform, (With<Camera3d>, With<FpsCamera>)>,
    mut load_section: ResMut<LoadSection>,
    memory_budget: Res<MemoryBudget>,
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

                // Combined timeline bar with draggable scrubber handle
                ui.horizontal(|ui| {
                    let bar_height = 20.0;
                    let handle_width = 12.0;
                    let handle_height = 24.0;
                    let margin = 5.0;

                    let available_width = ui.available_width() - margin * 2.0;
                    let desired_size = egui::Vec2::new(available_width, handle_height);

                    ui.add_space(margin);
                    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click_and_drag());

                    let max_frame = display_total_frames.saturating_sub(1).max(1);
                    let progress = if display_total_frames > 0 {
                        display_current_frame as f32 / max_frame as f32
                    } else {
                        0.0
                    };

                    // Handle dragging - scrub to frame
                    if response.dragged() || response.clicked() {
                        if let Some(pos) = response.interact_pointer_pos() {
                            let rel_x = ((pos.x - rect.min.x) / rect.width()).clamp(0.0, 1.0);
                            let new_frame = (rel_x * max_frame as f32).round() as usize;
                            sequence_manager.jump_to_frame(new_frame);
                        }
                    }

                    // Change cursor when hovering
                    if response.hovered() {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                    }

                    if ui.is_rect_visible(rect) {
                        let painter = ui.painter();

                        // Bar area (centered vertically)
                        let bar_y = rect.center().y - bar_height / 2.0;
                        let bar_rect = egui::Rect::from_min_size(
                            egui::pos2(rect.min.x, bar_y),
                            egui::Vec2::new(rect.width(), bar_height),
                        );

                        // Background
                        painter.rect_filled(bar_rect, 3.0, egui::Color32::from_gray(45));

                        // Progress fill (blue)
                        if progress > 0.0 {
                            let progress_rect = egui::Rect::from_min_size(
                                bar_rect.min,
                                egui::Vec2::new(bar_rect.width() * progress, bar_height),
                            );
                            painter.rect_filled(
                                progress_rect,
                                3.0,
                                egui::Color32::from_rgb(70, 130, 220),
                            );
                        }

                        // Frame markers (every 10%)
                        for i in 1..10 {
                            let marker_x = bar_rect.min.x + (bar_rect.width() * (i as f32 / 10.0));
                            painter.line_segment(
                                [
                                    egui::pos2(marker_x, bar_rect.min.y),
                                    egui::pos2(marker_x, bar_rect.max.y),
                                ],
                                egui::Stroke::new(1.0, egui::Color32::from_gray(70)),
                            );
                        }

                        // Scrubber handle (vertical bar with rounded ends)
                        let handle_x = rect.min.x + (rect.width() * progress);
                        let handle_rect = egui::Rect::from_center_size(
                            egui::pos2(handle_x, rect.center().y),
                            egui::Vec2::new(handle_width, handle_height),
                        );

                        // Handle shadow
                        let shadow_rect = handle_rect.translate(egui::vec2(1.0, 1.0));
                        painter.rect_filled(shadow_rect, 3.0, egui::Color32::from_black_alpha(60));

                        // Handle body
                        painter.rect_filled(
                            handle_rect,
                            3.0,
                            egui::Color32::from_rgb(255, 200, 80), // Orange/yellow
                        );

                        // Handle border
                        painter.rect_stroke(
                            handle_rect,
                            3.0,
                            egui::Stroke::new(1.5, egui::Color32::from_rgb(200, 150, 50)),
                            egui::StrokeKind::Outside,
                        );

                        // Handle grip lines (3 small horizontal lines)
                        let grip_color = egui::Color32::from_rgb(180, 130, 40);
                        for i in -1..=1 {
                            let y = handle_rect.center().y + (i as f32 * 4.0);
                            painter.line_segment(
                                [
                                    egui::pos2(handle_rect.min.x + 3.0, y),
                                    egui::pos2(handle_rect.max.x - 3.0, y),
                                ],
                                egui::Stroke::new(1.0, grip_color),
                            );
                        }
                    }

                    // Tooltip
                    response.on_hover_text(format!(
                        "Frame {} / {}\nClick or drag to scrub",
                        display_current_frame,
                        display_total_frames.saturating_sub(1)
                    ));
                });

                // Load section bar with draggable edges
                if display_total_frames > 0 {
                    ui.add_space(2.0);
                    ui.horizontal(|ui| {
                        ui.add_space(10.0);

                        let bar_height = 14.0;
                        let handle_width = 8.0;
                        let desired_size = egui::Vec2::new(ui.available_width() - 20.0, bar_height);
                        let (rect, response) =
                            ui.allocate_exact_size(desired_size, egui::Sense::click_and_drag());

                        let total = display_total_frames.max(1) as f32;
                        let section_start = load_section.start as f32 / total;
                        let section_end = load_section.end as f32 / total;

                        // Calculate pixel positions for section edges
                        let start_x = rect.min.x + rect.width() * section_start;
                        let end_x = rect.min.x + rect.width() * section_end;

                        // Handle dragging
                        if response.dragged() {
                            if let Some(pos) = response.interact_pointer_pos() {
                                let rel_x = (pos.x - rect.min.x) / rect.width();
                                let frame = (rel_x * total).round() as usize;
                                let max_frames = memory_budget.max_frames.min(display_total_frames);

                                // Determine which edge to drag based on click position or ongoing drag
                                let near_start = (pos.x - start_x).abs() < handle_width * 1.5;
                                let near_end = (pos.x - end_x).abs() < handle_width * 1.5;

                                match ui_state.load_section_dragging {
                                    Some(LoadSectionEdge::Start) => {
                                        load_section.set_start(frame, max_frames);
                                    }
                                    Some(LoadSectionEdge::End) => {
                                        load_section.set_end(frame, display_total_frames, max_frames);
                                    }
                                    None => {
                                        // Start a new drag
                                        if near_start && !near_end {
                                            ui_state.load_section_dragging = Some(LoadSectionEdge::Start);
                                            load_section.set_start(frame, max_frames);
                                        } else if near_end {
                                            ui_state.load_section_dragging = Some(LoadSectionEdge::End);
                                            load_section.set_end(frame, display_total_frames, max_frames);
                                        }
                                    }
                                }
                            }
                        } else {
                            ui_state.load_section_dragging = None;
                        }

                        // Change cursor when hovering over handles
                        if response.hovered() {
                            if let Some(pos) = ui.ctx().pointer_hover_pos() {
                                let near_start = (pos.x - start_x).abs() < handle_width * 1.5;
                                let near_end = (pos.x - end_x).abs() < handle_width * 1.5;
                                if near_start || near_end {
                                    ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                                }
                            }
                        }

                        if ui.is_rect_visible(rect) {
                            let painter = ui.painter();

                            // Background (unloaded region) - darker
                            painter.rect_filled(rect, 2.0, egui::Color32::from_gray(35));

                            // Loaded region - brighter color
                            if section_end > section_start {
                                let loaded_rect = egui::Rect::from_min_max(
                                    egui::pos2(start_x, rect.min.y),
                                    egui::pos2(end_x, rect.max.y),
                                );
                                painter.rect_filled(
                                    loaded_rect,
                                    2.0,
                                    egui::Color32::from_rgb(60, 120, 80), // Green for loaded
                                );

                                // Left handle
                                let left_handle = egui::Rect::from_min_size(
                                    egui::pos2(start_x - handle_width / 2.0, rect.min.y),
                                    egui::Vec2::new(handle_width, bar_height),
                                );
                                painter.rect_filled(
                                    left_handle,
                                    2.0,
                                    egui::Color32::from_rgb(100, 180, 120),
                                );

                                // Right handle
                                let right_handle = egui::Rect::from_min_size(
                                    egui::pos2(end_x - handle_width / 2.0, rect.min.y),
                                    egui::Vec2::new(handle_width, bar_height),
                                );
                                painter.rect_filled(
                                    right_handle,
                                    2.0,
                                    egui::Color32::from_rgb(100, 180, 120),
                                );
                            }

                            // Border
                            painter.rect_stroke(rect, 2.0, egui::Stroke::new(1.0, egui::Color32::from_gray(60)), egui::StrokeKind::Outside);
                        }

                        // Tooltip with frame range info
                        response.on_hover_text(format!(
                            "Loaded: frames {} - {} ({} frames)\nMax: {} frames\nDrag edges to adjust",
                            load_section.start,
                            load_section.end.saturating_sub(1),
                            load_section.frame_count(),
                            memory_budget.max_frames.min(display_total_frames)
                        ));
                    });
                }
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
