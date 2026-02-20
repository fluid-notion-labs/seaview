//! App-layer autosave system.
//!
//! Watches settings-relevant Bevy resources for changes, marks the autosave
//! state dirty on any mutation, and triggers a debounced save with history
//! rotation.
//!
//! ## Design
//!
//! - **Snapshot-based change detection**: each frame, the system gathers all
//!   saveable state into a [`Settings`] struct, hashes it, and compares to the
//!   previous hash.  This bypasses Bevy's `is_changed()` which fires on every
//!   `ResMut` borrow even when values don't actually change.
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

use std::hash::{Hash, Hasher};

use bevy::prelude::*;
use log::debug;

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
    /// Hash of the last snapshot we compared against.  When the current
    /// snapshot hash differs, we know the user actually changed something.
    prev_hash: u64,
    /// Elapsed time since app start.  Change detection is suppressed until
    /// this exceeds [`STARTUP_GRACE_SECS`] so that resource initialisation
    /// during loading doesn't spam dirty marks.
    elapsed: f64,
    /// Whether the initial snapshot has been captured (after grace period).
    baseline_captured: bool,
}

/// Seconds after app start before autosave change detection activates.
/// During this window, resources like timeline, series_manager, watch_config
/// etc. are being initialised by the loader and would trigger false positives.
const STARTUP_GRACE_SECS: f64 = 5.0;

impl Default for AutosaveRes {
    fn default() -> Self {
        Self {
            state: AutosaveState::with_defaults(),
            prev_hash: 0,
            elapsed: 0.0,
            baseline_captured: false,
        }
    }
}

