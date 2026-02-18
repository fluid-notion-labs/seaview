//! Resource definitions for the sequence loader.
//!
//! This module contains the three focused resources that hold frame data:
//!
//! - **[`FrameStore`]** — public, read-heavy: frame handles, paths, entities,
//!   loading state, progress.
//! - **[`LoaderConfig`]** — semi-public: shared material handle and coordinate
//!   transform.
//! - **[`AssetIndex`]** — crate-private: reverse `AssetId → frame` maps and
//!   intermediate glTF handles.
//!
//! Also contains [`MeshFileFormat`] (format detection) and [`LoadingStats`]
//! (UI-friendly progress snapshot).

use bevy::asset::RenderAssetUsages;
use bevy::gltf::Gltf;
use bevy::mesh::Mesh;
use bevy::prelude::*;
use std::collections::HashMap;
use std::path::PathBuf;

// ---------------------------------------------------------------------------
// MeshFileFormat
// ---------------------------------------------------------------------------

/// Recognised mesh file formats for sequence loading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshFileFormat {
    /// STL (binary or ASCII) – loaded directly as [`Mesh`] via our custom loader.
    Stl,
    /// OBJ (Wavefront) – loaded directly as [`Mesh`] via our custom loader.
    Obj,
    /// glTF 2.0 text format – loaded via Bevy's gltf crate.
    Gltf,
    /// glTF 2.0 binary container – loaded via Bevy's gltf crate.
    Glb,
}

impl MeshFileFormat {
    /// Try to detect the format from a file extension string (case-insensitive).
    pub fn from_extension(ext: &str) -> Option<Self> {
        match ext.to_ascii_lowercase().as_str() {
            "stl" => Some(Self::Stl),
            "obj" => Some(Self::Obj),
            "gltf" => Some(Self::Gltf),
            "glb" => Some(Self::Glb),
            _ => None,
        }
    }

    /// Returns true if this format is a glTF variant (gltf or glb).
    pub fn is_gltf(&self) -> bool {
        matches!(self, Self::Gltf | Self::Glb)
    }

    /// Human-readable label for display.
    pub fn label(&self) -> &'static str {
        match self {
            Self::Stl => "STL",
            Self::Obj => "OBJ",
            Self::Gltf => "glTF",
            Self::Glb => "GLB",
        }
    }

    /// Returns true if this format loads directly as a Mesh (not via Gltf intermediate).
    pub fn is_direct_mesh(&self) -> bool {
        matches!(self, Self::Stl | Self::Obj)
    }
}

// ---------------------------------------------------------------------------
// FrameStore
// ---------------------------------------------------------------------------

/// Public resource holding frame handles, paths, entities, and loading progress.
///
/// This is the read-heavy resource — UI systems and many loader systems need
/// it.  Separating it from [`AssetIndex`] lets Bevy schedule UI reads in
/// parallel with loader writes to the index maps.
#[derive(Resource, Default)]
pub struct FrameStore {
    /// Handles to loaded mesh frames (indexed by frame number).
    /// Each frame may contain multiple meshes (e.g. multi-object GLB files).
    /// `None` means the asset has not been requested from the server yet.
    pub frame_handles: Vec<Option<Vec<Handle<Mesh>>>>,
    /// On-disk paths for every frame so we can load on demand.
    pub frame_paths: Vec<PathBuf>,
    /// Which frames have finished loading (asset ready)
    pub loaded_frames: Vec<bool>,
    /// Number of frames that have finished loading
    pub loaded_count: usize,
    /// Total frames expected
    pub total_frames: usize,
    /// Base path of the sequence (for asset source registration)
    pub base_path: Option<PathBuf>,
    /// Whether loading is in progress
    pub loading: bool,
    /// Spawned entities per frame index (`None` = not currently spawned).
    /// Multi-mesh frames have multiple entities per frame.
    pub frame_entities: Vec<Option<Vec<Entity>>>,
    /// Currently displayed (visible) frame index
    pub displayed_frame: Option<usize>,
    /// Detected file format for this sequence (set on first load request)
    pub format: Option<MeshFileFormat>,
    /// Estimated GPU memory per frame in bytes (set after first frame loads)
    pub estimated_frame_bytes: u64,
}

