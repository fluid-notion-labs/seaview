//! App-layer autosave system.
//!
//! Watches settings-relevant Bevy resources for changes, marks the autosave
//! state dirty on any mutation, and triggers a debounced save with history
//! rotation.
//!
//! ## Design
//!
//! - **Change detection** runs every frame in `PostUpdate` (after UI and
//!   gameplay systems have finished mutating resources).
//! - **Debounce** is handled by [`AutosaveState`] in `lib/autosave.rs` —
//!   pure logic, no Bevy dependencies.
//! - **History rotation** keeps the last N versions of `seaview.toml` so the
//!   user can recover from bad saves.
//! - **"Save now"** button in the transport row calls
//!   [`AutosaveState::save_now`] to skip the debounce timer.
//!
//! ## Scheduling
//!
//! The autosave system runs in `PostUpdate` after `EguiPostUpdateSet::EndPass`
//! so it sees the final state of all resources for the frame, including any
//! mutations from the egui UI pass.

use bevy::prelude::*;
use log::{debug, trace};

use bevy::ecs::system::SystemParam;

use crate::app::systems::camera::FpsCamera;
use crate::app::ui::state::{MaterialConfig, SsrConfig, UiState};
use crate::lib::autosave::{rotate_history, AutosaveAction, AutosaveConfig, AutosaveState};
use crate::lib::fade::FadeEffect;
use crate::lib::lighting::{EnvironmentLightConfig, LightingRig};
use crate::lib::sequence::{SeriesManagerRes, Timeline, WatchConfig};
use crate::lib::settings::{SavedCameras, SettingsResource, SETTINGS_FILENAME};

use crate::app::environment_objects::EnvironmentObjectStoreRes;
use crate::lib::coordinates::SourceOrientation;
use crate::lib::sequence::SequenceManager;

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

/// Bevy resource wrapping the pure [`AutosaveState`].
#[derive(Resource)]
pub struct AutosaveRes {
    pub state: AutosaveState,
}

impl Default for AutosaveRes {
    fn default() -> Self {
        Self {
            state: AutosaveState::with_defaults(),
        }
    }
}

impl AutosaveRes {
    pub fn new(config: AutosaveConfig) -> Self {
        Self {
            state: AutosaveState::new(config),
        }
    }
}

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// Registers the autosave change-detection and save systems.
pub struct AutosavePlugin;

impl Plugin for AutosavePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<AutosaveRes>().add_systems(
            PostUpdate,
            autosave_system.after(bevy_egui::EguiPostUpdateSet::EndPass),
        );
    }
}

// ---------------------------------------------------------------------------
// System
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// SystemParam bundles (to stay under Bevy's 16-parameter limit)
// ---------------------------------------------------------------------------

/// Resources watched for changes (dirty detection).
#[derive(SystemParam)]
struct WatchedResources<'w> {
    env_light_cfg: Res<'w, EnvironmentLightConfig>,
    lighting_rig: Res<'w, LightingRig>,
    material_cfg: Res<'w, MaterialConfig>,
    ssr_config: Res<'w, SsrConfig>,
    watch_config: Res<'w, WatchConfig>,
    timeline: Res<'w, Timeline>,
    env_obj_store: Res<'w, EnvironmentObjectStoreRes>,
    series_mgr: Res<'w, SeriesManagerRes>,
    source_orientation: Res<'w, SourceOrientation>,
    sequence_manager: Res<'w, SequenceManager>,
    saved_cameras: Option<Res<'w, SavedCameras>>,
}

impl WatchedResources<'_> {
    /// Returns a list of human-readable names of resources that changed this
    /// frame.  Empty vec means nothing changed.
    fn changed_names(&self) -> Vec<&'static str> {
        let mut names = Vec::new();
        if self.env_light_cfg.is_changed() { names.push("environment_light"); }
        if self.lighting_rig.is_changed() { names.push("lighting_rig"); }
        if self.material_cfg.is_changed() { names.push("material"); }
        if self.ssr_config.is_changed() { names.push("ssr"); }
        if self.watch_config.is_changed() { names.push("watch_config"); }
        if self.timeline.is_changed() { names.push("timeline"); }
        if self.env_obj_store.is_changed() { names.push("environment_objects"); }
        if self.series_mgr.is_changed() { names.push("series_manager"); }
        if self.source_orientation.is_changed() { names.push("source_orientation"); }
        if self.saved_cameras.as_ref().map_or(false, |sc| sc.is_changed()) {
            names.push("saved_cameras");
        }
        names
    }
}

// ---------------------------------------------------------------------------
// System
// ---------------------------------------------------------------------------

