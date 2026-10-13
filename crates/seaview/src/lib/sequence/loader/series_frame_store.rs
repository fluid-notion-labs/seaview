//! Per-series entity tracking for multi-series playback.
//!
//! [`SeriesFrameStore`] holds the entity-level state that was previously
//! embedded in [`super::assets::FrameStore`] — spawned entities, displayed
//! frame, and the previous spawn-window state used by
//! [`super::frame_window::manage_frame_window`].
//!
//! ## Motivation
//!
//! With a single global `FrameStore`, only one series can have spawned
//! entities at a time.  `SeriesFrameStore` maps each [`SeriesId`] to its
//! own [`SeriesEntityState`], enabling simultaneous entity management
//! across multiple series once union loading (Phase 3) is in place.
//!
//! ## Phase 2 (current)
//!
//! `manage_frame_window` operates on the active series only — behaviour is
//! unchanged from before.  The per-series structure is in place so that
//! Phase 3 can iterate all visible series without further restructuring.
//!
//! ## Phase 3 (future)
//!
//! All visible series will have their frame windows computed and entities
//! spawned/despawned independently.  GPU residency will be managed via
//! [`MeshCache::gpu_consumers`](super::mesh_cache::CacheEntry::gpu_consumers)
//! refcounting.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::Mesh;
use bevy::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;

use crate::lib::series::SeriesId;
use crate::lib::sequence::loader::SequenceKind;

// ---------------------------------------------------------------------------
// SeriesEntityState — per-series entity + window tracking + asset handles
// ---------------------------------------------------------------------------

/// Full per-series state: asset handles, loading progress, entities, and
/// display tracking.
///
/// Previously asset handles lived exclusively in the global `FrameStore`.
/// Moving them here makes each series self-contained so that multiple series
/// can load, track, and display their own distinct meshes simultaneously.
#[derive(Debug)]
pub struct SeriesEntityState {
    // ── Asset / loading ──────────────────────────────────────────────

    /// Mesh handles per frame (`None` = not yet requested from asset server).
    /// Each frame may contain multiple meshes (e.g. multi-primitive GLB).
    pub frame_handles: Vec<Option<Vec<Handle<Mesh>>>>,

    /// On-disk paths for every frame (needed for `asset_server.load()`).
    pub frame_paths: Vec<PathBuf>,

    /// Which frames have finished loading (asset ready for spawn).
    pub loaded_frames: Vec<bool>,

    /// Number of frames that have finished loading.
    pub loaded_count: usize,

    /// Total frames in this series.
    pub total_frames: usize,

    /// Detected sequence kind (set when `LoadSequenceRequest` is processed).
    pub format: Option<SequenceKind>,

    /// Frames flagged as degenerate (near-zero geometry — skipped during display).
    pub degenerate_frames: Vec<bool>,

    /// Estimated GPU bytes per frame (set after first frame loads).
    pub estimated_frame_bytes: u64,

    // ── Entity / display ─────────────────────────────────────────────

    /// Spawned entities per frame index (`None` = not currently spawned).
    /// Multi-mesh frames have multiple entities per frame.
    ///
    /// Indexed by frame number.  Length matches the series' total frame count.
    pub frame_entities: Vec<Option<Vec<Entity>>>,

    /// Currently displayed (visible) frame index for this series.
    pub displayed_frame: Option<usize>,

    /// Previous spawn windows — tracked per-series so that
    /// [`compute_window_delta`](super::frame_window::compute_window_delta)
    /// can compute the correct delta for each series independently.
    pub prev_windows: Vec<(usize, usize)>,
    /// Meshlet handles — populated instead of `frame_handles` for `.meshlet` sequences.
    ///
    /// # Feature seam
    /// Only exists with `--features meshlet`. `SeriesEntityState` is constructed
    /// explicitly in `new()` (not via `Default`), so any new construction sites
    /// must include a `#[cfg(feature = "meshlet")] meshlet_handles: vec![None; n]`
    /// arm to avoid a compile error when the feature is active.
    #[cfg(feature = "meshlet")]
    pub meshlet_handles: Vec<Option<Handle<bevy::pbr::experimental::meshlet::MeshletMesh>>>,
}