impl FrameStore {
    /// Reset for a new sequence.
    pub fn reset(&mut self) {
        self.frame_handles.clear();
        self.frame_paths.clear();
        self.loaded_frames.clear();
        self.loaded_count = 0;
        self.total_frames = 0;
        self.loading = false;
        self.displayed_frame = None;
        self.format = None;
        self.frame_entities.clear();
        self.estimated_frame_bytes = 0;
    }

    /// Check if all frames are loaded.
    pub fn is_fully_loaded(&self) -> bool {
        self.total_frames > 0 && self.loaded_count >= self.total_frames
    }

    /// Get loading progress as a percentage (0.0 to 1.0).
    pub fn progress(&self) -> f32 {
        if self.total_frames == 0 {
            0.0
        } else {
            self.loaded_count as f32 / self.total_frames as f32
        }
    }

    /// Get the first mesh handle for a specific frame (back-compat).
    ///
    /// For multi-mesh GLB frames this returns only the first primitive.
    /// Use [`get_frame_meshes`] to get all meshes.
    pub fn get_frame(&self, index: usize) -> Option<&Handle<Mesh>> {
        self.frame_handles
            .get(index)
            .and_then(|opt| opt.as_ref())
            .and_then(|vec| vec.first())
    }

    /// Get all mesh handles for a specific frame.
    pub fn get_frame_meshes(&self, index: usize) -> Option<&[Handle<Mesh>]> {
        self.frame_handles
            .get(index)
            .and_then(|opt| opt.as_ref())
            .map(|vec| vec.as_slice())
    }

    /// Check if a specific frame's asset has finished loading.
    pub fn is_frame_loaded(&self, index: usize) -> bool {
        self.loaded_frames.get(index).copied().unwrap_or(false)
    }

    /// Get the first entity for the currently displayed frame.
    pub fn displayed_entity(&self) -> Option<Entity> {
        self.displayed_frame
            .and_then(|idx| self.frame_entities.get(idx))
            .and_then(|opt| opt.as_ref())
            .and_then(|entities| entities.first().copied())
    }

    /// Compute the window range `[lo, hi)` centered on `center` with the given
    /// half-width, clamped to `[0, total_frames)`.
    pub fn window_range(&self, center: usize, window_size: usize) -> (usize, usize) {
        let half = window_size / 2;
        let lo = center.saturating_sub(half);
        let hi = (center + half + 1).min(self.total_frames);
        (lo, hi)
    }

    // ── GPU residency management ──────────────────────────────────────
    //
    // Meshes are loaded with `MAIN_WORLD` only (CPU-resident).  These
    // helpers flip the `asset_usage` flag so Bevy's render-asset system
    // uploads / releases the GPU buffer on demand.

    /// Promote a frame's mesh to GPU by adding the `RENDER_WORLD` flag.
    ///
    /// This triggers Bevy's render-asset system to upload the mesh to
    /// VRAM on the next extract pass.  Returns `true` if the flag was set.
    pub fn promote_to_gpu(&self, idx: usize, meshes: &mut Assets<Mesh>) -> bool {
        let Some(handles) = self.get_frame_meshes(idx) else {
            return false;
        };
        let mut any = false;
        for handle in handles {
            if let Some(mesh) = meshes.get_mut(handle) {
                mesh.asset_usage = RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD;
                any = true;
            }
        }
        any
    }

    /// Demote a frame's mesh from GPU by removing the `RENDER_WORLD` flag.
    ///
    /// This triggers Bevy's render-asset system to release the GPU buffer.
    /// Returns `true` if the flag was cleared.
    pub fn demote_from_gpu(&self, idx: usize, meshes: &mut Assets<Mesh>) -> bool {
        let Some(handles) = self.get_frame_meshes(idx) else {
            return false;
        };
        let mut any = false;
        for handle in handles {
            if let Some(mesh) = meshes.get_mut(handle) {
                mesh.asset_usage = RenderAssetUsages::MAIN_WORLD;
                any = true;
            }
        }
        any
    }
}