impl AutosaveRes {
    pub fn new(config: AutosaveConfig) -> Self {
        Self {
            state: AutosaveState::new(config),
            prev_hash: 0,
            elapsed: 0.0,
            baseline_captured: false,
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
// Snapshot hashing
// ---------------------------------------------------------------------------

/// A lightweight, hashable snapshot of all settings-relevant state.
///
/// We don't need a perfect hash — collisions just mean we occasionally skip
/// a save, and the next real change will catch it.  Using `f32_bits` for
/// floats so NaN etc. don't break hashing.
struct Snapshot {
    hash: u64,
}

impl Snapshot {
    fn capture(
        camera_transform: Option<&Transform>,
        env_light: &EnvironmentLightConfig,
        lighting_rig: &LightingRig,
        material: &MaterialConfig,
        ssr: &SsrConfig,
        watch: &WatchConfig,
        timeline: &Timeline,
        env_objs: &EnvironmentObjectStoreRes,
        series_mgr: &SeriesManagerRes,
        source_orientation: &SourceOrientation,
        seq_mgr: &SequenceManager,
        saved_cameras: Option<&SavedCameras>,
    ) -> Self {
        let mut h = FnvHasher::new();

        // Camera
        if let Some(t) = camera_transform {
            hash_vec3(&mut h, &t.translation);
            hash_quat(&mut h, &t.rotation);
        }

        // Environment light
        env_light.enabled.hash(&mut h);
        hash_color(&mut h, &env_light.ambient_color);
        hash_f32(&mut h, env_light.ambient_brightness);
        hash_f32(&mut h, env_light.directional_illuminance);
        hash_color(&mut h, &env_light.directional_color);

        // Lighting rig — hash group count + each group's name and light count
        lighting_rig.groups.len().hash(&mut h);
        for group in &lighting_rig.groups {
            group.name.hash(&mut h);
            group.visible.hash(&mut h);
            group.lights.len().hash(&mut h);
            for light in &group.lights {
                hash_f32(&mut h, light.intensity);
                hash_f32(&mut h, light.position.x);
                hash_f32(&mut h, light.position.y);
                hash_f32(&mut h, light.height);
                hash_color(&mut h, &light.color);
                light.visible.hash(&mut h);
                hash_f32(&mut h, light.range);
                hash_f32(&mut h, light.cone_angle);
            }
        }

        // Material
        hash_color(&mut h, &material.base_color);
        hash_f32(&mut h, material.perceptual_roughness);
        hash_f32(&mut h, material.metallic);

        // SSR
        ssr.enabled.hash(&mut h);
        hash_f32(&mut h, ssr.perceptual_roughness_threshold);
        hash_f32(&mut h, ssr.thickness);
        ssr.linear_steps.hash(&mut h);
        hash_f32(&mut h, ssr.linear_march_exponent);
        ssr.bisection_steps.hash(&mut h);
        ssr.use_secant.hash(&mut h);

        // Watch
        watch.enabled.hash(&mut h);

        // Timeline — sections
        timeline.sections.len().hash(&mut h);
        for section in &timeline.sections {
            section.start.hash(&mut h);
            section.end.hash(&mut h);
        }

        // Environment objects — use the generation counter (bumped on every mutation)
        env_objs.0.generation().hash(&mut h);
        env_objs.0.len().hash(&mut h);
        for obj in env_objs.0.iter() {
            obj.id.hash(&mut h);
            obj.path.hash(&mut h);
            obj.visible.hash(&mut h);
            hash_f32(&mut h, obj.transform.position[0]);
            hash_f32(&mut h, obj.transform.position[1]);
            hash_f32(&mut h, obj.transform.position[2]);
            hash_f32(&mut h, obj.transform.rotation_degrees[0]);
            hash_f32(&mut h, obj.transform.rotation_degrees[1]);
            hash_f32(&mut h, obj.transform.rotation_degrees[2]);
            hash_f32(&mut h, obj.transform.scale[0]);
            hash_f32(&mut h, obj.transform.scale[1]);
            hash_f32(&mut h, obj.transform.scale[2]);
        }

        // Series manager — active series + count
        series_mgr.len().hash(&mut h);
        if let Some(active) = series_mgr.active_id() {
            active.raw().hash(&mut h);
        }

        // Source orientation
        source_orientation.to_string().hash(&mut h);

        // Sequence manager — speed + loop
        hash_f32(&mut h, seq_mgr.speed);
        seq_mgr.loop_enabled.hash(&mut h);

        // Saved cameras
        if let Some(sc) = saved_cameras {
            sc.active.hash(&mut h);
            sc.cameras.len().hash(&mut h);
            // Sort keys for deterministic hashing
            let mut keys: Vec<_> = sc.cameras.keys().collect();
            keys.sort();
            for key in keys {
                key.hash(&mut h);
                if let Some(cam) = sc.cameras.get(key) {
                    hash_f32(&mut h, cam.position[0]);
                    hash_f32(&mut h, cam.position[1]);
                    hash_f32(&mut h, cam.position[2]);
                    hash_f32(&mut h, cam.rotation[0]);
                    hash_f32(&mut h, cam.rotation[1]);
                    hash_f32(&mut h, cam.rotation[2]);
                }
            }
        }

        Snapshot {
            hash: h.finish(),
        }
    }
}

/// FNV-1a 64-bit hasher — fast, good enough for change detection.
struct FnvHasher(u64);

impl FnvHasher {
    fn new() -> Self {
        Self(0xcbf29ce484222325)
    }
}

impl Hasher for FnvHasher {
    fn finish(&self) -> u64 {
        self.0
    }

    fn write(&mut self, bytes: &[u8]) {
        for &byte in bytes {
            self.0 ^= byte as u64;
            self.0 = self.0.wrapping_mul(0x100000001b3);
        }
    }
}

fn hash_f32(h: &mut FnvHasher, v: f32) {
    v.to_bits().hash(h);
}

fn hash_vec3(h: &mut FnvHasher, v: &Vec3) {
    hash_f32(h, v.x);
    hash_f32(h, v.y);
    hash_f32(h, v.z);
}

fn hash_quat(h: &mut FnvHasher, q: &Quat) {
    hash_f32(h, q.x);
    hash_f32(h, q.y);
    hash_f32(h, q.z);
    hash_f32(h, q.w);
}

fn hash_color(h: &mut FnvHasher, c: &Color) {
    let srgba = c.to_srgba();
    hash_f32(h, srgba.red);
    hash_f32(h, srgba.green);
    hash_f32(h, srgba.blue);
    hash_f32(h, srgba.alpha);
}

// ---------------------------------------------------------------------------
// SystemParam bundles (to stay under Bevy's 16-parameter limit)
// ---------------------------------------------------------------------------

/// Resources read for snapshot hashing.
#[derive(SystemParam)]
struct SnapshotResources<'w> {
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

// ---------------------------------------------------------------------------
// System
// ---------------------------------------------------------------------------

/// Combined snapshot-based change detection + debounce + save system.
///
/// Each frame: snapshot all saveable state → hash → compare to previous hash.
/// If different, mark dirty + start debounce.  When debounce expires, save.
fn autosave_system(
    mut autosave: ResMut<AutosaveRes>,
    mut settings_res: ResMut<SettingsResource>,
    time: Res<Time>,
    mut ui_state: ResMut<UiState>,
    camera_query: Query<&Transform, (With<Camera3d>, With<FpsCamera>)>,
    resources: SnapshotResources,
) {
    // -- Startup grace period ------------------------------------------------
    autosave.elapsed += time.delta_secs_f64();
    if autosave.elapsed < STARTUP_GRACE_SECS {
        return;
    }

    // -- Snapshot + compare --------------------------------------------------
    let camera_transform = camera_query.iter().next();

    let snapshot = Snapshot::capture(
        camera_transform,
        &resources.env_light_cfg,
        &resources.lighting_rig,
        &resources.material_cfg,
        &resources.ssr_config,
        &resources.watch_config,
        &resources.timeline,
        &resources.env_obj_store,
        &resources.series_mgr,
        &resources.source_orientation,
        &resources.sequence_manager,
        resources.saved_cameras.as_deref(),
    );

    // First frame after grace period: capture baseline, don't mark dirty.
    if !autosave.baseline_captured {
        autosave.prev_hash = snapshot.hash;
        autosave.baseline_captured = true;
        debug!("Autosave: baseline captured (hash={:#018x})", snapshot.hash);
        return;
    }

    if snapshot.hash != autosave.prev_hash {
        let was_dirty = autosave.state.is_dirty();
        autosave.state.mark_dirty();
        autosave.prev_hash = snapshot.hash;

        if !was_dirty {
            debug!(
                "Autosave: state changed (hash={:#018x}) → marking dirty, debounce started",
                snapshot.hash,
            );
        }
    }

    // -- Tick + save ---------------------------------------------------------

    let delta = time.delta_secs_f64();
    let action = autosave.state.tick(delta);

    match action {
        AutosaveAction::Clean | AutosaveAction::Wait => return,
        AutosaveAction::Save => {
            debug!("Autosave: debounce expired → saving seaview.toml");
        }
    }

    // --- Gather current state into Settings ---

    // Camera
    if let Some(transform) = camera_transform {
        settings_res
            .settings
            .set_camera_from_transform(transform);
    }

    // Playback + coordinates + watch
    settings_res.settings.set_playback(
        resources.sequence_manager.speed,
        resources.sequence_manager.loop_enabled,
        resources.watch_config.enabled,
    );
    settings_res.settings.sequence = Some(crate::lib::settings::SequenceSettings {
        source_coordinates: Some(resources.source_orientation.to_string()),
    });

    // Environment — env light + material + lighting rig
    settings_res
        .settings
        .set_environment(&resources.env_light_cfg, &resources.material_cfg);
    settings_res
        .settings
        .set_lighting_rig(&resources.lighting_rig);

    // Load sections / series
    if resources.series_mgr.len() > 0 {
        settings_res
            .settings
            .set_series_from_manager(&resources.series_mgr);
    } else {
        settings_res
            .settings
            .set_load_sections(&resources.timeline.sections);
    }

    // Environment objects
    settings_res
        .settings
        .set_environment_objects(&resources.env_obj_store);

    // SSR
    settings_res.settings.set_ssr(&resources.ssr_config);

    // TODO: persist EnvironmentMapConfig — follow-up issue

    // --- History rotation + save ---

    if let Some(ref dir) = settings_res.directory {
        let settings_path = dir.join(SETTINGS_FILENAME);
        let max_history = autosave.state.config().max_history;

        if max_history > 0 {
            match rotate_history(&settings_path, max_history) {
                Ok(n) if n > 0 => {
                    debug!("Autosave: rotated {} history file(s)", n);
                }
                Ok(_) => {}
                Err(e) => {
                    warn!("Autosave: failed to rotate history: {}", e);
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
                "Autosave: wrote seaview.toml (save #{}) → last_saved_at={:.2}, save_fade active",
                autosave.state.save_count(),
                now,
            );
        }
        Err(e) => {
            error!("Autosave: write failed: {} → will retry next frame", e);
        }
    }
}
