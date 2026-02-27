//! Frame window management — entity spawning, despawning, and visibility.
//!
//! This module contains the [`manage_frame_window`] system that maintains the
//! ring-buffer of spawned frame entities, and the pure function
//! [`compute_window_delta`] that computes the set-difference between old and
//! new frame windows.
//!
//! [`manage_frame_window`] is the **sole owner of visibility** for sequence
//! mesh entities.  It runs in `PostUpdate` so that `timeline.playhead` is
//! finalised for the current tick.
//!
//! ## Per-series entity tracking (Phase 2)
//!
//! Entity spawning and despawning now goes through [`SeriesFrameStore`] —
//! each series independently tracks its spawned entities, displayed frame,
//! and previous spawn windows.  In Phase 2 only the active series has
//! entities spawned (behaviour unchanged from before).  Phase 3 will
//! extend this to iterate all visible series simultaneously.
//!
//! The global `FrameStore.frame_entities` and `FrameStore.displayed_frame`
//! are kept in sync as **mirrors** so that existing consumers (UI, spatial
//! index, etc.) continue to work without changes.

use bevy::mesh::Mesh;
use bevy::prelude::*;

use super::assets::{FrameStore, LoaderConfig};
use super::mesh_cache::MeshCache;
use super::series_frame_store::SeriesFrameStore;
use super::{rss_mb, SequenceMeshDisplay, SPAWNS_PER_FRAME};
use crate::app::ui::state::RenderConfig;
use crate::lib::environment_objects::Transform3D;
use crate::lib::memory_budget::MemoryBudget;
use crate::lib::sequence::{compute_spawn_windows, SeriesManagerRes, Timeline};

// ---------------------------------------------------------------------------
// Pure functions
// ---------------------------------------------------------------------------

/// Result of computing the delta between old and new frame windows.
///
/// Produced by [`compute_window_delta`], consumed by [`manage_frame_window`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowDelta {
    /// Frame indices that were in the old windows but are NOT in the new
    /// windows — candidates for despawning.
    pub to_despawn: Vec<usize>,
    /// Frame indices that are in the new windows but were NOT in the old
    /// windows — candidates for spawning.
    pub to_spawn: Vec<usize>,
}

/// Pure set-difference on window ranges.
///
/// Given the previous frame windows and the new frame windows (each a list
/// of `[lo, hi)` ranges), returns which indices left and which entered.
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

