//! Throttled load queue and section reconciliation.
//!
//! This module contains the [`LoadQueue`] resource that buffers frame indices
//! waiting to be sent to the asset server, and two systems that populate and
//! drain it:
//!
//! - **[`sync_section_loads`]** — watches for [`Timeline`] section changes and
//!   reconciles the set of loaded frame assets to match, pushing new indices
//!   into the queue and dropping handles for frames that left all sections.
//! - **[`drain_load_queue`]** — pops at most [`LOADS_PER_FRAME`](super::LOADS_PER_FRAME)
//!   items per tick and issues the actual `asset_server.load()` calls.
//!
//! The pure function [`reconcile_section_loads`] is extracted for testability.

use bevy::mesh::Mesh;
use bevy::prelude::*;
use std::collections::{HashSet, VecDeque};

use super::assets::{AssetIndex, FrameStore, MeshFileFormat};
use super::{load_direct_mesh_handle, load_gltf_handle, rss_mb, LOADS_PER_FRAME};
use crate::lib::sequence::{frames_to_load, Timeline};

// ---------------------------------------------------------------------------
// LoadQueue
// ---------------------------------------------------------------------------

/// A throttled queue of frame indices waiting to be sent to the asset server.
///
/// Systems push indices here instead of calling `asset_server.load()` directly.
/// [`drain_load_queue`] pops at most [`LOADS_PER_FRAME`](super::LOADS_PER_FRAME)
/// per tick.
#[derive(Resource, Default)]
pub struct LoadQueue {
    pub(super) queue: VecDeque<usize>,
    /// Tracks indices already in the queue to prevent duplicate enqueues.
    pub(super) pending: HashSet<usize>,
}

impl LoadQueue {
    /// Push a frame index to be loaded (deduplicates automatically).
    pub(crate) fn push(&mut self, idx: usize) {
        if self.pending.insert(idx) {
            self.queue.push_back(idx);
        }
    }

    /// How many items are waiting.
    pub fn len(&self) -> usize {
        self.queue.len()
    }

    /// Whether the queue is empty.
    pub fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    /// Clear all pending loads (used when resetting for a new sequence).
    pub(crate) fn clear(&mut self) {
        self.queue.clear();
        self.pending.clear();
    }
}

// ---------------------------------------------------------------------------
// Pure functions
// ---------------------------------------------------------------------------

/// Result of reconciling the set of loaded frames against the current sections.
///
/// Produced by [`reconcile_section_loads`], consumed by [`sync_section_loads`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct SectionReconciliation {
    /// Frame indices that need to be enqueued for loading (in sections but
    /// no handle yet).
    pub to_load: Vec<usize>,
    /// Frame indices that should be unloaded (have a handle but are outside
    /// all sections).
    pub to_unload: Vec<usize>,
}