impl SeriesEntityState {
    /// Create a new entity state for a series with `total_frames` frames and
    /// the given on-disk paths.
    pub fn new(frame_paths: Vec<PathBuf>, format: Option<SequenceKind>) -> Self {
        let total_frames = frame_paths.len();
        Self {
            frame_handles: vec![None; total_frames],
            frame_paths,
            loaded_frames: vec![false; total_frames],
            loaded_count: 0,
            total_frames,
            format,
            degenerate_frames: vec![false; total_frames],
            estimated_frame_bytes: 0,
            frame_entities: vec![None; total_frames],
            displayed_frame: None,
            prev_windows: Vec::new(),
            #[cfg(feature = "meshlet")]
            meshlet_handles: vec![None; total_frames],
        }
    }

    /// Reset for reload — clears asset + entity state but keeps paths/format.
    pub fn reset_for_reload(&mut self) {
        self.frame_handles.iter_mut().for_each(|h| *h = None);
        self.loaded_frames.iter_mut().for_each(|l| *l = false);
        self.loaded_count = 0;
        self.degenerate_frames.iter_mut().for_each(|d| *d = false);
        self.estimated_frame_bytes = 0;
        self.frame_entities.iter_mut().for_each(|e| *e = None);
        self.displayed_frame = None;
        self.prev_windows.clear();
        #[cfg(feature = "meshlet")]
        self.meshlet_handles.iter_mut().for_each(|h| *h = None);
    }

    // ── Asset helpers ────────────────────────────────────────────────

    /// Get all mesh handles for a specific frame.
    pub fn get_frame_meshes(&self, index: usize) -> Option<&[Handle<Mesh>]> {
        self.frame_handles
            .get(index)
            .and_then(|opt| opt.as_ref())
            .map(|v| v.as_slice())
    }

    /// Whether a frame has a handle (requested from asset server).
    pub fn has_handle(&self, index: usize) -> bool {
        self.frame_handles
            .get(index)
            .map(|h| h.is_some())
            .unwrap_or(false)
    }

    /// Whether a frame has finished loading.
    pub fn is_frame_loaded(&self, index: usize) -> bool {
        self.loaded_frames.get(index).copied().unwrap_or(false)
    }

    /// Whether this frame is degenerate.
    pub fn is_degenerate(&self, index: usize) -> bool {
        self.degenerate_frames.get(index).copied().unwrap_or(false)
    }

    /// Promote a frame's mesh to GPU (adds RENDER_WORLD flag).
    pub fn promote_to_gpu(&self, idx: usize, meshes: &mut Assets<Mesh>) -> bool {
        let Some(handles) = self.get_frame_meshes(idx) else {
            return false;
        };
        let mut any = false;
        for handle in handles {
            if let Some(mut mesh) = meshes.get_mut(handle) {
                mesh.asset_usage =
                    RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD;
                any = true;
            }
        }
        any
    }

    /// Demote a frame's mesh from GPU (removes RENDER_WORLD flag).
    pub fn demote_from_gpu(&self, idx: usize, meshes: &mut Assets<Mesh>) -> bool {
        let Some(handles) = self.get_frame_meshes(idx) else {
            return false;
        };
        let mut any = false;
        for handle in handles {
            if let Some(mut mesh) = meshes.get_mut(handle) {
                mesh.asset_usage = RenderAssetUsages::MAIN_WORLD;
                any = true;
            }
        }
        any
    }

    /// Grow asset/entity vecs if the series gained frames (watch mode).
    pub fn grow_to(&mut self, new_total: usize) {
        if new_total > self.total_frames {
            self.frame_handles.resize_with(new_total, || None);
            self.loaded_frames.resize(new_total, false);
            self.degenerate_frames.resize(new_total, false);
            self.frame_entities.resize_with(new_total, || None);
            self.total_frames = new_total;
        }
    }

    // ── Entity helpers ───────────────────────────────────────────────

    /// Reset all entity tracking (e.g. when reloading).
    ///
    /// Does NOT despawn entities — the caller must do that before calling
    /// reset (typically via `take_all_entities`).
    pub fn reset(&mut self) {
        self.frame_entities.iter_mut().for_each(|e| *e = None);
        self.displayed_frame = None;
        self.prev_windows.clear();
    }

    /// Whether any entities are currently spawned for this series.
    pub fn has_spawned_entities(&self) -> bool {
        self.frame_entities.iter().any(|e| e.is_some())
    }

    /// Count how many frames have spawned entities.
    pub fn spawned_count(&self) -> usize {
        self.frame_entities.iter().filter(|e| e.is_some()).count()
    }

    /// Get the entities for a specific frame.
    pub fn get_entities(&self, frame: usize) -> Option<&[Entity]> {
        self.frame_entities
            .get(frame)
            .and_then(|opt| opt.as_ref())
            .map(|v| v.as_slice())
    }

