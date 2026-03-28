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
use super::series_frame_store::SeriesFrameStore;
use super::{load_mesh_handle, rss_mb, SequenceKind, LOADS_PER_FRAME};
#[cfg(feature = "meshlet")]
use super::load_meshlet_handle;
use crate::lib::sequence::{frames_to_load, SeriesManagerRes, Timeline};
use crate::lib::series::SeriesId;

// ---------------------------------------------------------------------------
// LoadQueue
// ---------------------------------------------------------------------------

/// A throttled queue of (series_id, frame_index) pairs waiting to be sent
/// to the asset server.
#[derive(Resource, Default)]
pub struct LoadQueue {
    pub(super) queue: VecDeque<(SeriesId, usize)>,
    pub(super) pending: HashSet<(SeriesId, usize)>,
}

impl LoadQueue {
    /// Push a frame for the active (legacy) series path.
    pub(crate) fn push(&mut self, idx: usize) {
        // Sentinel SeriesId(u64::MAX) used for the active-series path
        // so that drain_load_queue falls back to FrameStore.
        self.push_series(idx, SeriesId::from_raw(u64::MAX));
    }

    /// Push a frame for a specific series.
    pub(crate) fn push_series(&mut self, idx: usize, series_id: SeriesId) {
        let key = (series_id, idx);
        if self.pending.insert(key) {
            self.queue.push_back(key);
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
    mut series_fs: ResMut<SeriesFrameStore>,
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

    let active_id = series_mgr.active_id();
    let visible_ids = series_mgr.visible_ids();

    // ── Per-series reconciliation ─────────────────────────────────────
    //
    // Each visible series is reconciled independently against its own
    // SeriesFrameStore state so that non-active series' loads are correctly
    // enqueued with the right series ID.

    let mut loads_enqueued = 0usize;
    let mut unloads = 0usize;
    let mut handles_with_entities = 0usize;
    let mut meshes_removed = 0usize;

    for &sid in &visible_ids {
        let is_active = Some(sid) == active_id;

        // Determine which frames this series needs loaded.
        //
        // Non-active series inherit the active timeline's sections
        // (proportionally mapped to their own frame count) so they
        // respect the visible region the user has set, rather than
        // loading their full default [0, N) range.
        let (sections, total_frames) = if is_active {
            (timeline.sections.clone(), frame_store.total_frames)
        } else {
            match series_mgr.get(sid) {
                Some(state) => {
                    let tf = state.timeline.total_frames;
                    let mapped = super::map_sections_proportional(
                        &timeline.sections,
                        timeline.total_frames,
                        tf,
                    );
                    if mapped.is_empty() {
                        (state.timeline.sections.clone(), tf)
                    } else {
                        (mapped, tf)
                    }
                }
                None => continue,
            }
        };

        if total_frames == 0 {
            continue;
        }

        let needed = frames_to_load(&sections, total_frames);

        if is_active {
            // Active series: reconcile against FrameStore (the authoritative
            // mirror for the active series).
            let reconciliation = reconcile_section_loads(&needed, total_frames, |idx| {
                frame_store.frame_handles.get(idx).map(|h| h.is_some()).unwrap_or(false)
            });

            for idx in &reconciliation.to_load {
                load_queue.push(*idx);
                loads_enqueued += 1;
            }

            for &idx in &reconciliation.to_unload {
                if frame_store.frame_entities.get(idx).map(|e| e.is_some()).unwrap_or(false) {
                    handles_with_entities += 1;
                }
                let mesh_ids: Vec<AssetId<Mesh>> = frame_store.frame_handles.get(idx)
                    .and_then(|h| h.as_ref())
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
                if idx < frame_store.frame_handles.len() {
                    frame_store.frame_handles[idx] = None;
                }
                if idx < frame_store.loaded_frames.len() && frame_store.loaded_frames[idx] {
                    frame_store.loaded_frames[idx] = false;
                    frame_store.loaded_count = frame_store.loaded_count.saturating_sub(1);
                }

                // Also clear from SeriesFrameStore mirror.
                if let Some(state) = series_fs.get_mut(&sid) {
                    if idx < state.frame_handles.len() {
                        state.frame_handles[idx] = None;
                    }
                    if idx < state.loaded_frames.len() && state.loaded_frames[idx] {
                        state.loaded_frames[idx] = false;
                        state.loaded_count = state.loaded_count.saturating_sub(1);
                    }
                }

                unloads += 1;
            }
        } else {
            // Non-active series: reconcile against SeriesFrameStore.
            let has_handle = |idx: usize| -> bool {
                series_fs
                    .get(&sid)
                    .map(|s| s.has_handle(idx))
                    .unwrap_or(false)
            };
            let reconciliation = reconcile_section_loads(&needed, total_frames, has_handle);

            for idx in &reconciliation.to_load {
                load_queue.push_series(*idx, sid);
                loads_enqueued += 1;
            }

            for &idx in &reconciliation.to_unload {
                if let Some(state) = series_fs.get_mut(&sid) {
                    // Remove mesh assets.
                    let mesh_ids: Vec<AssetId<Mesh>> = state.frame_handles.get(idx)
                        .and_then(|h| h.as_ref())
                        .map(|handles| handles.iter().map(|h| h.id()).collect())
                        .unwrap_or_default();
                    for id in mesh_ids {
                        asset_index.asset_id_to_frame.remove(&id);
                        if meshes.remove(id).is_some() {
                            meshes_removed += 1;
                        }
                    }
                    if idx < state.frame_paths.len() {
                        let path = state.frame_paths[idx].clone();
                        mesh_cache.release(&path);
                    }
                    if idx < state.frame_handles.len() {
                        state.frame_handles[idx] = None;
                    }
                    if idx < state.loaded_frames.len() && state.loaded_frames[idx] {
                        state.loaded_frames[idx] = false;
                        state.loaded_count = state.loaded_count.saturating_sub(1);
                    }
                }
                unloads += 1;
            }
        }
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
///
/// Each queue item carries a `(SeriesId, frame_index)` pair.  Handles are
/// written into the appropriate [`SeriesEntityState`] in [`SeriesFrameStore`]
/// and mirrored into [`FrameStore`] for the active series.
pub(super) fn drain_load_queue(
    mut load_queue: ResMut<LoadQueue>,
    mut frame_store: ResMut<FrameStore>,
    mut asset_index: ResMut<AssetIndex>,
    asset_server: Res<AssetServer>,
    mut mesh_cache: ResMut<MeshCache>,
    mut series_fs: ResMut<SeriesFrameStore>,
    series_mgr: Res<SeriesManagerRes>,
) {
    if load_queue.is_empty() {
        return;
    }

    let active_sentinel = SeriesId::from_raw(u64::MAX);
    let active_id = series_mgr.active_id();

    let mut loaded_this_frame = 0usize;

    while let Some((series_id, idx)) = load_queue.queue.pop_front() {
        load_queue.pending.remove(&(series_id, idx));
        if loaded_this_frame >= LOADS_PER_FRAME {
            break;
        }

        // Resolve whether this is active-series (sentinel) or a real series.
        let is_active_path = series_id == active_sentinel;

        if is_active_path {
            // Legacy active-series path — check FrameStore.
            if frame_store.format.is_none() {
                continue;
            }
            if frame_store.frame_handles.get(idx).map(|h| h.is_some()).unwrap_or(false) {
                continue;
            }
            if idx >= frame_store.frame_paths.len() {
                continue;
            }

            let path = frame_store.frame_paths[idx].clone();

            #[cfg(feature = "meshlet")]
            if frame_store.format == Some(SequenceKind::Meshlet) {
                let handle = load_meshlet_handle(&asset_server, &path);
                if let Some(v) = frame_store.meshlet_handles.get_mut(idx) {
                    *v = Some(handle.clone());
                }
                if let Some(aid) = active_id {
                    if let Some(state) = series_fs.get_mut(&aid) {
                        if let Some(v) = state.meshlet_handles.get_mut(idx) {
                            *v = Some(handle);
                        }
                    }
                }
                loaded_this_frame += 1;
                trace!("Kicked meshlet load for series {:?} frame {}", active_sentinel, idx);
                continue;
            }

            let handle = load_mesh_handle(&asset_server, &path);
            let real_id = active_id.unwrap_or(active_sentinel);
            asset_index.asset_id_to_frame.insert(handle.id(), (real_id, idx));
            frame_store.frame_handles[idx] = Some(vec![handle.clone()]);

            // Mirror into SeriesFrameStore for the active series.
            if let Some(aid) = active_id {
                if let Some(state) = series_fs.get_mut(&aid) {
                    if idx < state.frame_handles.len() {
                        state.frame_handles[idx] = Some(vec![handle.clone()]);
                    }
                }
            }

            let path = frame_store.frame_paths[idx].clone();
            if mesh_cache.contains(&path) {
                if let Some(entry) = mesh_cache.get_mut(&path) {
                    entry.add_ref();
                }
            } else {
                mesh_cache.insert(path, CacheEntry::new(vec![handle]));
            }
        } else {
            // Per-series path — read from SeriesFrameStore.
            let Some(state) = series_fs.get_mut(&series_id) else {
                continue;
            };
            if state.format.is_none() {
                continue;
            }
            if state.frame_handles.get(idx).map(|h| h.is_some()).unwrap_or(false) {
                continue;
            }
            if idx >= state.frame_paths.len() {
                continue;
            }

            let path = state.frame_paths[idx].clone();

            #[cfg(feature = "meshlet")]
            if state.format == Some(SequenceKind::Meshlet) {
                let handle = load_meshlet_handle(&asset_server, &path);
                if let Some(v) = state.meshlet_handles.get_mut(idx) {
                    *v = Some(handle);
                }
                loaded_this_frame += 1;
                trace!("Kicked meshlet load for series {:?} frame {}", series_id, idx);
                continue;
            }

            let handle = load_mesh_handle(&asset_server, &path);
            asset_index.asset_id_to_frame.insert(handle.id(), (series_id, idx));
            state.frame_handles[idx] = Some(vec![handle.clone()]);

            if mesh_cache.contains(&path) {
                if let Some(entry) = mesh_cache.get_mut(&path) {
                    entry.add_ref();
                }
            } else {
                mesh_cache.insert(path, CacheEntry::new(vec![handle]));
            }
        }

        loaded_this_frame += 1;
        trace!("Kicked load for series {:?} frame {}", series_id, idx);
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
