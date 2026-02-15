//! Toast event bridge
//!
//! Thin app-layer system that:
//! 1. Updates `UiState.toast_clock` from `Time` each frame.
//! 2. Detects `LoadingPhase` state transitions and fires appropriate toasts.
//!
//! All logic is glue — no business logic lives here.

use bevy::prelude::*;

use crate::app::ui::state::UiState;
use crate::lib::sequence::{LoadingPhase, LoadingPhaseState, WatchConfig};

/// Tracks the previous `LoadingPhaseState` so we can detect transitions.
#[derive(Resource, Default)]
pub struct PreviousLoadingPhaseState {
    pub state: LoadingPhaseState,
    /// Whether assets were loading on the previous frame.
    pub was_loading: bool,
    /// Loaded frame count on the previous frame (for progress toasts).
    pub prev_loaded: usize,
}

/// System that keeps the toast clock in sync with Bevy's `Time` and
/// detects `LoadingPhase` transitions to fire informational toasts.
///
/// Also watches `UiState.loading` for section-level load start/complete
/// transitions (e.g. when the user drags a section to a new position and
/// `sync_section_loads` kicks off new asset loads).
///
/// Registered in `UiSystemsPlugin` as an `Update` system (not an egui pass).
pub fn toast_events_system(
    time: Res<Time>,
    mut ui_state: ResMut<UiState>,
    loading_phase: Res<LoadingPhase>,
    watch_config: Res<WatchConfig>,
    mut prev_phase: ResMut<PreviousLoadingPhaseState>,
) {
    // ── 1. Tick the toast clock ────────────────────────────────────
    ui_state.update_toast_clock(time.elapsed_secs_f64());

    let now = time.elapsed_secs_f64();

    // ── 2. Detect asset loading transitions (section sync) ─────────
    let is_loading = ui_state.loading.is_loading;
    let loaded = ui_state.loading.loaded_frames;

    if is_loading && !prev_phase.was_loading {
        // Loading just started (section moved / new section added)
        let total = ui_state.loading.total_frames;
        ui_state
            .toasts
            .info(format!("Loading frames… ({}/{})", loaded, total), now);
    } else if !is_loading && prev_phase.was_loading && loaded > 0 {
        // Loading just finished
        ui_state
            .toasts
            .success(format!("✓ {} frames loaded", loaded), now);
    }

    prev_phase.was_loading = is_loading;
    prev_phase.prev_loaded = loaded;

    // ── 3. Detect phase transitions ────────────────────────────────
    let current = loading_phase.state;
    let previous = prev_phase.state;

    if current == previous {
        return;
    }

    match (previous, current) {
        // Initial discovery started
        (_, LoadingPhaseState::InitialDiscovery) => {
            ui_state.toasts.info("Discovering sequences…", now);
        }

        // Initial load started
        (LoadingPhaseState::InitialDiscovery, LoadingPhaseState::InitialLoading) => {
            ui_state.toasts.info("Loading sequence…", now);
        }

        // Initial load → watching
        (LoadingPhaseState::InitialLoading, LoadingPhaseState::Watching) => {
            ui_state
                .toasts
                .success("Sequence loaded — watching for new frames", now);
        }

        // Initial load → idle (watch not enabled)
        (LoadingPhaseState::InitialLoading, LoadingPhaseState::Idle) => {
            ui_state.toasts.success("Sequence loaded", now);
        }

        // Incremental load started
        (LoadingPhaseState::Watching, LoadingPhaseState::IncrementalLoad) => {
            let count = loading_phase.pending_frame_count;
            let msg = if count == 1 {
                "Loading 1 new frame…".to_string()
            } else {
                format!("Loading {count} new frames…")
            };
            ui_state.toasts.info(msg, now);
        }

        // Incremental load complete → back to watching
        (LoadingPhaseState::IncrementalLoad, LoadingPhaseState::Watching) => {
            ui_state.toasts.success("New frames loaded", now);
        }

        // Incremental load complete → idle (watch disabled mid-load)
        (LoadingPhaseState::IncrementalLoad, LoadingPhaseState::Idle) => {
            ui_state.toasts.success("New frames loaded", now);
        }

        // Watch mode toggled on from idle
        (LoadingPhaseState::Idle, LoadingPhaseState::Watching) => {
            if watch_config.enabled {
                ui_state.toasts.info("Watch mode enabled", now);
            }
        }

        // Watch mode toggled off
        (LoadingPhaseState::Watching, LoadingPhaseState::Idle) => {
            ui_state.toasts.info("Watch mode disabled", now);
        }

        // Fallback for any other transition we haven't thought of
        _ => {
            debug!("Loading phase transition: {:?} → {:?}", previous, current);
        }
    }

    prev_phase.state = current;
}