/// System that manages the ring-buffer of spawned frame entities.
///
/// This is the **sole owner of visibility** for sequence mesh entities.
/// Runs in `PostUpdate` so that `timeline.playhead` is finalised for this
/// tick (all `Update` writers have run).
///
/// Entity spawns are capped at [`SPAWNS_PER_FRAME`](super::SPAWNS_PER_FRAME)
/// to avoid stalling the render thread with a sudden batch of GPU uploads.
///
/// ## Per-series tracking (Phase 2)
///
/// Entity state is read/written via [`SeriesFrameStore`] for the active
/// series.  `FrameStore.frame_entities` and `FrameStore.displayed_frame`
/// are kept in sync as mirrors for backward compatibility with UI and
/// spatial-index consumers.
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
    // Need shared material before we can spawn anything
    let Some(material) = loader_config.shared_material.clone() else {
        return;
    };

    // Determine the active series.  If there's no active series, or it
    // isn't registered in SeriesFrameStore yet, fall back to the legacy
    // FrameStore path (prev_windows stored in FrameStore.displayed_frame
    // won't apply, but the system still works for single-series).
    let active_id = series_mgr.active_id();
    let active_series = series_mgr.active();

    // Ensure the active series is registered in SeriesFrameStore.
    if let Some(id) = active_id {
        if !series_fs.contains(&id) {
            series_fs.register(id, frame_store.total_frames);
        }
    }

    let current = timeline.playhead;

    // Compute spawn windows across ALL load sections.  Each section may
    // contribute a window if it overlaps the render region centred on the
    // playhead.  The union of these windows is the set of frames we want
    // spawned as entities.
    let new_windows = compute_spawn_windows(
        current,
        render_config.frame_window_size,
        &timeline.sections,
        frame_store.total_frames,
    );

    // Compose series transform (user position/rotation/scale) with the
    // coordinate-system transform.  Series transform is the "parent" —
    // it positions the whole series in world space.  Coord transform is
    // the "child" — it fixes axis orientation per-mesh.
    let series_transform = active_series
        .map(|s| s.transform.to_bevy_transform())
        .unwrap_or_default();
    let transform = series_transform * loader_config.coord_transform;
    let frame_bytes = frame_store.estimated_frame_bytes;

    // Get the previous windows from the per-series state (or empty if no
    // series registered yet — first frame after load).
    let prev_windows: Vec<(usize, usize)> = active_id
        .and_then(|id| series_fs.get(&id))
        .map(|state| state.prev_windows.clone())
        .unwrap_or_default();

    // Use the pure delta function to decide what to despawn/spawn.
    let delta = compute_window_delta(&prev_windows, &new_windows);

    // 1. Despawn entities that were in old windows but are NOT in new windows.
    //    Despawns are NOT throttled — freeing memory is always welcome.
    //    We also demote the mesh back to MAIN_WORLD only so Bevy's render
    //    asset system releases the GPU buffer.
    //
    //    IMPORTANT: defer despawning the previously displayed frame — we may
    //    need it as a visual fallback if the new playhead frame isn't ready.
    let prev_displayed = active_id
        .and_then(|id| series_fs.get(&id))
        .and_then(|state| state.displayed_frame);
    let rss_before_despawn = rss_mb();
    let mut despawns = 0usize;
    let mut demote_ok = 0usize;
    let mut demote_fail = 0usize;
    let mut deferred_despawn: Option<usize> = None;
    for idx in &delta.to_despawn {
        let idx = *idx;
        // Keep the previously displayed frame alive for now
        if Some(idx) == prev_displayed {
            deferred_despawn = Some(idx);
            continue;
        }

        // Take entities from the per-series store.
        let entities = active_id
            .and_then(|id| series_fs.get_mut(&id))
            .and_then(|state| state.take_entities(idx));
        if let Some(entities) = entities {
            for entity in &entities {
                if let Ok(mut vis) = visibility_query.get_mut(*entity) {
                    *vis = Visibility::Hidden;
                }
                commands.entity(*entity).despawn();
            }

            // GPU residency: try MeshCache refcounted demote first, fall
            // back to FrameStore direct demote for backward compat.
            let path = frame_store.frame_paths.get(idx).cloned();
            let demoted_via_cache = if let Some(ref p) = path {
                if let Some(cache_entry) = mesh_cache.get_mut(p) {
                    let should_demote = cache_entry.remove_gpu_consumer();
                    if should_demote {
                        cache_entry.demote_from_gpu(&mut meshes)
                    } else {
                        true // another series still using it — skip demote
                    }
                } else {
                    false
                }
            } else {
                false
            };

            if demoted_via_cache {
                demote_ok += 1;
            } else {
                // Fallback: direct demote via FrameStore (Phase 1 compat)
                let has_handle = frame_store.frame_handles[idx].is_some();
                if frame_store.demote_from_gpu(idx, &mut meshes) {
                    demote_ok += 1;
                } else {
                    demote_fail += 1;
                    if demote_fail <= 3 {
                        warn!(
                            "demote_from_gpu FAILED for frame {} (handle present: {})",
                            idx, has_handle
                        );
                    }
                }
            }

            if frame_bytes > 0 {
                memory_budget.record_frame_unloaded(frame_bytes);
            }

            // Mirror: keep FrameStore.frame_entities in sync.
            frame_store.frame_entities[idx] = None;

            despawns += 1;
        }
    }
    if despawns > 0 {
        let rss_after_despawn = rss_mb();
        debug!(
            "manage_frame_window: despawned {} entities (demote ok: {}, fail: {}) | RSS {:.1} → {:.1} MB (Δ {:.1})",
            despawns, demote_ok, demote_fail,
            rss_before_despawn, rss_after_despawn, rss_after_despawn - rss_before_despawn,
        );
    }

    // 2. Spawn entities that are in new windows but were NOT in old windows,
    //    PLUS retry any window frames that weren't ready on previous ticks.
    //    Capped at SPAWNS_PER_FRAME to spread GPU upload work.
    //
    //    Build the full set of candidate indices: delta.to_spawn first (new
    //    window entries), then all window frames that still lack entities
    //    (retry for assets that weren't ready when the window was created).
    let mut spawn_candidates: Vec<usize> = delta.to_spawn.clone();
    for &(start, end) in &new_windows {
        for idx in start..end {
            if !spawn_candidates.contains(&idx) {
                // Check per-series store first, fall back to FrameStore mirror.
                let is_spawned = active_id
                    .and_then(|id| series_fs.get(&id))
                    .map(|state| state.is_spawned(idx))
                    .unwrap_or_else(|| {
                        frame_store
                            .frame_entities
                            .get(idx)
                            .and_then(|e| e.as_ref())
                            .is_some()
                    });
                if !is_spawned {
                    spawn_candidates.push(idx);
                }
            }
        }
    }

    let mut spawns_this_frame = 0usize;
    for &idx in &spawn_candidates {
        if spawns_this_frame >= SPAWNS_PER_FRAME {
            break;
        }
        // Check if already spawned in per-series store.
        let already_spawned = active_id
            .and_then(|id| series_fs.get(&id))
            .map(|state| state.is_spawned(idx))
            .unwrap_or(false);
        if already_spawned {
            continue;
        }
        if !frame_store.is_frame_loaded(idx) {
            continue; // asset not ready yet
        }
        let Some(mesh_handles) = frame_store.get_frame_meshes(idx) else {
            continue;
        };
        let mesh_handles: Vec<Handle<Mesh>> = mesh_handles.to_vec();

        // GPU residency: promote via MeshCache refcount, fall back to
        // FrameStore direct promote.
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
                    Name::new(format!("Frame {} mesh {}", idx, mesh_i)),
                ))
                .id();
            entities.push(entity);
        }

        debug!("Spawned frame {} — {} entities", idx, entities.len(),);

        // Write to per-series store.
        if let Some(id) = active_id {
            if let Some(state) = series_fs.get_mut(&id) {
                state.set_entities(idx, entities.clone());
            }
        }

        // Mirror: keep FrameStore.frame_entities in sync.
        frame_store.frame_entities[idx] = Some(entities);

        if frame_bytes > 0 {
            memory_budget.record_frame_loaded(frame_bytes);
        }

        spawns_this_frame += 1;
    }

    // Log window changes
    if new_windows != prev_windows {
        // Count active handles for the log line.
        let active_handles = frame_store
            .frame_handles
            .iter()
            .filter(|fh| fh.is_some())
            .count();
        let live_entities = active_id
            .and_then(|id| series_fs.get(&id))
            .map(|state| state.spawned_count())
            .unwrap_or_else(|| {
                frame_store
                    .frame_entities
                    .iter()
                    .filter(|e| e.is_some())
                    .count()
            });
        debug!(
            "Frame windows changed: {:?} → {:?}, current frame: {} | handles: {}, entities: {}, RSS: {:.1} MB",
            prev_windows, new_windows, current,
            active_handles, live_entities, rss_mb(),
        );
    }

    // 3. Determine which frame to actually display.
    //    If the current playhead frame has a spawned entity and isn't
    //    degenerate, show it.  Otherwise, scan forward for the nearest
    //    available non-degenerate spawned frame.  As a last resort, use the
    //    deferred (previously displayed) frame.
    let is_displayable = |idx: usize| -> bool {
        let has_entity = active_id
            .and_then(|id| series_fs.get(&id))
            .map(|state| state.is_spawned(idx))
            .unwrap_or_else(|| {
                frame_store
                    .frame_entities
                    .get(idx)
                    .and_then(|e| e.as_ref())
                    .is_some()
            });
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
        // Scan forward from playhead for the nearest spawned frame
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
            debug!(
                "Frame {} not ready, skipping to next available frame {}",
                current, fb,
            );
            fb
        } else if deferred_despawn.is_some() && is_displayable(deferred_despawn.unwrap()) {
            // Fall back to the previously displayed frame we kept alive
            let fb = deferred_despawn.unwrap();
            debug!("Frame {} not ready, holding previous frame {}", current, fb,);
            fb
        } else {
            // Nothing spawned at all — will be black but unavoidable
            warn!(
                "⚫ No fallback: frame {} not ready and no spawned frames available",
                current,
            );
            current
        }
    };

    // Now despawn the deferred frame if it's not being used as the display
    if let Some(idx) = deferred_despawn {
        if idx != display_frame {
            let entities = active_id
                .and_then(|id| series_fs.get_mut(&id))
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

                // Mirror: keep FrameStore.frame_entities in sync.
                frame_store.frame_entities[idx] = None;

                debug!(
                    "Deferred despawn of frame {} (replaced by {})",
                    idx, display_frame
                );
            }
        }
    }

    // Set visibility: only the display_frame is Visible; everything else Hidden.
    // We iterate ALL spawned entities (across all windows AND the fallback) to
    // ensure exactly one frame is visible.
    //
    // Use the per-series store as the source of truth, with FrameStore as
    // fallback for the transition period.
    let entity_iter: Vec<(usize, Vec<Entity>)> = if let Some(id) = active_id {
        if let Some(state) = series_fs.get(&id) {
            state
                .frame_entities
                .iter()
                .enumerate()
                .filter_map(|(idx, opt)| opt.as_ref().map(|e| (idx, e.clone())))
                .collect()
        } else {
            frame_store
                .frame_entities
                .iter()
                .enumerate()
                .filter_map(|(idx, opt)| opt.as_ref().map(|e| (idx, e.clone())))
                .collect()
        }
    } else {
        frame_store
            .frame_entities
            .iter()
            .enumerate()
            .filter_map(|(idx, opt)| opt.as_ref().map(|e| (idx, e.clone())))
            .collect()
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

    // Update displayed frame and prev_windows in per-series state.
    if let Some(id) = active_id {
        if let Some(state) = series_fs.get_mut(&id) {
            // Log frame transitions (only when actually switching what's on screen)
            if state.displayed_frame != Some(display_frame) {
                debug!(
                    "▶ display frame {} (playhead: {}, prev: {:?})",
                    display_frame, current, state.displayed_frame,
                );
            }
            state.displayed_frame = Some(display_frame);
            state.prev_windows = new_windows;
        }
    }

    // Mirror: keep FrameStore.displayed_frame in sync.
    frame_store.displayed_frame = Some(display_frame);
}

