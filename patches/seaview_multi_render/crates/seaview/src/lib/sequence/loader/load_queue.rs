//! Throttled load queue and section reconciliation.
//!
//! - **[`sync_section_loads`]** — watches for [`Timeline`] section changes and
//!   reconciles the set of loaded frame assets to match across all visible
//!   series.
//! - **[`drain_load_queue`]** — pops at most [`LOADS_PER_FRAME`] items per
//!   tick and issues `asset_server.load()` calls.
//!
//! The union of load sections across all **visible** series determines which
//! frames are resident in memory at any given time.

use bevy::mesh::Mesh;
use bevy::prelude::*;
use std::collections::{HashSet, VecDeque};

use super::assets::{AssetIndex, FrameStore};
use super::mesh_cache::{CacheEntry, MeshCache};
use super::{load_mesh_handle, rss_mb, LOADS_PER_FRAME};
use crate::lib::sequence::{frames_to_load, union_frames_to_load, SeriesManagerRes, Timeline};

// ---------------------------------------------------------------------------
// LoadQueue
// ---------------------------------------------------------------------------

/// A throttled queue of frame indices waiting to be sent to the asset server.
#[derive(Resource, Default)]
pub struct LoadQueue {
    pub(super) queue: VecDeque<usize>,
    pub(super) pending: HashSet<usize>,
}

impl LoadQueue {
    pub(crate) fn push(&mut self, idx: usize) {
        if self.pending.insert(idx) {
            self.queue.push_back(idx);
        }
    }

    pub fn len(&self) -> usize {
        self.queue.len()
    }

    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    pub(crate) fn clear(&mut self) {
        self.queue.clear();
        self.pending.clear();
    }
}

// ---------------------------------------------------------------------------
// Pure functions
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SectionReconciliation {
    pub to_load: Vec<usize>,
    pub to_unload: Vec<usize>,
}

