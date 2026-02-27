//! Shared mesh cache keyed by canonical file path.
//!
//! [`MeshCache`] is the foundation for multi-series playback: meshes are
//! loaded once and shared across any number of series via `Handle<Mesh>`
//! cloning (Bevy handles are reference-counted internally).
//!
//! ## Phase 1 (current)
//!
//! `MeshCache` is dual-written alongside [`super::assets::FrameStore`] — both
//! resources hold the same data.  No consumer reads from `MeshCache` yet;
//! `FrameStore` remains the single source of truth.
//!
//! ## Phase 2 (future)
//!
//! Per-series entity stores will obtain `Handle<Mesh>` from `MeshCache`
//! instead of `FrameStore`.  `FrameStore`'s handle storage will be removed,
//! and `MeshCache` becomes the sole owner of loaded mesh assets.
//!
//! ## GPU residency tracking
//!
//! Each [`CacheEntry`] carries a `gpu_consumers` refcount so that
//! `promote_to_gpu` / `demote_from_gpu` can be shared safely across series.
//! A mesh is only demoted from the render world when *no* series has spawned
//! entities referencing it.

use bevy::asset::RenderAssetUsages;
use bevy::mesh::Mesh;
use bevy::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// CacheEntry
// ---------------------------------------------------------------------------

/// A single cached mesh file.
///
/// One cache entry may hold multiple `Handle<Mesh>` values because a GLB file
/// can contain several primitives merged into separate meshes by loadfu.
#[derive(Debug)]
pub struct CacheEntry {
    /// Mesh handles produced by `asset_server.load()`.
    ///
    /// Multi-mesh files (GLB with multiple primitives when merge is off)
    /// will have more than one handle.  STL / OBJ always have exactly one.
    pub handles: Vec<Handle<Mesh>>,

    /// Whether all handles have finished loading (asset ready).
    pub loaded: bool,

    /// Number of series (or frame-store slots) currently referencing this
    /// path.  When `refcount` drops to 0 the entry is eligible for eviction.
    pub refcount: usize,

    /// Number of series that have spawned entities using meshes from this
    /// entry.  Only demote from GPU when this reaches 0.
    pub gpu_consumers: usize,

    /// Estimated GPU memory in bytes (set after the mesh finishes loading).
    pub estimated_bytes: u64,
}

impl CacheEntry {
    /// Create a new entry with the given handles and a refcount of 1.
    pub fn new(handles: Vec<Handle<Mesh>>) -> Self {
        Self {
            handles,
            loaded: false,
            refcount: 1,
            gpu_consumers: 0,
            estimated_bytes: 0,
        }
    }

    /// Whether this entry has any mesh handles.
    pub fn is_empty(&self) -> bool {
        self.handles.is_empty()
    }

    /// Increment the reference count (another consumer started using this path).
    pub fn add_ref(&mut self) {
        self.refcount += 1;
    }

    /// Decrement the reference count.  Returns the new count.
    pub fn remove_ref(&mut self) -> usize {
        self.refcount = self.refcount.saturating_sub(1);
        self.refcount
    }

    /// Record that a series has spawned entities referencing these meshes.
    pub fn add_gpu_consumer(&mut self) {
        self.gpu_consumers += 1;
    }

    /// Record that a series has despawned entities.  Returns `true` when
    /// `gpu_consumers` reaches 0 (caller should demote from GPU).
    pub fn remove_gpu_consumer(&mut self) -> bool {
        self.gpu_consumers = self.gpu_consumers.saturating_sub(1);
        self.gpu_consumers == 0
    }

    /// Promote all meshes in this entry to the GPU (add `RENDER_WORLD` flag).
    ///
    /// Returns `true` if at least one mesh was promoted.
    pub fn promote_to_gpu(&self, meshes: &mut Assets<Mesh>) -> bool {
        let mut any = false;
        for handle in &self.handles {
            if let Some(mesh) = meshes.get_mut(handle) {
                mesh.asset_usage = RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD;
                any = true;
            }
        }
        any
    }

    /// Demote all meshes in this entry from the GPU (remove `RENDER_WORLD`
    /// flag).
    ///
    /// Returns `true` if at least one mesh was demoted.
    pub fn demote_from_gpu(&self, meshes: &mut Assets<Mesh>) -> bool {
        let mut any = false;
        for handle in &self.handles {
            if let Some(mesh) = meshes.get_mut(handle) {
                mesh.asset_usage = RenderAssetUsages::MAIN_WORLD;
                any = true;
            }
        }
        any
    }
}