/// Pure logic extracted from [`sync_section_loads`].
///
/// Given the set of needed frame indices (from sections) and a per-frame
/// predicate indicating whether a handle already exists, returns which frames
/// to enqueue for loading and which to unload.
///
/// `has_handle` should return `true` when frame `idx` already has either a
/// mesh handle or an intermediate glTF handle.
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
/// assets to match.
///
/// **Only runs when the [`Timeline`] resource has changed** (Bevy change
/// detection), avoiding O(n) work every frame during steady-state playback.
///
/// Frames entering a section are pushed into the [`LoadQueue`]; frames leaving
/// all sections have their handle dropped immediately so the asset server can
/// reclaim memory.
pub(super) fn sync_section_loads(
    mut frame_store: ResMut<FrameStore>,
    mut asset_index: ResMut<AssetIndex>,
    timeline: Res<Timeline>,
    mut load_queue: ResMut<LoadQueue>,
    mut prev_sections: Local<Vec<crate::lib::sequence::LoadSection>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    // Early-out: nothing to do if no sequence loaded.
    if frame_store.total_frames == 0 {
        return;
    }
    // Only re-sync when sections actually changed (not on every playhead tick).
    if timeline.sections == *prev_sections {
        return;
    }
    *prev_sections = timeline.sections.clone();
    let Some(_format) = frame_store.format else {
        return;
    };

    // Clear stale queue entries — frames from previously-active sections
    // would otherwise be picked up by drain_load_queue and re-loaded even
    // though their section is now inactive.
    load_queue.clear();

    let rss_before = rss_mb();

    let needed = frames_to_load(&timeline.sections, frame_store.total_frames);

    // Use the pure reconciliation function to decide what to load/unload.
    let reconciliation = reconcile_section_loads(&needed, frame_store.total_frames, |idx| {
        frame_store.frame_handles[idx].is_some() || asset_index.gltf_handles[idx].is_some()
    });

    let mut loads_enqueued = 0usize;
    for idx in &reconciliation.to_load {
        load_queue.push(*idx);
        loads_enqueued += 1;
    }

    // Unload frames that are outside all sections (drop handle, reclaim memory).
    let mut unloads = 0usize;
    let mut handles_with_entities = 0usize;
    let mut meshes_removed = 0usize;
    for &idx in &reconciliation.to_unload {
        if frame_store.frame_entities[idx].is_some() {
            handles_with_entities += 1;
        }
        // Remove all mesh asset IDs from reverse index and force-remove
        // from Assets<Mesh> immediately.
        // Collect IDs first to avoid borrowing frame_store immutably
        // and mutably at the same time.
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
        // Also remove the intermediate Gltf handle if present.
        if let Some(gltf_handle) = asset_index.gltf_handles[idx].take() {
            asset_index.gltf_id_to_frame.remove(&gltf_handle.id());
        }
        frame_store.frame_handles[idx] = None;
        if frame_store.loaded_frames[idx] {
            frame_store.loaded_frames[idx] = false;
            frame_store.loaded_count = frame_store.loaded_count.saturating_sub(1);
        }
        unloads += 1;
    }

    // After bulk unloads, ask glibc to return freed pages to the OS.
    // Without this, RSS stays high because malloc keeps pages in its arena
    // for reuse rather than returning them via munmap.
    //
    // Gated behind the `mesh_unload_malloc_trim` feature because
    // malloc_trim() can be expensive on highly-fragmented heaps and may
    // not be desirable in all workloads.  Enable with:
    //   cargo build --features mesh_unload_malloc_trim
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
        info!(
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

/// Drains up to [`LOADS_PER_FRAME`](super::LOADS_PER_FRAME) items from the
/// [`LoadQueue`], issuing `asset_server.load()` for each and populating the
/// reverse `AssetId → frame` index for O(1) lookups in
/// [`track_asset_loading`](super::loading_tracker::track_asset_loading).
pub(super) fn drain_load_queue(
    mut load_queue: ResMut<LoadQueue>,
    mut frame_store: ResMut<FrameStore>,
    mut asset_index: ResMut<AssetIndex>,
    asset_server: Res<AssetServer>,
) {
    if load_queue.is_empty() {
        return;
    }
    let Some(format) = frame_store.format else {
        return;
    };

    let mut loaded_this_frame = 0usize;

    while let Some(idx) = load_queue.queue.pop_front() {
        load_queue.pending.remove(&idx);
        if loaded_this_frame >= LOADS_PER_FRAME {
            break;
        }

        // Skip if already has a handle (mesh or intermediate Gltf).
        if frame_store.frame_handles[idx].is_some() || asset_index.gltf_handles[idx].is_some() {
            continue;
        }

        // Guard against out-of-bounds (e.g. sequence was reset).
        if idx >= frame_store.frame_paths.len() {
            continue;
        }

        let filename = frame_store.frame_paths[idx]
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_owned();

        match format {
            MeshFileFormat::Stl | MeshFileFormat::Obj => {
                // Direct mesh load (STL and OBJ both produce Mesh assets directly)
                let handle = load_direct_mesh_handle(&asset_server, &filename);
                // Populate the reverse index for O(1) event lookup.
                asset_index.asset_id_to_frame.insert(handle.id(), idx);
                frame_store.frame_handles[idx] = Some(vec![handle]);
            }
            MeshFileFormat::Gltf | MeshFileFormat::Glb => {
                let gltf_handle = load_gltf_handle(&asset_server, &filename);
                asset_index.gltf_id_to_frame.insert(gltf_handle.id(), idx);
                asset_index.gltf_handles[idx] = Some(gltf_handle);
                // frame_handles[idx] stays None — populated by extract_gltf_meshes
            }
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

    // ── reconcile_section_loads ────────────────────────────────────────

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
        // Frames 3 and 4 have handles but are not needed → unload
        assert_eq!(r.to_unload, vec![3, 4]);
    }

    #[test]
    fn test_reconcile_partial_overlap() {
        // Frames 0..5, needed=[2,3,4], handles at [1,2,3]
        let needed = vec![2, 3, 4];
        let has_handle = |idx: usize| idx >= 1 && idx <= 3;
        let r = reconcile_section_loads(&needed, 5, has_handle);
        // Frame 4 is needed but has no handle → load
        assert_eq!(r.to_load, vec![4]);
        // Frame 1 has a handle but is not needed → unload
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
        // 10 frames, all loaded, only 2-4 needed
        let needed = vec![2, 3, 4];
        let r = reconcile_section_loads(&needed, 10, |_| true);
        assert!(r.to_load.is_empty());
        assert_eq!(r.to_unload, vec![0, 1, 5, 6, 7, 8, 9]);
    }
}