// ---------------------------------------------------------------------------
// LoaderConfig
// ---------------------------------------------------------------------------

/// Semi-public resource holding the material handle and coordinate transform
/// determined by the loader when the first frame loads.
///
/// Read by [`super::frame_window`] when spawning entities and by material
/// systems.  Written only by [`super::loading_tracker`] systems.
#[derive(Resource)]
pub struct LoaderConfig {
    /// Shared material handle (created once, cloned to each entity)
    pub shared_material: Option<Handle<StandardMaterial>>,
    /// Cached coordinate transform (set once when sequence loads)
    pub coord_transform: Transform,
}

impl Default for LoaderConfig {
    fn default() -> Self {
        Self {
            shared_material: None,
            coord_transform: Transform::default(),
        }
    }
}

impl LoaderConfig {
    /// Reset for a new sequence.
    pub fn reset(&mut self) {
        self.shared_material = None;
        self.coord_transform = Transform::default();
    }
}

// ---------------------------------------------------------------------------
// AssetIndex
// ---------------------------------------------------------------------------

/// Loader-private resource holding reverse `AssetId → frame` index maps and
/// intermediate glTF handles.
///
/// Write-heavy, only accessed by loader systems.  Separating it from
/// [`FrameStore`] lets Bevy schedule UI reads in parallel.
#[derive(Resource, Default)]
pub(crate) struct AssetIndex {
    /// O(1) reverse index from `AssetId<Mesh>` → frame index.
    ///
    /// Populated by [`super::load_queue::drain_load_queue`] (STL) or
    /// [`super::loading_tracker::extract_gltf_meshes`] (GLB) and consumed by
    /// [`super::loading_tracker::track_asset_loading`] to avoid a linear scan.
    pub(crate) asset_id_to_frame: HashMap<AssetId<Mesh>, usize>,

    /// Intermediate Gltf asset handles for GLB two-phase loading.
    /// Phase 1: `drain_load_queue` loads the top-level `Gltf` asset.
    /// Phase 2: `extract_gltf_meshes` extracts all mesh primitives.
    pub(crate) gltf_handles: Vec<Option<Handle<Gltf>>>,

    /// Reverse index from `AssetId<Gltf>` → frame index.
    pub(crate) gltf_id_to_frame: HashMap<AssetId<Gltf>, usize>,
}

impl AssetIndex {
    /// Reset for a new sequence.
    pub(crate) fn reset(&mut self) {
        self.asset_id_to_frame.clear();
        self.gltf_handles.clear();
        self.gltf_id_to_frame.clear();
    }
}

// ---------------------------------------------------------------------------
// LoadingStats
// ---------------------------------------------------------------------------

/// Get loading statistics for UI display
#[derive(Debug, Clone)]
pub struct LoadingStats {
    pub total_frames: usize,
    pub loaded_frames: usize,
    pub loading: bool,
    pub progress_percent: f32,
}