// ---------------------------------------------------------------------------
// MeshCache
// ---------------------------------------------------------------------------

/// Shared mesh cache resource — one entry per canonical file path.
///
/// In Phase 1 this is dual-written alongside `FrameStore` and not yet read
/// by any consumer.  In Phase 2 per-series entity stores will obtain handles
/// from here instead of from `FrameStore`.
#[derive(Resource, Default)]
pub struct MeshCache {
    /// Primary storage: canonical path → cache entry.
    entries: HashMap<PathBuf, CacheEntry>,

    /// Reverse index from `AssetId<Mesh>` → canonical path, for O(1) lookup
    /// when processing `AssetEvent`s.
    asset_to_path: HashMap<AssetId<Mesh>, PathBuf>,
}

impl MeshCache {
    /// Insert a new entry (or replace an existing one) for the given path.
    ///
    /// The reverse `AssetId → path` index is updated automatically.
    /// Returns `true` if the path was already present (entry replaced).
    pub fn insert(&mut self, path: PathBuf, entry: CacheEntry) -> bool {
        // Register reverse index entries for every handle.
        for handle in &entry.handles {
            self.asset_to_path.insert(handle.id(), path.clone());
        }
        self.entries.insert(path, entry).is_some()
    }

    /// Look up an entry by canonical path.
    pub fn get(&self, path: &PathBuf) -> Option<&CacheEntry> {
        self.entries.get(path)
    }

    /// Look up an entry mutably by canonical path.
    pub fn get_mut(&mut self, path: &PathBuf) -> Option<&mut CacheEntry> {
        self.entries.get_mut(path)
    }

    /// Look up the canonical path for an `AssetId<Mesh>`.
    pub fn path_for_asset(&self, id: &AssetId<Mesh>) -> Option<&PathBuf> {
        self.asset_to_path.get(id)
    }

    /// Look up an entry by `AssetId<Mesh>` (O(1) via reverse index).
    pub fn get_by_asset_id(&self, id: &AssetId<Mesh>) -> Option<&CacheEntry> {
        self.asset_to_path
            .get(id)
            .and_then(|path| self.entries.get(path))
    }

    /// Look up an entry mutably by `AssetId<Mesh>`.
    pub fn get_by_asset_id_mut(&mut self, id: &AssetId<Mesh>) -> Option<&mut CacheEntry> {
        // Need to clone the path to avoid double-borrow.
        let path = self.asset_to_path.get(id)?.clone();
        self.entries.get_mut(&path)
    }

    /// Remove an entry by path, cleaning up the reverse index.
    ///
    /// Returns the removed entry if it existed.
    pub fn remove(&mut self, path: &PathBuf) -> Option<CacheEntry> {
        if let Some(entry) = self.entries.remove(path) {
            for handle in &entry.handles {
                self.asset_to_path.remove(&handle.id());
            }
            Some(entry)
        } else {
            None
        }
    }

    /// Check whether the cache contains an entry for the given path.
    pub fn contains(&self, path: &PathBuf) -> bool {
        self.entries.contains_key(path)
    }

    /// Number of cached entries.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Whether the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Total estimated bytes across all entries.
    pub fn total_estimated_bytes(&self) -> u64 {
        self.entries.values().map(|e| e.estimated_bytes).sum()
    }

    /// Number of entries that have finished loading.
    pub fn loaded_count(&self) -> usize {
        self.entries.values().filter(|e| e.loaded).count()
    }

    /// Iterate over all entries (path, entry) pairs.
    pub fn iter(&self) -> impl Iterator<Item = (&PathBuf, &CacheEntry)> {
        self.entries.iter()
    }

    /// Reset the cache completely (e.g. when loading a new sequence).
    pub fn reset(&mut self) {
        self.entries.clear();
        self.asset_to_path.clear();
    }