pub(crate) fn reconcile_section_loads(
    needed: &[usize],
    total_frames: usize,
    has_handle: impl Fn(usize) -> bool,
) -> SectionReconciliation {
    let mut to_load = Vec::new();
    for &idx in needed {
        if !has_handle(idx) {
            to_load.push(idx);
        }
    }

    let mut is_needed = vec![false; total_frames];
    for &idx in needed {
        if idx < total_frames {
            is_needed[idx] = true;
        }
    }

    let mut to_unload = Vec::new();
    for idx in 0..total_frames {
        if !is_needed[idx] && has_handle(idx) {
            to_unload.push(idx);
        }
    }

    SectionReconciliation { to_load, to_unload }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Watches for timeline section changes and reconciles the set of loaded frame
/// assets across all **visible** series.
///
/// Only runs when the [`Timeline`] resource has changed (Bevy change detection).
/// Uses the union of load sections across all visible series so that frames
/// required by any visible series remain loaded.
pub(super) fn sync_section_loads(
    mut frame_store: ResMut<FrameStore>,
    mut asset_index: ResMut<AssetIndex>,
    timeline: Res<Timeline>,
    mut load_queue: ResMut<LoadQueue>,
    mut prev_sections: Local<Vec<crate::lib::sequence::LoadSection>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut mesh_cache: ResMut<MeshCache>,
    series_mgr: Res<SeriesManagerRes>,
) {
    if frame_store.total_frames == 0 {
        return;
    }
    if timeline.sections == *prev_sections {
        return;
    }
    *prev_sections = timeline.sections.clone();
    let Some(_format) = frame_store.format else {
        return;
    };

    load_queue.clear();

    let rss_before = rss_mb();

    // Compute the UNION of needed frames across all **visible** series.
    // Hidden series do not contribute to the load set — their frames will be
    // unloaded naturally as they leave all active sections.
    let needed = {
        let visible_ids = series_mgr.visible_ids();
        if visible_ids.is_empty() {
            // Nothing visible: fall back to global timeline (avoids empty state
            // on first load before series manager is fully initialised).
            frames_to_load(&timeline.sections, frame_store.total_frames)
        } else if visible_ids.len() == 1 {
            // Single visible series — common case, no dedup overhead.
            let id = visible_ids[0];
            if series_mgr.active_id() == Some(id) {
                // Active series: use the global timeline (authoritative).
                frames_to_load(&timeline.sections, frame_store.total_frames)
            } else {
                match series_mgr.get(id) {
                    Some(state) => frames_to_load(
                        &state.timeline.sections,
                        frame_store.total_frames,
                    ),
                    None => frames_to_load(&timeline.sections, frame_store.total_frames),
                }
            }
        } else {
            // Multiple visible series — build the union.
            let active_id = series_mgr.active_id();
            let all_sections: Vec<(&[crate::lib::sequence::LoadSection], usize)> = visible_ids
                .iter()
                .filter_map(|&id| {
                    if Some(id) == active_id {
                        // Active series uses the global timeline.
                        Some((timeline.sections.as_slice(), frame_store.total_frames))
                    } else {
                        series_mgr.get(id).map(|state| {
                            (state.timeline.sections.as_slice(), frame_store.total_frames)
                        })
                    }
                })
                .collect();
            union_frames_to_load(&all_sections)
        }
    };

    let reconciliation = reconcile_section_loads(&needed, frame_store.total_frames, |idx| {
        frame_store.frame_handles[idx].is_some()
    });

    let mut loads_enqueued = 0usize;
    for idx in &reconciliation.to_load {
        load_queue.push(*idx);
        loads_enqueued += 1;
    }

    let mut unloads = 0usize;
    let mut handles_with_entities = 0usize;
    let mut meshes_removed = 0usize;
    for &idx in &reconciliation.to_unload {
        if frame_store.frame_entities[idx].is_some() {
            handles_with_entities += 1;
        }
        let mesh_ids: Vec<AssetId<Mesh>> = frame_store.frame_handles[idx]
            .as_ref()
            .map(|handles| handles.iter().map(|h| h.id()).collect())
            .unwrap_or_default();
        for id in mesh_ids {
            asset_index.asset_id_to_frame.remove(&id);
            if meshes.remove(id).is_some() {
                meshes_removed += 1;
            }
        }
        if idx < frame_store.frame_paths.len() {
            let path = frame_store.frame_paths[idx].clone();
            mesh_cache.release(&path);
        }
        frame_store.frame_handles[idx] = None;
        if frame_store.loaded_frames[idx] {
            frame_store.loaded_frames[idx] = false;
            frame_store.loaded_count = frame_store.loaded_count.saturating_sub(1);
        }
        unloads += 1;
    }

    #[cfg(feature = "mesh_unload_malloc_trim")]
    if unloads > 0 {
        #[cfg(target_os = "linux")]
        unsafe {
            libc::malloc_trim(0);
        }
    }

    let rss_after = rss_mb();

    if loads_enqueued > 0 || unloads > 0 {
        frame_store.loading = true;
        debug!(
            "Section sync: {} loads enqueued, {} handles dropped ({} had live entities, {} meshes removed), queue len: {} | RSS {:.1} → {:.1} MB (Δ {:.1})",
            loads_enqueued,
            unloads,
            handles_with_entities,
            meshes_removed,
            load_queue.len(),
            rss_before,
            rss_after,
            rss_after - rss_before,
        );
    }
}

/// Drains up to [`LOADS_PER_FRAME`] items from the [`LoadQueue`], issuing
/// `asset_server.load()` for each.
pub(super) fn drain_load_queue(
    mut load_queue: ResMut<LoadQueue>,
    mut frame_store: ResMut<FrameStore>,
    mut asset_index: ResMut<AssetIndex>,
    asset_server: Res<AssetServer>,
    mut mesh_cache: ResMut<MeshCache>,
) {
    if load_queue.is_empty() {
        return;
    }
    if frame_store.format.is_none() {
        return;
    }

    let mut loaded_this_frame = 0usize;

    while let Some(idx) = load_queue.queue.pop_front() {
        load_queue.pending.remove(&idx);
        if loaded_this_frame >= LOADS_PER_FRAME {
            break;
        }

        if frame_store.frame_handles[idx].is_some() {
            continue;
        }

        if idx >= frame_store.frame_paths.len() {
            continue;
        }

        let filename = frame_store.frame_paths[idx]
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_owned();

        let handle = load_mesh_handle(&asset_server, &filename);
        asset_index.asset_id_to_frame.insert(handle.id(), idx);
        frame_store.frame_handles[idx] = Some(vec![handle.clone()]);

        let path = frame_store.frame_paths[idx].clone();
        if mesh_cache.contains(&path) {
            if let Some(entry) = mesh_cache.get_mut(&path) {
                entry.add_ref();
            }
        } else {
            mesh_cache.insert(path, CacheEntry::new(vec![handle]));
        }

        loaded_this_frame += 1;
        trace!("Kicked load for frame {} ('{}')", idx, filename);
    }

    if loaded_this_frame > 0 {
        debug!(
            "drain_load_queue: issued {} loads, {} remaining in queue",
            loaded_this_frame,
            load_queue.len(),
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_reconcile_empty_needed() {
        let r = reconcile_section_loads(&[], 5, |_| false);
        assert!(r.to_load.is_empty());
        assert!(r.to_unload.is_empty());
    }

    #[test]
    fn test_reconcile_all_needed_none_loaded() {
        let needed = vec![0, 1, 2];
        let r = reconcile_section_loads(&needed, 5, |_| false);
        assert_eq!(r.to_load, vec![0, 1, 2]);
        assert!(r.to_unload.is_empty());
    }

    #[test]
    fn test_reconcile_all_needed_all_loaded() {
        let needed = vec![0, 1, 2];
        let r = reconcile_section_loads(&needed, 5, |_| true);
        assert!(r.to_load.is_empty());
        assert_eq!(r.to_unload, vec![3, 4]);
    }

    #[test]
    fn test_reconcile_partial_overlap() {
        let needed = vec![2, 3, 4];
        let has_handle = |idx: usize| idx >= 1 && idx <= 3;
        let r = reconcile_section_loads(&needed, 5, has_handle);
        assert_eq!(r.to_load, vec![4]);
        assert_eq!(r.to_unload, vec![1]);
    }

    #[test]
    fn test_reconcile_zero_total_frames() {
        let r = reconcile_section_loads(&[], 0, |_| false);
        assert!(r.to_load.is_empty());
        assert!(r.to_unload.is_empty());
    }

    #[test]
    fn test_reconcile_needed_subset_of_loaded() {
        let needed = vec![2, 3, 4];
        let r = reconcile_section_loads(&needed, 10, |_| true);
        assert!(r.to_load.is_empty());
        assert_eq!(r.to_unload, vec![0, 1, 5, 6, 7, 8, 9]);
    }
}
