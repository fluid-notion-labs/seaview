//! Frame window management — entity spawning, despawning, and visibility.
//!
//! [`manage_frame_window`] maintains the ring-buffer of spawned frame entities
//! for **all visible series** simultaneously.  Each visible series has its own
//! spawn window, entity lifecycle, and displayed frame determined by its own
//! playhead.
//!
//! [`compute_window_delta`] is a pure function computing the set-difference
//! between old and new frame windows.
//!
//! ## Phase 3 (current)
//!
//! All series in `series_mgr.visible_ids()` have frame entities spawned and
//! displayed each tick.  The active series continues to drive the global
//! `Timeline` resource for playback controls.  Each non-active visible series
//! uses its own per-series `Timeline` (stored in `SeriesManagerRes`) for its
//! playhead.
//!
//! ## Entity tracking
//!
//! Entity spawning/despawning goes through [`SeriesFrameStore`].  Each
//! registered series has a [`SeriesEntityState`] that tracks its own entities,
//! displayed frame, and previous spawn windows.
//!
//! `FrameStore.frame_entities` and `FrameStore.displayed_frame` are kept in
//! sync as mirrors for the **active series only**, preserving backward
//! compatibility with existing UI and spatial-index consumers.

use bevy::math::{Mat3, Vec3};
use bevy::mesh::Mesh;
use bevy::prelude::*;

use super::assets::{FrameStore, LoaderConfig};
use super::mesh_cache::MeshCache;
use super::series_frame_store::SeriesFrameStore;
use super::{rss_mb, SequenceMeshDisplay, SPAWNS_PER_FRAME};
use crate::app::ui::state::RenderConfig;
use crate::lib::environment_objects::{AxisSwizzle, Transform3D};
use crate::lib::memory_budget::MemoryBudget;
use crate::lib::sequence::{compute_spawn_windows, SeriesManagerRes, Timeline};

// ---------------------------------------------------------------------------
// Pure functions
// ---------------------------------------------------------------------------

/// Result of computing the delta between old and new frame windows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowDelta {
    pub to_despawn: Vec<usize>,
    pub to_spawn: Vec<usize>,
}