/// Sync the transform of all spawned sequence-mesh entities when the active
/// series' transform changes.
///
/// Runs in `PostUpdate` alongside `manage_frame_window`.  Tracks the last
/// applied composed transform and, when it differs, bulk-updates every
/// spawned entity.
///
/// ## Per-series tracking (Phase 2)
///
/// Reads spawned entities from [`SeriesFrameStore`] for the active series,
/// with a fallback to `FrameStore.frame_entities` for backward compat.
pub(super) fn sync_series_transform(
    frame_store: Res<FrameStore>,
    loader_config: Res<LoaderConfig>,
    series_mgr: Res<SeriesManagerRes>,
    series_fs: Res<SeriesFrameStore>,
    mut transform_query: Query<&mut Transform, With<SequenceMeshDisplay>>,
    mut prev_transform: Local<Option<Transform3D>>,
) {
    let current_t3d = series_mgr
        .active()
        .map(|s| s.transform.clone())
        .unwrap_or_default();

    // Early out if transform hasn't changed
    let changed = match prev_transform.as_ref() {
        Some(prev) => {
            prev.position != current_t3d.position
                || prev.rotation_degrees != current_t3d.rotation_degrees
                || prev.scale != current_t3d.scale
        }
        None => true, // first run — always apply
    };

    if !changed {
        return;
    }

    let composed = current_t3d.to_bevy_transform() * loader_config.coord_transform;

    let active_id = series_mgr.active_id();

    // Prefer per-series entity store; fall back to FrameStore mirror.
    let used_series_store = if let Some(id) = active_id {
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
            true
        } else {
            false
        }
    } else {
        false
    };

    if !used_series_store {
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

    *prev_transform = Some(current_t3d);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── compute_window_delta ──────────────────────────────────────────

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
        // Left out of old: 0,1,2 from first window; 10,11 from second
        assert_eq!(delta.to_despawn, vec![0, 1, 2, 10, 11]);
        // New in new: 5,6,7 from first window; 15,16 from second
        assert_eq!(delta.to_spawn, vec![5, 6, 7, 15, 16]);
    }

    #[test]
    fn test_window_delta_overlapping_new_windows() {
        // Two new windows that overlap: [0,5) and [3,8)
        // Old was [0,5)
        let old = vec![(0, 5)];
        let new = vec![(0, 5), (3, 8)];
        let delta = compute_window_delta(&old, &new);
        assert!(delta.to_despawn.is_empty());
        assert_eq!(delta.to_spawn, vec![5, 6, 7]);
    }
}