impl LoadingStats {
    pub fn from_frame_store(store: &FrameStore) -> Self {
        Self {
            total_frames: store.total_frames,
            loaded_frames: store.loaded_count,
            loading: store.loading,
            progress_percent: store.progress() * 100.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── FrameStore ────────────────────────────────────────────────────

    #[test]
    fn test_frame_store_reset() {
        let mut store = FrameStore::default();
        store.total_frames = 10;
        store.loaded_count = 5;
        store.loading = true;
        store.format = Some(MeshFileFormat::Stl);
        store.estimated_frame_bytes = 1024;

        store.reset();

        assert_eq!(store.total_frames, 0);
        assert_eq!(store.loaded_count, 0);
        assert!(!store.loading);
        assert!(store.format.is_none());
        assert_eq!(store.estimated_frame_bytes, 0);
    }

    #[test]
    fn test_frame_store_progress() {
        let mut store = FrameStore::default();
        store.total_frames = 200;
        store.loaded_count = 100;
        assert!((store.progress() - 0.5).abs() < f32::EPSILON);
    }

    #[test]
    fn test_frame_store_is_fully_loaded() {
        let mut store = FrameStore::default();
        assert!(!store.is_fully_loaded());

        store.total_frames = 10;
        store.loaded_count = 10;
        assert!(store.is_fully_loaded());
    }

    #[test]
    fn test_frame_store_is_frame_loaded() {
        let mut store = FrameStore::default();
        store.loaded_frames = vec![false, true, false];
        assert!(!store.is_frame_loaded(0));
        assert!(store.is_frame_loaded(1));
        assert!(!store.is_frame_loaded(2));
        assert!(!store.is_frame_loaded(999)); // out of bounds
    }

    #[test]
    fn test_frame_store_window_range() {
        let mut store = FrameStore::default();
        store.total_frames = 100;

        // Center at 50, window 10 → [45, 56)
        let (lo, hi) = store.window_range(50, 10);
        assert_eq!(lo, 45);
        assert_eq!(hi, 56);

        // Near start → clamped
        let (lo, hi) = store.window_range(2, 10);
        assert_eq!(lo, 0);
        assert_eq!(hi, 8);

        // Near end → clamped
        let (lo, hi) = store.window_range(98, 10);
        assert_eq!(lo, 93);
        assert_eq!(hi, 100);
    }

    // ── MeshFileFormat ────────────────────────────────────────────────

    #[test]
    fn test_mesh_format_from_extension() {
        assert_eq!(
            MeshFileFormat::from_extension("stl"),
            Some(MeshFileFormat::Stl)
        );
        assert_eq!(
            MeshFileFormat::from_extension("STL"),
            Some(MeshFileFormat::Stl)
        );
        assert_eq!(
            MeshFileFormat::from_extension("obj"),
            Some(MeshFileFormat::Obj)
        );
        assert_eq!(
            MeshFileFormat::from_extension("OBJ"),
            Some(MeshFileFormat::Obj)
        );
        assert_eq!(
            MeshFileFormat::from_extension("gltf"),
            Some(MeshFileFormat::Gltf)
        );
        assert_eq!(
            MeshFileFormat::from_extension("glb"),
            Some(MeshFileFormat::Glb)
        );
        assert_eq!(MeshFileFormat::from_extension("fbx"), None);
    }

    #[test]
    fn test_mesh_format_is_gltf() {
        assert!(!MeshFileFormat::Stl.is_gltf());
        assert!(!MeshFileFormat::Obj.is_gltf());
        assert!(MeshFileFormat::Gltf.is_gltf());
        assert!(MeshFileFormat::Glb.is_gltf());
    }

    #[test]
    fn test_mesh_format_is_direct_mesh() {
        assert!(MeshFileFormat::Stl.is_direct_mesh());
        assert!(MeshFileFormat::Obj.is_direct_mesh());
        assert!(!MeshFileFormat::Gltf.is_direct_mesh());
        assert!(!MeshFileFormat::Glb.is_direct_mesh());
    }

    // ── LoadingStats ──────────────────────────────────────────────────

    #[test]
    fn test_loading_stats_from_frame_store() {
        let mut store = FrameStore::default();
        store.total_frames = 100;
        store.loaded_count = 50;
        store.loading = true;

        let stats = LoadingStats::from_frame_store(&store);
        assert_eq!(stats.total_frames, 100);
        assert_eq!(stats.loaded_frames, 50);
        assert!(stats.loading);
        assert!((stats.progress_percent - 50.0).abs() < f32::EPSILON);
    }

    #[test]
    fn test_loading_stats_zero_frames() {
        let store = FrameStore::default();
        let stats = LoadingStats::from_frame_store(&store);
        assert_eq!(stats.progress_percent, 0.0);
    }
}