    /// Set the entities for a specific frame.
    pub fn set_entities(&mut self, frame: usize, entities: Vec<Entity>) {
        if frame < self.frame_entities.len() {
            self.frame_entities[frame] = Some(entities);
        }
    }

    /// Take (remove) the entities for a specific frame, returning them.
    pub fn take_entities(&mut self, frame: usize) -> Option<Vec<Entity>> {
        if frame < self.frame_entities.len() {
            self.frame_entities[frame].take()
        } else {
            None
        }
    }

    /// Take ALL spawned entities across all frames, returning them as a flat
    /// list.  Useful for bulk despawn when a series is removed or reloaded.
    pub fn take_all_entities(&mut self) -> Vec<Entity> {
        let mut all = Vec::new();
        for slot in &mut self.frame_entities {
            if let Some(entities) = slot.take() {
                all.extend(entities);
            }
        }
        self.displayed_frame = None;
        self.prev_windows.clear();
        all
    }

    /// Whether a frame index has spawned entities.
    pub fn is_spawned(&self, frame: usize) -> bool {
        self.frame_entities
            .get(frame)
            .and_then(|e| e.as_ref())
            .is_some()
    }

}

// ---------------------------------------------------------------------------
// SeriesFrameStore — maps SeriesId → SeriesEntityState
// ---------------------------------------------------------------------------

/// Per-series entity tracking resource.
///
/// Owns a [`SeriesEntityState`] for each series that has been registered.
/// Systems use this instead of the global `FrameStore.frame_entities` to
/// manage spawned entities per-series.
#[derive(Resource, Default)]
pub struct SeriesFrameStore {
    states: HashMap<SeriesId, SeriesEntityState>,
}

impl SeriesFrameStore {
    /// Register a new series with its frame paths and format.
    ///
    /// If the series already exists, this is a no-op (returns `false`).
    pub fn register(
        &mut self,
        id: SeriesId,
        frame_paths: Vec<PathBuf>,
        format: Option<SequenceKind>,
    ) -> bool {
        if self.states.contains_key(&id) {
            return false;
        }
        self.states
            .insert(id, SeriesEntityState::new(frame_paths, format));
        true
    }

    /// Remove a series, returning all of its spawned entities (for despawn).
    pub fn remove(&mut self, id: &SeriesId) -> Vec<Entity> {
        if let Some(mut state) = self.states.remove(id) {
            state.take_all_entities()
        } else {
            Vec::new()
        }
    }

    /// Get the entity state for a series.
    pub fn get(&self, id: &SeriesId) -> Option<&SeriesEntityState> {
        self.states.get(id)
    }

    /// Get the entity state for a series mutably.
    pub fn get_mut(&mut self, id: &SeriesId) -> Option<&mut SeriesEntityState> {
        self.states.get_mut(id)
    }

    /// Check whether a series is registered.
    pub fn contains(&self, id: &SeriesId) -> bool {
        self.states.contains_key(id)
    }

    /// Number of registered series.
    pub fn len(&self) -> usize {
        self.states.len()
    }

    /// Whether no series are registered.
    pub fn is_empty(&self) -> bool {
        self.states.is_empty()
    }

