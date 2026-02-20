//! Transport controls row
//!
//! Extracted from the monolithic `playback_controls_system`.  Contains the
//! horizontal row of buttons (prev / play-pause / next), camera controls,
//! speed slider, loop & watch checkboxes, and the right-aligned FPS / time
//! display.

use bevy::prelude::*;
use bevy_egui::egui;

use crate::app::systems::camera::CenterOnMeshEvent;
use crate::lib::sequence::{
    LoadingPhase, LoadingPhaseState, SequenceManager, Timeline, WatchConfig,
};
use crate::lib::series::SeriesSummary;
use crate::lib::settings::{CameraSettings, SaveViewEvent, SavedCameras};

/// Actions the transport row can request that must be applied *after* the UI
/// closure finishes (to satisfy borrow-checker constraints on camera queries).
#[derive(Default)]
pub struct TransportActions {
    /// Switch the active camera to the named preset.
    pub switch_to_camera: Option<String>,
    /// Save the current camera transform under a new name.
    pub save_camera_as: Option<String>,
    /// Switch to the previous series.
    pub cycle_series_prev: bool,
    /// Switch to the next series.
    pub cycle_series_next: bool,
}

/// Draw the transport controls row.
///
/// Returns any deferred actions that the caller must apply outside of the egui
/// closure (e.g. camera transform mutations).
#[allow(clippy::too_many_arguments)]
pub fn draw_transport_row(
    ui: &mut egui::Ui,
    timeline: &mut Timeline,
    sequence_manager: &mut SequenceManager,
    watch_config: &mut WatchConfig,
    loading_phase: &mut LoadingPhase,
    center_events: &mut MessageWriter<CenterOnMeshEvent>,
    save_view_events: &mut MessageWriter<SaveViewEvent>,
    saved_cameras: &Option<ResMut<SavedCameras>>,
    series_summaries: &[SeriesSummary],
    display_current_frame: usize,
    display_total_frames: usize,
    display_is_playing: bool,
    delta_secs: f32,
) -> TransportActions {
    let mut actions = TransportActions::default();

    ui.horizontal(|ui| {
        ui.add_space(10.0);

        // -- Prev / Play-Pause / Next -----------------------------------------

        if ui.button("⏮").on_hover_text("Previous frame").clicked() {
            timeline.previous_frame();
        }

        let play_pause_text = if display_is_playing { "⏸" } else { "▶" };
        if ui
            .button(play_pause_text)
            .on_hover_text(if display_is_playing { "Pause" } else { "Play" })
            .clicked()
        {
            sequence_manager.toggle_playback();
        }

        if ui.button("⏭").on_hover_text("Next frame").clicked() {
            timeline.next_frame();
        }

        // -- Series selector (only when multiple series loaded) ---------------

        if series_summaries.len() > 1 {
            ui.separator();

            if ui
                .button("◀")
                .on_hover_text("Previous series")
                .clicked()
            {
                actions.cycle_series_prev = true;
            }

            let active_label = series_summaries
                .iter()
                .find(|s| s.is_active)
                .map(|s| s.label.as_str())
                .unwrap_or("—");
            let active_idx = series_summaries
                .iter()
                .position(|s| s.is_active)
                .map(|i| i + 1)
                .unwrap_or(0);
            ui.label(
                egui::RichText::new(format!(
                    "📂 {} ({}/{})",
                    active_label,
                    active_idx,
                    series_summaries.len()
                ))
                .strong(),
            )
            .on_hover_text("Active series");

            if ui.button("▶").on_hover_text("Next series").clicked() {
                actions.cycle_series_next = true;
            }
        } else if series_summaries.len() == 1 {
            // Single series — just show the name, no cycle buttons
            ui.separator();
            ui.label(
                egui::RichText::new(format!("📂 {}", series_summaries[0].label)).weak(),
            )
            .on_hover_text("Loaded series");
        }

        ui.separator();

        // -- Center camera ----------------------------------------------------

        if ui
            .button("🎯")
            .on_hover_text("Center camera on mesh")
            .clicked()
        {
            debug!("🎯 Center camera button clicked!");
            center_events.write(CenterOnMeshEvent);
            debug!("CenterOnMeshEvent sent");
        }

        // -- Save view --------------------------------------------------------

        if ui
            .button("💾")
            .on_hover_text("Save view (camera + environment) to seaview.toml")
            .clicked()
        {
            debug!("💾 Save view button clicked!");
            save_view_events.write(SaveViewEvent);
        }

        // -- Camera switching dropdown ----------------------------------------

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

        if !camera_names.is_empty() {
            ui.separator();
            ui.label("📷");

            let mut selected = active_camera_name;
            let combo = egui::ComboBox::from_id_salt("camera_selector")
                .selected_text(&selected)
                .width(100.0);

            combo.show_ui(ui, |ui| {
                for name in &camera_names {
                    if ui
                        .selectable_value(&mut selected, name.clone(), name)
                        .clicked()
                    {
                        actions.switch_to_camera = Some(name.clone());
                    }
                }
            });
        }

        // -- "Save camera as…" button + popup ---------------------------------

        let save_as_btn = ui
            .button("📷+")
            .on_hover_text("Save current camera as a named preset");

        egui::Popup::from_toggle_button_response(&save_as_btn)
            .close_behavior(egui::PopupCloseBehavior::CloseOnClickOutside)
            .show(|ui| {
                ui.set_min_width(160.0);
                ui.label("Camera name:");

                let buf_id = ui.id().with("save_as_name_buf");
                let mut name_buf = ui
                    .data_mut(|d| d.get_temp::<String>(buf_id))
                    .unwrap_or_default();

                let response = ui.text_edit_singleline(&mut name_buf);
                ui.data_mut(|d| d.insert_temp(buf_id, name_buf.clone()));

                let enter_pressed =
                    response.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));

                ui.horizontal(|ui| {
                    let popup_id = egui::Popup::default_response_id(&save_as_btn);
                    let ctx = ui.ctx().clone();
                    let save_clicked = ui.button("Save").clicked();
                    if (save_clicked || enter_pressed) && !name_buf.trim().is_empty() {
                        let trimmed = name_buf.trim().to_string();
                        actions.save_camera_as = Some(trimmed);
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

        // -- Frame counter ----------------------------------------------------

        ui.label(format!(
            "Frame: {} / {}",
            display_current_frame, display_total_frames
        ));

        ui.separator();

        // -- Speed slider -----------------------------------------------------

        ui.label("Speed:");
        ui.add(
            egui::Slider::new(&mut sequence_manager.speed, 0.1..=5.0)
                .text("x")
                .fixed_decimals(1)
                .custom_formatter(|n, _| format!("{:.1}x", n))
                .custom_parser(|s| s.trim_end_matches('x').parse::<f64>().ok()),
        );

        ui.separator();

        // -- Loop toggle ------------------------------------------------------

        ui.checkbox(&mut sequence_manager.loop_enabled, "Loop");

        ui.separator();

        // -- Watch toggle -----------------------------------------------------

        let watch_label = if loading_phase.state.is_watching() {
            "👁 Watch"
        } else {
            "Watch"
        };
        let mut watch_enabled = watch_config.enabled;
        if ui
            .checkbox(&mut watch_enabled, watch_label)
            .on_hover_text("Watch directory for new frames (Ctrl+W)")
            .changed()
        {
            watch_config.enabled = watch_enabled;
            if watch_enabled && loading_phase.state == LoadingPhaseState::Idle {
                loading_phase.state = LoadingPhaseState::Watching;
            } else if !watch_enabled && loading_phase.state == LoadingPhaseState::Watching {
                loading_phase.state = LoadingPhaseState::Idle;
            }
        }

        // -- Right-aligned FPS / time -----------------------------------------

        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let fps = 1.0 / delta_secs;
            ui.label(format!("FPS: {:.0}", fps));
            ui.separator();

            if display_total_frames > 0 {
                let current_time = display_current_frame as f32 / 30.0;
                let total_time = display_total_frames as f32 / 30.0;
                ui.label(format!("Time: {:.1}s / {:.1}s", current_time, total_time));
            }
        });
    });

    actions
}

/// Apply deferred camera actions that were collected during the transport row.
///
/// Call this **outside** of the egui panel closure so that `camera_query` is
/// not borrowed at the same time as the UI context.
pub fn apply_camera_actions(
    actions: &TransportActions,
    saved_cameras: &mut Option<ResMut<SavedCameras>>,
    camera_query: &mut Query<
        &mut Transform,
        (With<Camera3d>, With<crate::app::systems::camera::FpsCamera>),
    >,
    save_view_events: &mut MessageWriter<SaveViewEvent>,
) {
    // Switch to a named camera
    if let Some(ref name) = actions.switch_to_camera {
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
    if let Some(ref name) = actions.save_camera_as {
        if let Ok(transform) = camera_query.single() {
            let cam_settings = CameraSettings::from_transform(&transform);
            if let Some(ref mut sc) = saved_cameras {
                sc.cameras.insert(name.clone(), cam_settings);
                sc.active = name.clone();
                debug!("📷+ Saved current camera as '{}'", name);
                save_view_events.write(SaveViewEvent);
            }
        } else {
            warn!("No camera found to save as '{}'", name);
        }
    }
}