/// Pure set-difference on window ranges.
pub fn compute_window_delta(
    old_windows: &[(usize, usize)],
    new_windows: &[(usize, usize)],
) -> WindowDelta {
    let in_windows = |idx: usize, windows: &[(usize, usize)]| -> bool {
        windows.iter().any(|&(lo, hi)| idx >= lo && idx < hi)
    };

    let mut to_despawn = Vec::new();
    for &(lo, hi) in old_windows {
        for idx in lo..hi {
            if !in_windows(idx, new_windows) {
                to_despawn.push(idx);
            }
        }
    }

    let mut to_spawn = Vec::new();
    for &(lo, hi) in new_windows {
        for idx in lo..hi {
            if !in_windows(idx, old_windows) {
                to_spawn.push(idx);
            }
        }
    }

    WindowDelta {
        to_despawn,
        to_spawn,
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Manage the ring-buffer of spawned frame entities for all visible series.
///
/// Runs in `PostUpdate` so that `timeline.playhead` is finalised for the
/// current tick.  Entity spawns are capped at [`SPAWNS_PER_FRAME`] *per
/// series* to spread GPU upload work.
///
/// For the active series the global `Timeline` resource provides the playhead.
/// For all other visible series the per-series `Timeline` stored in
/// `SeriesManagerRes` is used directly.
pub(super) fn manage_frame_window(
    mut commands: Commands,
    mut frame_store: ResMut<FrameStore>,
    loader_config: Res<LoaderConfig>,
    timeline: Res<Timeline>,
    render_config: Res<RenderConfig>,
    mut visibility_query: Query<&mut Visibility, With<SequenceMeshDisplay>>,
    mut memory_budget: ResMut<MemoryBudget>,
    mut meshes: ResMut<Assets<Mesh>>,
    series_mgr: Res<SeriesManagerRes>,
    mut series_fs: ResMut<SeriesFrameStore>,
    mut mesh_cache: ResMut<MeshCache>,
) {
    if frame_store.total_frames == 0 {
        return;
    }
    let Some(material) = loader_config.shared_material.clone() else {
        return;
    };

    let active_id = series_mgr.active_id();
    let visible_ids = series_mgr.visible_ids();

    // Ensure every visible series is registered in SeriesFrameStore.
    for &id in &visible_ids {
        if !series_fs.contains(&id) {
            series_fs.register(id, frame_store.total_frames);
        }
    }

    // Despawn entities belonging to series that are no longer visible.
    // Collect IDs to avoid borrow issues.
    let registered_ids: Vec<_> = series_fs.iter().map(|(id, _)| *id).collect();
    for id in registered_ids {
        if !visible_ids.contains(&id) {
            let entities = series_fs.remove(&id);
            for entity in entities {
                commands.entity(entity).despawn();
            }
        }
    }

    let frame_bytes = frame_store.estimated_frame_bytes;

    // Process each visible series independently.
    for &series_id in &visible_ids {
        // Determine the playhead for this series.
        // Active series uses the global Timeline (which may have been advanced
        // by playback / keyboard this tick).  Other series use their own
        // per-series timeline stored in SeriesManagerRes.
        let (current, sections, total_frames) = if Some(series_id) == active_id {
            (
                timeline.playhead,
                timeline.sections.clone(),
                timeline.total_frames,
            )
        } else {
            match series_mgr.get(series_id) {
                Some(state) => (
                    state.timeline.playhead,
                    state.timeline.sections.clone(),
                    state.timeline.total_frames,
                ),
                None => continue,
            }
        };

        if total_frames == 0 {
            continue;
        }

        // Compose series transform.
        let (series_swizzle, series_t3d) = series_mgr
            .get(series_id)
            .map(|s| (s.swizzle.clone(), s.transform.clone()))
            .unwrap_or_default();
        let series_transform = compose_series_transform(&series_t3d, &series_swizzle);
        let transform = series_transform * loader_config.coord_transform;

        // Compute spawn windows.
        let new_windows = compute_spawn_windows(
            current,
            render_config.frame_window_size,
            &sections,
            frame_store.total_frames,
        );

        let prev_windows: Vec<(usize, usize)> = series_fs
            .get(&series_id)
            .map(|state| state.prev_windows.clone())
            .unwrap_or_default();

        let delta = compute_window_delta(&prev_windows, &new_windows);

        // --- Despawn ---------------------------------------------------
        let prev_displayed = series_fs
            .get(&series_id)
            .and_then(|state| state.displayed_frame);

        let mut despawns = 0usize;
        let mut deferred_despawn: Option<usize> = None;

        for idx in &delta.to_despawn {
            let idx = *idx;
            if Some(idx) == prev_displayed {
                deferred_despawn = Some(idx);
                continue;
            }

            let entities = series_fs
                .get_mut(&series_id)
                .and_then(|state| state.take_entities(idx));

            if let Some(entities) = entities {
                for entity in &entities {
                    if let Ok(mut vis) = visibility_query.get_mut(*entity) {
                        *vis = Visibility::Hidden;
                    }
                    commands.entity(*entity).despawn();
                }

                // GPU residency: MeshCache refcounted demote.
                let path = frame_store.frame_paths.get(idx).cloned();
                let demoted_via_cache = if let Some(ref p) = path {
                    if let Some(cache_entry) = mesh_cache.get_mut(p) {
                        let should_demote = cache_entry.remove_gpu_consumer();
                        if should_demote {
                            cache_entry.demote_from_gpu(&mut meshes)
                        } else {
                            true
                        }
                    } else {
                        false
                    }
                } else {
                    false
                };

                if !demoted_via_cache {
                    frame_store.demote_from_gpu(idx, &mut meshes);
                }

                if frame_bytes > 0 {
                    memory_budget.record_frame_unloaded(frame_bytes);
                }

                // Mirror for active series only.
                if Some(series_id) == active_id {
                    frame_store.frame_entities[idx] = None;
                }

                despawns += 1;
            }
        }

        if despawns > 0 {
            debug!(
                "[series {}] manage_frame_window: despawned {} entities",
                series_id, despawns,
            );
        }

        // --- Spawn -----------------------------------------------------
        let mut spawn_candidates: Vec<usize> = delta.to_spawn.clone();
        for &(start, end) in &new_windows {
            for idx in start..end {
                if !spawn_candidates.contains(&idx) {
                    let is_spawned = series_fs
                        .get(&series_id)
                        .map(|state| state.is_spawned(idx))
                        .unwrap_or(false);
                    if !is_spawned {
                        spawn_candidates.push(idx);
                    }
                }
            }
        }

        let mut spawns_this_series = 0usize;
        for &idx in &spawn_candidates {
            if spawns_this_series >= SPAWNS_PER_FRAME {
                break;
            }
            let already_spawned = series_fs
                .get(&series_id)
                .map(|state| state.is_spawned(idx))
                .unwrap_or(false);
            if already_spawned {
                continue;
            }
            if !frame_store.is_frame_loaded(idx) {
                continue;
            }
            let Some(mesh_handles) = frame_store.get_frame_meshes(idx) else {
                continue;
            };
            let mesh_handles: Vec<Handle<Mesh>> = mesh_handles.to_vec();

            // GPU residency: promote.
            let path = frame_store.frame_paths.get(idx).cloned();
            if let Some(ref p) = path {
                if let Some(cache_entry) = mesh_cache.get_mut(p) {
                    cache_entry.add_gpu_consumer();
                    cache_entry.promote_to_gpu(&mut meshes);
                } else {
                    frame_store.promote_to_gpu(idx, &mut meshes);
                }
            } else {
                frame_store.promote_to_gpu(idx, &mut meshes);
            }

            let visibility = if idx == current {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };

            let mut entities = Vec::with_capacity(mesh_handles.len());
            for (mesh_i, handle) in mesh_handles.iter().enumerate() {
                let entity = commands
                    .spawn((
                        Mesh3d(handle.clone()),
                        MeshMaterial3d(material.clone()),
                        transform,
                        SequenceMeshDisplay,
                        visibility,
                        Name::new(format!(
                            "Series {} Frame {} mesh {}",
                            series_id, idx, mesh_i
                        )),
                    ))
                    .id();
                entities.push(entity);
            }

            debug!(
                "[series {}] Spawned frame {} — {} entities",
                series_id,
                idx,
                entities.len(),
            );

            if let Some(state) = series_fs.get_mut(&series_id) {
                state.set_entities(idx, entities.clone());
            }

            // Mirror for active series only.
            if Some(series_id) == active_id {
                frame_store.frame_entities[idx] = Some(entities);
            }

            if frame_bytes > 0 {
                memory_budget.record_frame_loaded(frame_bytes);
            }

            spawns_this_series += 1;
        }

        // --- Determine display frame -----------------------------------
        let is_displayable = |idx: usize| -> bool {
            let has_entity = series_fs
                .get(&series_id)
                .map(|state| state.is_spawned(idx))
                .unwrap_or(false);
            let is_degenerate = frame_store
                .degenerate_frames
                .get(idx)
                .copied()
                .unwrap_or(false);
            has_entity && !is_degenerate
        };

        let display_frame = if is_displayable(current) {
            current
        } else {
            let n = frame_store.total_frames;
            let mut found = None;
            for offset in 1..n {
                let candidate = (current + offset) % n;
                if is_displayable(candidate) {
                    found = Some(candidate);
                    break;
                }
            }
            if let Some(fb) = found {
                fb
            } else if deferred_despawn.is_some()
                && is_displayable(deferred_despawn.unwrap())
            {
                deferred_despawn.unwrap()
            } else {
                current
            }
        };

        // Despawn deferred frame if not being used as the display.
        if let Some(idx) = deferred_despawn {
            if idx != display_frame {
                let entities = series_fs
                    .get_mut(&series_id)
                    .and_then(|state| state.take_entities(idx));
                if let Some(entities) = entities {
                    for entity in &entities {
                        if let Ok(mut vis) = visibility_query.get_mut(*entity) {
                            *vis = Visibility::Hidden;
                        }
                        commands.entity(*entity).despawn();
                    }

                    let path = frame_store.frame_paths.get(idx).cloned();
                    let demoted_via_cache = if let Some(ref p) = path {
                        if let Some(cache_entry) = mesh_cache.get_mut(p) {
                            let should_demote = cache_entry.remove_gpu_consumer();
                            if should_demote {
                                cache_entry.demote_from_gpu(&mut meshes)
                            } else {
                                true
                            }
                        } else {
                            false
                        }
                    } else {
                        false
                    };

                    if !demoted_via_cache {
                        frame_store.demote_from_gpu(idx, &mut meshes);
                    }

                    if frame_bytes > 0 {
                        memory_budget.record_frame_unloaded(frame_bytes);
                    }

                    if Some(series_id) == active_id {
                        frame_store.frame_entities[idx] = None;
                    }
                }
            }
        }

        // --- Set visibility: exactly one frame visible per series ------
        let entity_iter: Vec<(usize, Vec<Entity>)> =
            if let Some(state) = series_fs.get(&series_id) {
                state
                    .frame_entities
                    .iter()
                    .enumerate()
                    .filter_map(|(idx, opt)| opt.as_ref().map(|e| (idx, e.clone())))
                    .collect()
            } else {
                Vec::new()
            };

        for (idx, entities) in &entity_iter {
            let desired = if *idx == display_frame {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };
            for entity in entities {
                if let Ok(mut vis) = visibility_query.get_mut(*entity) {
                    if *vis != desired {
                        *vis = desired;
                    }
                }
            }
        }

        // --- Update per-series state -----------------------------------
        if let Some(state) = series_fs.get_mut(&series_id) {
            if state.displayed_frame != Some(display_frame) {
                debug!(
                    "[series {}] ▶ display frame {} (playhead: {}, prev: {:?})",
                    series_id, display_frame, current, state.displayed_frame,
                );
            }
            state.displayed_frame = Some(display_frame);
            state.prev_windows = new_windows;
        }

        // Mirror for active series only.
        if Some(series_id) == active_id {
            frame_store.displayed_frame = Some(display_frame);
        }
    }
}

/// Sync the transform of all spawned sequence-mesh entities when a visible
/// series' transform changes.
///
/// Tracks the last applied transform per-series and bulk-updates entities
/// only when something changed.
pub(super) fn sync_series_transform(
    frame_store: Res<FrameStore>,
    loader_config: Res<LoaderConfig>,
    series_mgr: Res<SeriesManagerRes>,
    series_fs: Res<SeriesFrameStore>,
    mut transform_query: Query<&mut Transform, With<SequenceMeshDisplay>>,
    mut prev_transforms: Local<
        std::collections::HashMap<
            crate::lib::series::SeriesId,
            (Transform3D, AxisSwizzle),
        >,
    >,
) {
    for series_state in series_mgr.visible_iter() {
        let id = series_state.id;
        let current_t3d = series_state.transform.clone();
        let current_swizzle = series_state.swizzle.clone();

        let changed = match prev_transforms.get(&id) {
            Some((prev_t, prev_s)) => {
                prev_t.position != current_t3d.position
                    || prev_t.rotation_degrees != current_t3d.rotation_degrees
                    || prev_t.scale != current_t3d.scale
                    || *prev_s != current_swizzle
            }
            None => true,
        };

        if !changed {
            continue;
        }

        let composed = compose_series_transform(&current_t3d, &current_swizzle)
            * loader_config.coord_transform;

        // Use per-series entity store.
        if let Some(state) = series_fs.get(&id) {
            for entity_opt in state.frame_entities.iter() {
                if let Some(entities) = entity_opt {
                    for entity in entities {
                        if let Ok(mut t) = transform_query.get_mut(*entity) {
                            *t = composed;
                        }
                    }
                }
            }
        }

        prev_transforms.insert(id, (current_t3d, current_swizzle));
    }

    // Fallback: if no visible series use the legacy FrameStore path
    // (handles the transient state between series manager init and first
    // registration tick).
    if series_mgr.visible_count() == 0 {
        let active = series_mgr.active();
        let current_t3d = active.map(|s| s.transform.clone()).unwrap_or_default();
        let current_swizzle = active.map(|s| s.swizzle.clone()).unwrap_or_default();
        let composed =
            compose_series_transform(&current_t3d, &current_swizzle) * loader_config.coord_transform;
        for entity_opt in frame_store.frame_entities.iter() {
            if let Some(entities) = entity_opt {
                for entity in entities {
                    if let Ok(mut t) = transform_query.get_mut(*entity) {
                        *t = composed;
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Transform composition helper
// ---------------------------------------------------------------------------

pub(super) fn compose_series_transform(t3d: &Transform3D, swizzle: &AxisSwizzle) -> Transform {
    let user_transform = t3d.to_bevy_transform();

    if swizzle.is_identity() {
        return user_transform;
    }

    let m = swizzle.to_matrix();
    let swizzle_mat = Mat3::from_cols(
        Vec3::new(m[0][0], m[0][1], m[0][2]),
        Vec3::new(m[1][0], m[1][1], m[1][2]),
        Vec3::new(m[2][0], m[2][1], m[2][2]),
    );

    let det = swizzle_mat.determinant();
    let (rot_mat, swizzle_scale) = if det < 0.0 {
        let flipped =
            Mat3::from_cols(swizzle_mat.col(0), swizzle_mat.col(1), -swizzle_mat.col(2));
        (flipped, Vec3::new(1.0, 1.0, -1.0))
    } else {
        (swizzle_mat, Vec3::ONE)
    };
    let swizzle_rot = Quat::from_mat3(&rot_mat);

    Transform {
        translation: user_transform.translation,
        rotation: user_transform.rotation * swizzle_rot,
        scale: user_transform.scale * swizzle_scale,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_window_delta_no_change() {
        let old = vec![(5, 10)];
        let new = vec![(5, 10)];
        let delta = compute_window_delta(&old, &new);
        assert!(delta.to_despawn.is_empty());
        assert!(delta.to_spawn.is_empty());
    }

    #[test]
    fn test_window_delta_shift_right() {
        let old = vec![(5, 10)];
        let new = vec![(7, 12)];
        let delta = compute_window_delta(&old, &new);
        assert_eq!(delta.to_despawn, vec![5, 6]);
        assert_eq!(delta.to_spawn, vec![10, 11]);
    }

    #[test]
    fn test_window_delta_shift_left() {
        let old = vec![(5, 10)];
        let new = vec![(3, 8)];
        let delta = compute_window_delta(&old, &new);
        assert_eq!(delta.to_despawn, vec![8, 9]);
        assert_eq!(delta.to_spawn, vec![3, 4]);
    }

    #[test]
    fn test_window_delta_completely_new() {
        let old = vec![(0, 5)];
        let new = vec![(10, 15)];
        let delta = compute_window_delta(&old, &new);
        assert_eq!(delta.to_despawn, vec![0, 1, 2, 3, 4]);
        assert_eq!(delta.to_spawn, vec![10, 11, 12, 13, 14]);
    }

    #[test]
    fn test_window_delta_empty_to_some() {
        let delta = compute_window_delta(&[], &[(5, 10)]);
        assert!(delta.to_despawn.is_empty());
        assert_eq!(delta.to_spawn, vec![5, 6, 7, 8, 9]);
    }

    #[test]
    fn test_window_delta_some_to_empty() {
        let delta = compute_window_delta(&[(5, 10)], &[]);
        assert_eq!(delta.to_despawn, vec![5, 6, 7, 8, 9]);
        assert!(delta.to_spawn.is_empty());
    }

    #[test]
    fn test_window_delta_multi_window() {
        let old = vec![(0, 5), (10, 15)];
        let new = vec![(3, 8), (12, 17)];
        let delta = compute_window_delta(&old, &new);
        assert_eq!(delta.to_despawn, vec![0, 1, 2, 10, 11]);
        assert_eq!(delta.to_spawn, vec![5, 6, 7, 15, 16]);
    }

    #[test]
    fn test_window_delta_overlapping_new_windows() {
        let old = vec![(0, 5)];
        let new = vec![(0, 5), (3, 8)];
        let delta = compute_window_delta(&old, &new);
        assert!(delta.to_despawn.is_empty());
        assert_eq!(delta.to_spawn, vec![5, 6, 7]);
    }
}