    /// Iterate over all (id, state) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&SeriesId, &SeriesEntityState)> {
        self.states.iter()
    }

    /// Iterate mutably over all (id, state) pairs.
    pub fn iter_mut(&mut self) -> impl Iterator<Item = (&SeriesId, &mut SeriesEntityState)> {
        self.states.iter_mut()
    }

    /// Total number of spawned entities across all series.
    pub fn total_spawned_entities(&self) -> usize {
        self.states.values().map(|s| s.spawned_count()).sum()
    }

    /// Collect all spawned entities across all series (flat list).
    /// Does NOT remove them — use `remove` or `take_all_entities` for that.
    pub fn all_entities(&self) -> Vec<Entity> {
        let mut out = Vec::new();
        for state in self.states.values() {
            for slot in &state.frame_entities {
                if let Some(entities) = slot {
                    out.extend(entities.iter().copied());
                }
            }
        }
        out
    }

    /// Reset the store completely (e.g. when loading a new project).
    ///
    /// Returns all entities that were spawned (caller must despawn them).
    pub fn reset(&mut self) -> Vec<Entity> {
        let mut all = Vec::new();
        for (_, state) in self.states.iter_mut() {
            all.extend(state.take_all_entities());
        }
        self.states.clear();
        all
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn sid(n: u64) -> SeriesId {
        SeriesId::from_raw(n)
    }

    /// Create a fake entity for testing.
    ///
    /// In unit tests we can't create real Bevy entities, but `Entity` can be
    /// constructed from bits.  These entities won't be valid in any ECS
    /// world but are fine for tracking logic.
    fn fake_entity(index: u32) -> Entity {
        // Encode as index in the low bits with generation 0.
        // Entity::from_bits layout: high 32 = generation, low 32 = !index
        Entity::from_bits(!(index as u64) & 0xFFFF_FFFF)
    }

    /// Build a `SeriesEntityState` with `n` dummy frame slots for testing.
    fn test_state(n: usize) -> SeriesEntityState {
        let paths: Vec<std::path::PathBuf> = (0..n)
            .map(|i| std::path::PathBuf::from(format!("/tmp/frame_{:04}.stl", i)))
            .collect();
        SeriesEntityState::new(paths, None)
    }

    /// Register a series with `n` dummy slots.
    fn reg(store: &mut SeriesFrameStore, id: SeriesId, n: usize) -> bool {
        let paths: Vec<std::path::PathBuf> = (0..n)
            .map(|i| std::path::PathBuf::from(format!("/tmp/s{}frame_{:04}.stl", id.raw(), i)))
            .collect();
        store.register(id, paths, None)
    }

    // ── SeriesEntityState ─────────────────────────────────────────────

    #[test]
    fn entity_state_new() {
        let state = test_state(10);
        assert_eq!(state.total_frames, 10);
        assert_eq!(state.frame_entities.len(), 10);
        assert_eq!(state.frame_handles.len(), 10);
        assert!(state.displayed_frame.is_none());
        assert!(state.prev_windows.is_empty());
        assert!(!state.has_spawned_entities());
        assert_eq!(state.spawned_count(), 0);
    }

    #[test]
    fn entity_state_set_and_get() {
        let mut state = test_state(5);
        let e1 = fake_entity(1);
        let e2 = fake_entity(2);

        state.set_entities(2, vec![e1, e2]);

        assert!(state.is_spawned(2));
        assert!(!state.is_spawned(0));
        assert_eq!(state.get_entities(2).unwrap().len(), 2);
        assert!(state.get_entities(0).is_none());
        assert!(state.has_spawned_entities());
        assert_eq!(state.spawned_count(), 1);
    }

    #[test]
    fn entity_state_set_out_of_bounds_is_noop() {
        let mut state = test_state(3);
        state.set_entities(10, vec![fake_entity(1)]);
        assert!(!state.has_spawned_entities());
    }

    #[test]
    fn entity_state_take_entities() {
        let mut state = test_state(5);
        state.set_entities(2, vec![fake_entity(1)]);

        let taken = state.take_entities(2);
        assert!(taken.is_some());
        assert_eq!(taken.unwrap().len(), 1);
        assert!(!state.is_spawned(2));

        // Take from empty slot
        assert!(state.take_entities(2).is_none());

        // Take out of bounds
        assert!(state.take_entities(99).is_none());
    }

    #[test]
    fn entity_state_take_all() {
        let mut state = test_state(5);
        state.set_entities(0, vec![fake_entity(1)]);
        state.set_entities(2, vec![fake_entity(2), fake_entity(3)]);
        state.set_entities(4, vec![fake_entity(4)]);
        state.displayed_frame = Some(2);
        state.prev_windows = vec![(0, 5)];

        let all = state.take_all_entities();
        assert_eq!(all.len(), 4);
        assert!(!state.has_spawned_entities());
        assert!(state.displayed_frame.is_none());
        assert!(state.prev_windows.is_empty());
    }

    #[test]
    fn entity_state_reset() {
        let mut state = test_state(5);
        state.set_entities(0, vec![fake_entity(1)]);
        state.displayed_frame = Some(0);
        state.prev_windows = vec![(0, 3)];

        state.reset();
        assert!(!state.has_spawned_entities());
        assert!(state.displayed_frame.is_none());
        assert!(state.prev_windows.is_empty());
        // Length preserved
        assert_eq!(state.frame_entities.len(), 5);
    }

    #[test]
    fn entity_state_grow_to() {
        let mut state = test_state(3);
        assert_eq!(state.frame_entities.len(), 3);

        state.grow_to(5);
        assert_eq!(state.frame_entities.len(), 5);
        assert_eq!(state.frame_handles.len(), 5);
        assert_eq!(state.total_frames, 5);

        // Growing to smaller size is a no-op
        state.grow_to(2);
        assert_eq!(state.frame_entities.len(), 5);
    }

    // ── SeriesFrameStore ──────────────────────────────────────────────

    #[test]
    fn store_register_and_get() {
        let mut store = SeriesFrameStore::default();
        assert!(store.is_empty());

        assert!(reg(&mut store, sid(1), 10));
        assert!(!store.is_empty());
        assert_eq!(store.len(), 1);
        assert!(store.contains(&sid(1)));

        let state = store.get(&sid(1)).unwrap();
        assert_eq!(state.total_frames, 10);
    }

    #[test]
    fn store_register_duplicate_is_noop() {
        let mut store = SeriesFrameStore::default();
        assert!(reg(&mut store, sid(1), 10));
        assert!(!reg(&mut store, sid(1), 20)); // no-op
        assert_eq!(store.get(&sid(1)).unwrap().total_frames, 10);
    }

    #[test]
    fn store_get_nonexistent() {
        let store = SeriesFrameStore::default();
        assert!(store.get(&sid(99)).is_none());
    }

    #[test]
    fn store_remove() {
        let mut store = SeriesFrameStore::default();
        reg(&mut store, sid(1), 5);

        // Add some entities
        store
            .get_mut(&sid(1))
            .unwrap()
            .set_entities(0, vec![fake_entity(1)]);
        store
            .get_mut(&sid(1))
            .unwrap()
            .set_entities(2, vec![fake_entity(2)]);

        let entities = store.remove(&sid(1));
        assert_eq!(entities.len(), 2);
        assert!(!store.contains(&sid(1)));
        assert!(store.is_empty());
    }

    #[test]
    fn store_remove_nonexistent() {
        let mut store = SeriesFrameStore::default();
        let entities = store.remove(&sid(99));
        assert!(entities.is_empty());
    }

    #[test]
    fn store_multiple_series() {
        let mut store = SeriesFrameStore::default();
        reg(&mut store, sid(1), 10);
        reg(&mut store, sid(2), 20);
        reg(&mut store, sid(3), 5);

        assert_eq!(store.len(), 3);

        store
            .get_mut(&sid(1))
            .unwrap()
            .set_entities(0, vec![fake_entity(1)]);
        store
            .get_mut(&sid(2))
            .unwrap()
            .set_entities(5, vec![fake_entity(2), fake_entity(3)]);

        assert_eq!(store.total_spawned_entities(), 2); // 2 frames with entities

        let all = store.all_entities();
        assert_eq!(all.len(), 3); // 1 + 2 entities total
    }

    #[test]
    fn store_iter() {
        let mut store = SeriesFrameStore::default();
        reg(&mut store, sid(1), 5);
        reg(&mut store, sid(2), 10);

        let ids: Vec<SeriesId> = store.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids.len(), 2);
        assert!(ids.contains(&sid(1)));
        assert!(ids.contains(&sid(2)));
    }

    #[test]
    fn store_iter_mut() {
        let mut store = SeriesFrameStore::default();
        reg(&mut store, sid(1), 5);
        reg(&mut store, sid(2), 5);

        // Set entities on all series via iter_mut
        for (_, state) in store.iter_mut() {
            state.set_entities(0, vec![fake_entity(100)]);
        }

        assert_eq!(store.total_spawned_entities(), 2);
    }

    #[test]
    fn store_reset() {
        let mut store = SeriesFrameStore::default();
        reg(&mut store, sid(1), 5);
        reg(&mut store, sid(2), 10);

        store
            .get_mut(&sid(1))
            .unwrap()
            .set_entities(0, vec![fake_entity(1)]);
        store
            .get_mut(&sid(2))
            .unwrap()
            .set_entities(3, vec![fake_entity(2)]);

        let entities = store.reset();
        assert_eq!(entities.len(), 2);
        assert!(store.is_empty());
    }

    #[test]
    fn store_total_spawned_entities_empty() {
        let store = SeriesFrameStore::default();
        assert_eq!(store.total_spawned_entities(), 0);
    }

    #[test]
    fn store_all_entities_empty() {
        let store = SeriesFrameStore::default();
        assert!(store.all_entities().is_empty());
    }

    #[test]
    fn store_get_mut_and_modify() {
        let mut store = SeriesFrameStore::default();
        reg(&mut store, sid(1), 5);

        {
            let state = store.get_mut(&sid(1)).unwrap();
            state.set_entities(0, vec![fake_entity(1)]);
            state.displayed_frame = Some(0);
        }

        let state = store.get(&sid(1)).unwrap();
        assert!(state.is_spawned(0));
        assert_eq!(state.displayed_frame, Some(0));
    }
}