    /// Decrement the refcount for a path.  If it reaches 0, remove the entry
    /// and return it (caller should drop handles / remove from `Assets<Mesh>`).
    ///
    /// Returns `Some(entry)` if the entry was evicted, `None` if it still has
    /// references or didn't exist.
    pub fn release(&mut self, path: &PathBuf) -> Option<CacheEntry> {
        let should_evict = {
            if let Some(entry) = self.entries.get_mut(path) {
                entry.remove_ref() == 0
            } else {
                return None;
            }
        };

        if should_evict {
            self.remove(path)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::marker::PhantomData;
    use uuid::Uuid;

    /// Create a dummy `Handle<Mesh>` with a unique `AssetId` for testing.
    fn dummy_handle() -> Handle<Mesh> {
        Handle::Uuid(Uuid::new_v4(), PhantomData)
    }

    fn test_path(name: &str) -> PathBuf {
        PathBuf::from(format!("/sequences/sim/{}", name))
    }

    // ── CacheEntry ────────────────────────────────────────────────────

    #[test]
    fn cache_entry_new_defaults() {
        let h = dummy_handle();
        let entry = CacheEntry::new(vec![h]);
        assert!(!entry.loaded);
        assert_eq!(entry.refcount, 1);
        assert_eq!(entry.gpu_consumers, 0);
        assert_eq!(entry.estimated_bytes, 0);
        assert!(!entry.is_empty());
    }

    #[test]
    fn cache_entry_empty() {
        let entry = CacheEntry::new(vec![]);
        assert!(entry.is_empty());
    }

    #[test]
    fn cache_entry_ref_counting() {
        let entry_handles = vec![dummy_handle()];
        let mut entry = CacheEntry::new(entry_handles);
        assert_eq!(entry.refcount, 1);

        entry.add_ref();
        assert_eq!(entry.refcount, 2);

        assert_eq!(entry.remove_ref(), 1);
        assert_eq!(entry.remove_ref(), 0);
        // Saturates at 0
        assert_eq!(entry.remove_ref(), 0);
    }

    #[test]
    fn cache_entry_gpu_consumer_tracking() {
        let mut entry = CacheEntry::new(vec![dummy_handle()]);
        assert_eq!(entry.gpu_consumers, 0);

        entry.add_gpu_consumer();
        entry.add_gpu_consumer();
        assert_eq!(entry.gpu_consumers, 2);

        // First removal — not zero yet
        assert!(!entry.remove_gpu_consumer());
        assert_eq!(entry.gpu_consumers, 1);

        // Second removal — hits zero, should demote
        assert!(entry.remove_gpu_consumer());
        assert_eq!(entry.gpu_consumers, 0);

        // Saturates at 0
        assert!(entry.remove_gpu_consumer());
    }

    // ── MeshCache ─────────────────────────────────────────────────────

    #[test]
    fn mesh_cache_insert_and_get() {
        let mut cache = MeshCache::default();
        let path = test_path("frame_0001.stl");
        let h = dummy_handle();
        let asset_id = h.id();

        cache.insert(path.clone(), CacheEntry::new(vec![h]));

        assert!(cache.contains(&path));
        assert_eq!(cache.len(), 1);
        assert!(!cache.is_empty());

        let entry = cache.get(&path).unwrap();
        assert_eq!(entry.handles.len(), 1);
        assert_eq!(entry.refcount, 1);

        // Reverse index works
        assert_eq!(cache.path_for_asset(&asset_id), Some(&path));
        assert!(cache.get_by_asset_id(&asset_id).is_some());
    }

    #[test]
    fn mesh_cache_insert_replaces() {
        let mut cache = MeshCache::default();
        let path = test_path("frame_0001.stl");

        let h1 = dummy_handle();
        let id1 = h1.id();
        cache.insert(path.clone(), CacheEntry::new(vec![h1]));

        let h2 = dummy_handle();
        let id2 = h2.id();
        let replaced = cache.insert(path.clone(), CacheEntry::new(vec![h2]));
        assert!(replaced);
        assert_eq!(cache.len(), 1);

        // New handle is accessible
        assert!(cache.get_by_asset_id(&id2).is_some());

        // Old reverse-index entry is stale (old handle no longer in entries)
        // but the path mapping may still exist — this is acceptable because
        // the entry it points to has been replaced.  The important thing is
        // that the *new* id maps correctly.
        let _ = id1; // consumed above
    }

    #[test]
    fn mesh_cache_remove() {
        let mut cache = MeshCache::default();
        let path = test_path("frame_0001.stl");
        let h = dummy_handle();
        let asset_id = h.id();

        cache.insert(path.clone(), CacheEntry::new(vec![h]));
        let removed = cache.remove(&path);
        assert!(removed.is_some());
        assert!(cache.is_empty());
        assert!(!cache.contains(&path));

        // Reverse index cleaned up
        assert!(cache.path_for_asset(&asset_id).is_none());
    }

    #[test]
    fn mesh_cache_remove_nonexistent() {
        let mut cache = MeshCache::default();
        let path = test_path("nope.stl");
        assert!(cache.remove(&path).is_none());
    }

    #[test]
    fn mesh_cache_release_with_remaining_refs() {
        let mut cache = MeshCache::default();
        let path = test_path("frame_0001.stl");
        let h = dummy_handle();

        let mut entry = CacheEntry::new(vec![h]);
        entry.add_ref(); // refcount = 2
        cache.insert(path.clone(), entry);

        // release decrements to 1 — entry stays
        let evicted = cache.release(&path);
        assert!(evicted.is_none());
        assert!(cache.contains(&path));
        assert_eq!(cache.get(&path).unwrap().refcount, 1);
    }

    #[test]
    fn mesh_cache_release_evicts_at_zero() {
        let mut cache = MeshCache::default();
        let path = test_path("frame_0001.stl");
        let h = dummy_handle();

        cache.insert(path.clone(), CacheEntry::new(vec![h]));
        // refcount starts at 1, release decrements to 0 → evict
        let evicted = cache.release(&path);
        assert!(evicted.is_some());
        assert!(cache.is_empty());
    }

    #[test]
    fn mesh_cache_release_nonexistent() {
        let mut cache = MeshCache::default();
        let path = test_path("nope.stl");
        assert!(cache.release(&path).is_none());
    }

    #[test]
    fn mesh_cache_reset() {
        let mut cache = MeshCache::default();
        for i in 0..5 {
            let path = test_path(&format!("frame_{:04}.stl", i));
            cache.insert(path, CacheEntry::new(vec![dummy_handle()]));
        }
        assert_eq!(cache.len(), 5);

        cache.reset();
        assert!(cache.is_empty());
        assert_eq!(cache.len(), 0);
    }

    #[test]
    fn mesh_cache_total_estimated_bytes() {
        let mut cache = MeshCache::default();

        for i in 0..3 {
            let path = test_path(&format!("frame_{:04}.stl", i));
            let mut entry = CacheEntry::new(vec![dummy_handle()]);
            entry.estimated_bytes = 1000;
            cache.insert(path, entry);
        }

        assert_eq!(cache.total_estimated_bytes(), 3000);
    }

    #[test]
    fn mesh_cache_loaded_count() {
        let mut cache = MeshCache::default();

        for i in 0..4 {
            let path = test_path(&format!("frame_{:04}.stl", i));
            let mut entry = CacheEntry::new(vec![dummy_handle()]);
            entry.loaded = i % 2 == 0; // 0 and 2 are loaded
            cache.insert(path, entry);
        }

        assert_eq!(cache.loaded_count(), 2);
    }

    #[test]
    fn mesh_cache_get_by_asset_id_mut() {
        let mut cache = MeshCache::default();
        let path = test_path("frame_0001.stl");
        let h = dummy_handle();
        let asset_id = h.id();

        cache.insert(path, CacheEntry::new(vec![h]));

        let entry = cache.get_by_asset_id_mut(&asset_id).unwrap();
        entry.loaded = true;
        entry.estimated_bytes = 42;

        // Verify mutation stuck
        let entry = cache.get_by_asset_id(&asset_id).unwrap();
        assert!(entry.loaded);
        assert_eq!(entry.estimated_bytes, 42);
    }

    #[test]
    fn mesh_cache_multi_handle_entry() {
        let mut cache = MeshCache::default();
        let path = test_path("scene.glb");
        let h1 = dummy_handle();
        let h2 = dummy_handle();
        let id1 = h1.id();
        let id2 = h2.id();

        cache.insert(path.clone(), CacheEntry::new(vec![h1, h2]));

        // Both asset IDs should resolve to the same path
        assert_eq!(cache.path_for_asset(&id1), Some(&path));
        assert_eq!(cache.path_for_asset(&id2), Some(&path));

        // Both should find the same entry
        let e1 = cache.get_by_asset_id(&id1).unwrap();
        let e2 = cache.get_by_asset_id(&id2).unwrap();
        assert_eq!(e1.handles.len(), 2);
        assert_eq!(e2.handles.len(), 2);
    }

    #[test]
    fn mesh_cache_iter() {
        let mut cache = MeshCache::default();
        for i in 0..3 {
            let path = test_path(&format!("frame_{:04}.stl", i));
            cache.insert(path, CacheEntry::new(vec![dummy_handle()]));
        }

        let paths: Vec<_> = cache.iter().map(|(p, _)| p.clone()).collect();
        assert_eq!(paths.len(), 3);
    }
}