/// Combined change-detection + debounce + save system.
///
/// Runs once per frame in `PostUpdate`.  Checks whether any
/// settings-relevant resource was mutated this frame, marks the autosave
/// dirty if so, ticks the debounce timer, and performs the save (with
/// history rotation) when the timer expires.
///
/// Uses two camera queries:
/// - `camera_changed` with a `Changed<Transform>` filter for efficient
///   dirty detection (only yields results when the transform actually mutated).
/// - `camera_read` without the filter for reading the current transform at
///   save time (the change may have happened seconds ago, before the debounce
///   expired).
fn autosave_system(
    mut autosave: ResMut<AutosaveRes>,
    mut settings_res: ResMut<SettingsResource>,
    time: Res<Time>,
    mut ui_state: ResMut<UiState>,
    camera_changed: Query<(), (With<Camera3d>, With<FpsCamera>, Changed<Transform>)>,
    camera_read: Query<&Transform, (With<Camera3d>, With<FpsCamera>)>,
    watched: WatchedResources,
) {
    // -- Change detection ----------------------------------------------------
    // Log the *cause* first ("user did X"), then the effect ("→ marking dirty").

    let camera_moved = !camera_changed.is_empty();
    let changed_resources = watched.changed_names();

    if camera_moved || !changed_resources.is_empty() {
        if camera_moved && !changed_resources.is_empty() {
            debug!(
                "User moved camera + changed [{}] → marking dirty",
                changed_resources.join(", ")
            );
        } else if camera_moved {
            trace!("User moved camera → marking dirty");
        } else {
            debug!(
                "User changed [{}] → marking dirty",
                changed_resources.join(", ")
            );
        }
        autosave.state.mark_dirty();
    }

    // -- Tick + save ---------------------------------------------------------

    let delta = time.delta_secs_f64();
    let action = autosave.state.tick(delta);

    match action {
        AutosaveAction::Clean => return,
        AutosaveAction::Wait => return,
        AutosaveAction::Save => {
            debug!("Autosave: debounce expired → saving seaview.toml");
        }
    }

    // --- Gather current state into Settings ---

    // Camera — use the unrestricted query so we always get the transform
    if let Ok(transform) = camera_read.single() {
        settings_res
            .settings
            .set_camera_from_transform(transform);
    }

    // Playback + coordinates + watch
    settings_res.settings.set_playback(
        watched.sequence_manager.speed,
        watched.sequence_manager.loop_enabled,
        watched.watch_config.enabled,
    );
    settings_res.settings.sequence = Some(crate::lib::settings::SequenceSettings {
        source_coordinates: Some(watched.source_orientation.to_string()),
    });

    // Environment — env light + material + lighting rig
    settings_res
        .settings
        .set_environment(&watched.env_light_cfg, &watched.material_cfg);
    settings_res.settings.set_lighting_rig(&watched.lighting_rig);

    // Load sections / series
    if watched.series_mgr.len() > 0 {
        settings_res
            .settings
            .set_series_from_manager(&watched.series_mgr);
    } else {
        settings_res
            .settings
            .set_load_sections(&watched.timeline.sections);
    }

    // Environment objects
    settings_res
        .settings
        .set_environment_objects(&watched.env_obj_store);

    // SSR
    settings_res.settings.set_ssr(&watched.ssr_config);

    // TODO: persist EnvironmentMapConfig (env_map_config) — follow-up issue

    // --- History rotation + save ---

    if let Some(ref dir) = settings_res.directory {
        let settings_path = dir.join(SETTINGS_FILENAME);
        let max_history = autosave.state.config().max_history;

        if max_history > 0 {
            match rotate_history(&settings_path, max_history) {
                Ok(n) if n > 0 => {
                    debug!("📁 Rotated {} history file(s)", n);
                }
                Ok(_) => {} // no rotation needed (first save or no file)
                Err(e) => {
                    warn!("Failed to rotate settings history: {}", e);
                    // Continue with save anyway — history is nice-to-have
                }
            }
        }
    }

    match settings_res.save() {
        Ok(()) => {
            autosave.state.saved();

            let now = time.elapsed_secs_f64();
            ui_state.last_saved_at = Some(now);
            match ui_state.save_fade.as_mut() {
                Some(fade) => fade.restart(now),
                None => ui_state.save_fade = Some(FadeEffect::new(now, 3.0)),
            }
            debug!(
                "Autosave: wrote seaview.toml (save #{}) → set last_saved_at={:.2}, save_fade active",
                autosave.state.save_count(),
                now,
            );
        }
        Err(e) => {
            error!("Autosave: write failed: {} → will retry next frame", e);
            // Don't call saved() — will retry next frame
        }
    }
}
