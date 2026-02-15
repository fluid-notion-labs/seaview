//! Playback controls panel
//!
//! Split from the original 575-line monolithic function into focused
//! sub-modules as a prerequisite for multi-section support (seaview-wme).
//!
//! - [`transport`] — prev/play-pause/next, camera, speed, loop, watch, FPS
//! - [`timeline_bar`] — draggable scrubber handle on the timeline
//! - [`load_section_bar`] — load-section ranges with draggable edge handles

mod load_section_bar;
mod timeline_bar;
mod transport;

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::app::systems::camera::{CenterOnMeshEvent, FpsCamera};
use crate::app::ui::state::UiState;
use crate::lib::memory_budget::MemoryBudget;
use crate::lib::sequence::{LoadingPhase, SequenceManager, Timeline, WatchConfig};
use crate::lib::settings::{SaveViewEvent, SavedCameras};

use load_section_bar::LoadSectionDragState;

/// System that renders the playback controls panel.
///
/// Reads `UiState` for display values (frame counter, loading progress, etc.).
/// Writes directly to `SequenceManager` on user interaction (play/pause, step,
/// speed slider, loop checkbox, timeline scrubber).
pub fn playback_controls_system(
    mut contexts: EguiContexts,
    ui_state: ResMut<UiState>,
    mut center_events: MessageWriter<CenterOnMeshEvent>,
    mut save_view_events: MessageWriter<SaveViewEvent>,
    time: Res<Time>,
    mut sequence_manager: ResMut<SequenceManager>,
    mut timeline: ResMut<Timeline>,
    mut watch_config: ResMut<WatchConfig>,
    mut loading_phase: ResMut<LoadingPhase>,
    mut saved_cameras: Option<ResMut<SavedCameras>>,
    mut camera_query: Query<&mut Transform, (With<Camera3d>, With<FpsCamera>)>,
    memory_budget: Res<MemoryBudget>,
    mut section_drag: Local<Option<LoadSectionDragState>>,
) {
    if !ui_state.show_playback_controls {
        return;
    }

    let ctx = contexts.ctx_mut().unwrap();

    // Read display values from UiState (read-only mirror of SequenceManager).
    let display_current_frame = ui_state.playback.current_frame;
    let display_total_frames = ui_state.playback.total_frames;
    let display_is_playing = ui_state.playback.is_playing;

    // Loading state
    let loading_is_loading = ui_state.loading.is_loading;
    let loading_loaded_frames = ui_state.loading.loaded_frames;
    let loading_total_frames = ui_state.loading.total_frames;
    let loading_progress = ui_state.loading.progress();

    let panel_height = ui_state.panel_sizes.playback_panel_height;
    let delta_secs = time.delta_secs();

    let max_loaded_frames = if memory_budget.max_frames < usize::MAX {
        memory_budget.max_frames
    } else {
        display_total_frames
    };

    // Collect transport actions to apply after the panel closure
    let mut transport_actions = transport::TransportActions::default();

    egui::TopBottomPanel::bottom("playback_panel")
        .resizable(false)
        .exact_height(panel_height)
        .show(ctx, |ui| {
            ui.add_space(5.0);

            ui.vertical(|ui| {
                // -- Loading progress bar (fixed height — always present) -----
                // Always allocate the same space whether loading or not so
                // the layout never shifts during section edge drags.
                ui.horizontal(|ui| {
                    ui.add_space(10.0);
                    if loading_is_loading {
                        ui.add(
                            egui::ProgressBar::new(loading_progress)
                                .text(format!(
                                    "{}/{}  ({:.0}%)",
                                    loading_loaded_frames,
                                    loading_total_frames,
                                    loading_progress * 100.0
                                ))
                                .desired_width(ui.available_width() - 20.0),
                        );
                    } else {
                        // Empty bar placeholder — same height, no visual noise
                        ui.add(
                            egui::ProgressBar::new(0.0)
                                .text("")
                                .desired_width(ui.available_width() - 20.0),
                        );
                    }
                    ui.add_space(10.0);
                });

                // -- Transport controls row ----------------------------------
                transport_actions = transport::draw_transport_row(
                    ui,
                    &mut timeline,
                    &mut sequence_manager,
                    &mut watch_config,
                    &mut loading_phase,
                    &mut center_events,
                    &mut save_view_events,
                    &saved_cameras,
                    display_current_frame,
                    display_total_frames,
                    display_is_playing,
                    delta_secs,
                );

                ui.add_space(5.0);

                // -- Timeline scrubber bar ------------------------------------
                ui.horizontal(|ui| {
                    timeline_bar::draw_timeline_bar(
                        ui,
                        &mut timeline,
                        display_current_frame,
                        display_total_frames,
                    );
                });

                // -- Load section bar ----------------------------------------
                load_section_bar::draw_load_section_bar(
                    ui,
                    &mut timeline,
                    display_total_frames,
                    max_loaded_frames,
                    &mut section_drag,
                );
            });
        });

    // -- Apply deferred camera actions outside the egui closure ---------------
    transport::apply_camera_actions(
        &transport_actions,
        &mut saved_cameras,
        &mut camera_query,
        &mut save_view_events,
    );
}
