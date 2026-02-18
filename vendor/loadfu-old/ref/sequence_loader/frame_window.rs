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

use bevy::mesh::Mesh;
use bevy::prelude::*;

use super::assets::{FrameStore, LoaderConfig};
use super::{rss_mb, SequenceMeshDisplay, SPAWNS_PER_FRAME};
use crate::app::ui::state::RenderConfig;
use crate::lib::memory_budget::MemoryBudget;
use crate::lib::sequence::{compute_spawn_windows, Timeline};

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
pub(super) fn manage_frame_window(
    mut commands: Commands,
    mut frame_store: ResMut<FrameStore>,
    loader_config: Res<LoaderConfig>,
    timeline: Res<Timeline>,
    render_config: Res<RenderConfig>,
    mut visibility_query: Query<&mut Visibility, With<SequenceMeshDisplay>>,
    mut prev_windows: Local<Vec<(usize, usize)>>,
    mut memory_budget: ResMut<MemoryBudget>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    if frame_store.total_frames == 0 {
        return;
    }
    // Need shared material before we can spawn anything
    let Some(material) = loader_config.shared_material.clone() else {
        return;
    };

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

    let transform = loader_config.coord_transform;
    let frame_bytes = frame_store.estimated_frame_bytes;

    // Use the pure delta function to decide what to despawn/spawn.
    let delta = compute_window_delta(&prev_windows, &new_windows);

    // 1. Despawn entities that were in old windows but are NOT in new windows.
    //    Despawns are NOT throttled — freeing memory is always welcome.
    //    We also demote the mesh back to MAIN_WORLD only so Bevy's render
    //    asset system releases the GPU buffer.
    let rss_before_despawn = rss_mb();
    let mut despawns = 0usize;
    let mut demote_ok = 0usize;
    let mut demote_fail = 0usize;
    for idx in &delta.to_despawn {
        let idx = *idx;
        if let Some(entities) = frame_store.frame_entities[idx].take() {
            for entity in &entities {
                if let Ok(mut vis) = visibility_query.get_mut(*entity) {
                    *vis = Visibility::Hidden;
                }
                commands.entity(*entity).despawn();
            }
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
            if frame_bytes > 0 {
                memory_budget.record_frame_unloaded(frame_bytes);
            }
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

    // 2. Spawn entities that are in new windows but were NOT in old windows.
    //    Capped at SPAWNS_PER_FRAME to spread GPU upload work.
    let mut spawns_this_frame = 0usize;
    for &idx in &delta.to_spawn {
        if spawns_this_frame >= SPAWNS_PER_FRAME {
            break;
        }
        if frame_store.frame_entities[idx].is_some() {
            continue; // already spawned
        }
        if !frame_store.is_frame_loaded(idx) {
            continue; // asset not ready yet
        }
        let Some(mesh_handles) = frame_store.get_frame_meshes(idx) else {
            continue;
        };
        let mesh_handles: Vec<Handle<Mesh>> = mesh_handles.to_vec();

        frame_store.promote_to_gpu(idx, &mut meshes);

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

        frame_store.frame_entities[idx] = Some(entities);

        if frame_bytes > 0 {
            memory_budget.record_frame_loaded(frame_bytes);
        }

        spawns_this_frame += 1;
    }

    // Log window changes
    if new_windows != *prev_windows {
        // Count active handles — gltf_handles lives in AssetIndex which we
        // don't have here.  Just count frame_handles for the log line.
        let active_handles = frame_store
            .frame_handles
            .iter()
            .filter(|fh| fh.is_some())
            .count();
        let live_entities = frame_store
            .frame_entities
            .iter()
            .filter(|e| e.is_some())
            .count();
        debug!(
            "Frame windows changed: {:?} → {:?}, current frame: {} | handles: {}, entities: {}, RSS: {:.1} MB",
            *prev_windows, new_windows, current,
            active_handles, live_entities, rss_mb(),
        );
    }

    // 3. Set visibility on all entities across all windows.
    //    Only the current frame is Visible; everything else is Hidden.
    for &(win_lo, win_hi) in &new_windows {
        for idx in win_lo..win_hi {
            if let Some(Some(entities)) = frame_store.frame_entities.get(idx) {
                let desired = if idx == current {
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
        }
    }

    frame_store.displayed_frame = Some(current);
    *prev_windows = new_windows;
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
        let windows = vec![(5, 10)];
        let delta = compute_window_delta(&windows, &windows);
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
        let delta = compute_window_delta(&[], &[(0, 3)]);
        assert!(delta.to_despawn.is_empty());
        assert_eq!(delta.to_spawn, vec![0, 1, 2]);
    }

    #[test]
    fn test_window_delta_some_to_empty() {
        let delta = compute_window_delta(&[(0, 3)], &[]);
        assert_eq!(delta.to_despawn, vec![0, 1, 2]);
        assert!(delta.to_spawn.is_empty());
    }

    #[test]
    fn test_window_delta_multi_window() {
        let old = vec![(0, 5), (10, 15)];
        let new = vec![(3, 8), (10, 15)];
        let delta = compute_window_delta(&old, &new);
        // 0,1,2 leave; 5,6,7 enter; 10..15 unchanged
        assert_eq!(delta.to_despawn, vec![0, 1, 2]);
        assert_eq!(delta.to_spawn, vec![5, 6, 7]);
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
