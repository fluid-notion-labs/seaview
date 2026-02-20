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

/// Left margin (px) for the timeline and load-section track bars.
/// Both bars use this so frame positions align vertically.
pub(super) const TRACK_LEFT_MARGIN: f32 = 10.0;

/// Right-side reserve (px) after the track bar — leaves room for the
/// section-count label on the load-section bar.  The timeline bar uses
/// the same value so both bars share identical width.
pub(super) const TRACK_RIGHT_RESERVE: f32 = 54.0;

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::app::systems::camera::{CenterOnMeshEvent, FpsCamera};
use crate::app::ui::state::UiState;
use crate::lib::memory_budget::MemoryBudget;
use crate::lib::sequence::{LoadingPhase, SequenceManager, SeriesManagerRes, Timeline, WatchConfig};
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
    mut series_mgr: ResMut<SeriesManagerRes>,
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

    let panel_height = ui_state.panel_sizes.playback_panel_height;
    let delta_secs = time.delta_secs();

    let max_loaded_frames = if memory_budget.max_frames < usize::MAX {
        memory_budget.max_frames
    } else {
        display_total_frames
    };

    // Snapshot series summaries for the transport row (avoids borrowing
    // series_mgr inside the egui closure while we also need it mutable).
    let series_summaries = series_mgr.summary();

    // Collect transport actions to apply after the panel closure
    let mut transport_actions = transport::TransportActions::default();

    egui::TopBottomPanel::bottom("playback_panel")
        .resizable(false)
        .exact_height(panel_height)
        .show(ctx, |ui| {
            ui.add_space(5.0);

            ui.vertical(|ui| {
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
                    &series_summaries,
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

    // -- Apply deferred series cycle actions ----------------------------------
    if transport_actions.cycle_series_prev {
        if let Some(new_id) = series_mgr.cycle_prev() {
            debug!("◀ Cycled to previous series: {}", new_id);
        }
    }
    if transport_actions.cycle_series_next {
        if let Some(new_id) = series_mgr.cycle_next() {
            debug!("▶ Cycled to next series: {}", new_id);
        }
    }

    // -- Apply deferred camera actions outside the egui closure ---------------
    transport::apply_camera_actions(
        &transport_actions,
        &mut saved_cameras,
        &mut camera_query,
        &mut save_view_events,
    );
}
