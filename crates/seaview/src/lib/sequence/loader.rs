//! Mesh sequence loading using Bevy's native asset system
//!
//! This module provides loading functionality for mesh sequences supporting
//! multiple file formats:
//!
//! - **glTF / GLB**: Loaded via Bevy's built-in `bevy_gltf` crate.  Multi-mesh
//!   files are fully supported: the loader first loads the top-level [`Gltf`]
//!   asset, then extracts every mesh primitive once it finishes loading.  Each
//!   primitive becomes a separate [`Mesh3d`] entity sharing the same material.
//! - **STL**: Loaded directly as [`Mesh`] assets via the custom
//!   [`StlLoader`](crate::lib::asset_loaders::stl_loader::StlLoader) registered
//!   by [`AssetLoadersPlugin`](crate::lib::asset_loaders::AssetLoadersPlugin).
//!
//! File format is detected by extension at load time so that a single sequence
//! can only contain one format (mixing is not supported).
//!
//! ## Memory Budget Integration
//!
//! The loader integrates with [`MemoryBudget`] to:
//! - Estimate frame size from the first loaded mesh
//! - Track memory usage as frames are spawned/despawned
//! - Respect budget limits when managing the frame window
//!
//! ## Throttled Loading
//!
//! Asset load kicks and entity spawns are spread across frames to avoid
//! stalling the main thread:
//!
//! - **[`LoadQueue`]** — holds frame indices waiting to be sent to the asset
//!   server.  [`drain_load_queue`] processes at most
//!   [`LOADS_PER_FRAME`] per tick.
//! - **[`manage_frame_window`]** — caps entity spawns at
//!   [`SPAWNS_PER_FRAME`] per tick so the render thread doesn't choke on a
//!   sudden batch of new GPU uploads.
//! - **[`sync_section_loads`]** — only recomputes the needed-frame set when
//!   the [`Timeline`] actually changed (Bevy change detection).

use bevy::asset::{AssetEvent, LoadState, RenderAssetUsages};
use bevy::gltf::{Gltf, GltfLoaderSettings, GltfMesh};
use bevy::mesh::Mesh;
use bevy::prelude::*;
use std::collections::{HashMap, HashSet, VecDeque};
use std::path::PathBuf;

/// Read current process RSS in MB from `/proc/self/statm`.
/// Returns 0.0 on non-Linux or if the read fails.
fn rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
        .map(|pages| pages as f64 * 4096.0 / 1_048_576.0)
        .unwrap_or(0.0)
}

use super::{compute_spawn_windows, frames_to_load, SequenceManager, Timeline};

use crate::app::ui::state::RenderConfig;
use crate::lib::memory_budget::{estimate_mesh_gpu_bytes_from_vertices, MemoryBudget};

// ---------------------------------------------------------------------------
// Tuning constants
// ---------------------------------------------------------------------------

/// Maximum number of `asset_server.load()` calls issued per frame.
const LOADS_PER_FRAME: usize = 32;

/// Maximum number of mesh entities spawned per frame in
/// [`manage_frame_window`].
const SPAWNS_PER_FRAME: usize = 8;

// ---------------------------------------------------------------------------
// MeshFileFormat
// ---------------------------------------------------------------------------

/// Recognised mesh file formats for sequence loading.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeshFileFormat {
    /// STL (binary or ASCII) – loaded directly as [`Mesh`] via our custom loader.
    Stl,
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
            Self::Gltf => "glTF",
            Self::Glb => "GLB",
        }
    }
}

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// Plugin for mesh sequence loading
pub struct SequenceLoaderPlugin;

impl Plugin for SequenceLoaderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SequenceAssets>()
            .init_resource::<LoadQueue>()
            .add_message::<FrameLoadedEvent>()
            .add_message::<LoadSequenceRequest>()
            .add_systems(
                Update,
                (
                    handle_load_requests,
                    sync_section_loads,
                    drain_load_queue,
                    extract_gltf_meshes,
                    track_asset_loading,
                )
                    .chain(),
            )
            .add_systems(PostUpdate, manage_frame_window);
    }
}

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// Resource that holds all loaded mesh handles for the current sequence
#[derive(Resource, Default)]
pub struct SequenceAssets {
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
    /// Shared material handle (created once, cloned to each entity)
    pub shared_material: Option<Handle<StandardMaterial>>,
    /// Cached coordinate transform (set once when sequence loads)
    pub coord_transform: Transform,
    /// Estimated GPU memory per frame in bytes (set after first frame loads)
    pub estimated_frame_bytes: u64,
    /// O(1) reverse index from `AssetId<Mesh>` → frame index.
    ///
    /// Populated by [`drain_load_queue`] (STL) or [`extract_gltf_meshes`]
    /// (GLB) and consumed by [`track_asset_loading`] to avoid a linear scan.
    asset_id_to_frame: HashMap<AssetId<Mesh>, usize>,

    /// Intermediate Gltf asset handles for GLB two-phase loading.
    /// Phase 1: `drain_load_queue` loads the top-level `Gltf` asset.
    /// Phase 2: `extract_gltf_meshes` extracts all mesh primitives.
    pub(crate) gltf_handles: Vec<Option<Handle<Gltf>>>,
    /// Reverse index from `AssetId<Gltf>` → frame index.
    gltf_id_to_frame: HashMap<AssetId<Gltf>, usize>,
}

impl SequenceAssets {
    /// Reset the sequence assets for a new sequence
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
        self.shared_material = None;
        self.coord_transform = Transform::default();
        self.estimated_frame_bytes = 0;
        self.asset_id_to_frame.clear();
        self.gltf_handles.clear();
        self.gltf_id_to_frame.clear();
    }

    /// Check if all frames are loaded
    pub fn is_fully_loaded(&self) -> bool {
        self.total_frames > 0 && self.loaded_count >= self.total_frames
    }

    /// Get loading progress as a percentage (0.0 to 1.0)
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

    /// Check if a specific frame's asset has finished loading
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

/// A throttled queue of frame indices waiting to be sent to the asset server.
///
/// Systems push indices here instead of calling `asset_server.load()` directly.
/// [`drain_load_queue`] pops at most [`LOADS_PER_FRAME`] per tick.
#[derive(Resource, Default)]
pub struct LoadQueue {
    queue: VecDeque<usize>,
    /// Tracks indices already in the queue to prevent duplicate enqueues.
    pending: HashSet<usize>,
}

impl LoadQueue {
    /// Push a frame index to be loaded (deduplicates automatically).
    fn push(&mut self, idx: usize) {
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
    fn clear(&mut self) {
        self.queue.clear();
        self.pending.clear();
    }
}

// ---------------------------------------------------------------------------
// Events / Messages
// ---------------------------------------------------------------------------

/// Event fired when a frame finishes loading
#[derive(Message)]
pub struct FrameLoadedEvent {
    /// Index of the loaded frame
    pub frame_index: usize,
    /// Whether loading succeeded
    pub success: bool,
}

/// Event to request loading a sequence
#[derive(Message)]
pub struct LoadSequenceRequest {
    /// Paths to all frame files (in order)
    pub frame_paths: Vec<PathBuf>,
}

// ---------------------------------------------------------------------------
// Components
// ---------------------------------------------------------------------------

/// Component to mark the mesh display entity
#[derive(Component)]
pub struct SequenceMeshDisplay;

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Build the Bevy asset path for one frame.
///
/// For **STL** files the path is simply `seq://<filename>` – our custom
/// [`StlLoader`](crate::lib::asset_loaders::stl_loader::StlLoader) is
/// registered for the `.stl` extension so the asset server routes it
/// automatically.
///
/// For **glTF / GLB** files we use [`GltfAssetLabel::Primitive`] to request
/// the first mesh primitive (`mesh 0, primitive 0`), which gives back a
/// `Handle<Mesh>`.
/// Load a single STL mesh frame.
///
/// Returns a `Handle<Mesh>` directly — STL files always contain exactly one
/// mesh, so no two-phase loading is needed.
pub fn load_stl_mesh_handle(asset_server: &AssetServer, filename: &str) -> Handle<Mesh> {
    let asset_path = format!("seq://{}", filename);
    // The StlLoader registered for ".stl" will produce a Mesh directly.
    // STL loader already creates meshes with MAIN_WORLD only.
    asset_server.load(asset_path)
}

/// Load a GLB/glTF file as a top-level [`Gltf`] asset.
///
/// This is phase 1 of multi-mesh loading.  The returned handle must be
/// watched for `LoadedWithDependencies`; at that point
/// [`extract_gltf_meshes`] pulls out every mesh primitive.
pub fn load_gltf_handle(asset_server: &AssetServer, filename: &str) -> Handle<Gltf> {
    let asset_path = format!("seq://{}", filename);
    asset_server.load_with_settings(asset_path, |settings: &mut GltfLoaderSettings| {
        settings.load_meshes = RenderAssetUsages::MAIN_WORLD;
    })
}

/// Pre-estimate the maximum number of frames that fit in the memory budget by
/// stat-ing a sample of files on disk.  This gives a conservative upper bound
/// *before* any GPU data is available, preventing the loader from blasting out
/// thousands of asset-server requests when `MemoryBudget::max_frames` is still
/// `usize::MAX`.
///
/// The estimate uses raw file size as a proxy for GPU memory.  For GLB this
/// over-estimates (file includes JSON + padding) so we'll load slightly fewer
/// frames than optimal — `sync_section_loads` will expand the window once the
/// real vertex-based estimate arrives.
fn pre_estimate_max_frames(
    paths: &[PathBuf],
    budget: &MemoryBudget,
    sample_count: usize,
) -> Option<usize> {
    if paths.is_empty() {
        return None;
    }
    let budget_bytes = budget.effective_budget();
    if budget_bytes == 0 {
        return None;
    }

    // Sample up to `sample_count` evenly-spaced files
    let n = paths.len();
    let step = (n / sample_count).max(1);
    let mut total_bytes: u64 = 0;
    let mut sampled = 0u64;
    for i in (0..n).step_by(step).take(sample_count) {
        if let Ok(meta) = std::fs::metadata(&paths[i]) {
            total_bytes += meta.len();
            sampled += 1;
        }
    }
    if sampled == 0 {
        return None;
    }

    let avg_bytes = total_bytes / sampled;
    if avg_bytes == 0 {
        return None;
    }

    let max = (budget_bytes / avg_bytes) as usize;
    info!(
        "Pre-estimate from disk: ~{:.1} MB/frame (avg of {} samples), \
         budget {:.1} GB → max ~{} frames",
        avg_bytes as f64 / 1_048_576.0,
        sampled,
        budget_bytes as f64 / 1_073_741_824.0,
        max,
    );
    Some(max)
}

/// Detect format from the first path in the request and log a warning if any
/// frame has a different extension.
fn detect_sequence_format(paths: &[PathBuf]) -> Option<MeshFileFormat> {
    let first = paths.first()?;
    let ext = first.extension()?.to_str()?;
    let format = MeshFileFormat::from_extension(ext)?;

    // Warn about mixed extensions (we don't support mixing formats).
    for (i, p) in paths.iter().enumerate().skip(1) {
        if let Some(other_ext) = p.extension().and_then(|e| e.to_str()) {
            if MeshFileFormat::from_extension(other_ext) != Some(format) {
                warn!(
                    "Frame {} has extension '{}' but sequence format is {:?} – it may fail to load",
                    i, other_ext, format
                );
            }
        }
    }

    Some(format)
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// System that handles sequence load requests.
///
/// Sets up `SequenceAssets` and `Timeline` for a new sequence but does **not**
/// kick off any `asset_server.load()` calls itself.  Instead the needed frame
/// indices are pushed into the [`LoadQueue`] and drained over subsequent
/// frames by [`drain_load_queue`].
fn handle_load_requests(
    mut commands: Commands,
    mut load_requests: MessageReader<LoadSequenceRequest>,
    mut sequence_assets: ResMut<SequenceAssets>,
    mut timeline: ResMut<Timeline>,
    memory_budget: Res<MemoryBudget>,
    mut load_queue: ResMut<LoadQueue>,
) {
    for request in load_requests.read() {
        info!(
            "═══ LOADING SEQUENCE: {} frames ═══",
            request.frame_paths.len()
        );

        // Detect file format from the first frame.
        let format = match detect_sequence_format(&request.frame_paths) {
            Some(fmt) => {
                info!("Detected format: {:?}", fmt);
                fmt
            }
            None => {
                let first = request.frame_paths.first();
                let ext = first
                    .and_then(|p| p.extension())
                    .and_then(|e| e.to_str())
                    .unwrap_or("<none>");
                error!(
                    "Unsupported mesh file extension '{}' – cannot load sequence",
                    ext
                );
                continue;
            }
        };

        // Cleanup old sequence entities before resetting
        for entity_opt in sequence_assets.frame_entities.iter() {
            if let Some(entities) = entity_opt {
                for entity in entities {
                    commands.entity(*entity).despawn();
                }
            }
        }

        // Reset assets + queue for new sequence
        sequence_assets.reset();
        load_queue.clear();

        let n = request.frame_paths.len();
        sequence_assets.total_frames = n;
        sequence_assets.loading = true;
        sequence_assets.format = Some(format);
        // Pre-allocate vecs — handles start as None (loaded on demand)
        sequence_assets.frame_handles = vec![None; n];
        sequence_assets.frame_entities = vec![None; n];
        sequence_assets.gltf_handles = vec![None; n];
        sequence_assets.loaded_frames = vec![false; n];
        // Store all paths for on-demand loading
        sequence_assets.frame_paths = request.frame_paths.clone();

        // Initialize Timeline for the new sequence.
        // If the real budget isn't known yet (first load, no vertex data),
        // pre-estimate from file sizes on disk so we don't blast out thousands
        // of asset requests.
        let max_frames = if memory_budget.max_frames < usize::MAX {
            memory_budget.max_frames
        } else {
            pre_estimate_max_frames(&request.frame_paths, &memory_budget, 5).unwrap_or(n)
        };
        timeline.init_sequence(n, max_frames);
        info!(
            "Timeline initialized: frames 0..{} (total: {}, budget max: {})",
            timeline.section().end,
            n,
            max_frames
        );

        // Store base path from first frame
        if let Some(first_path) = request.frame_paths.first() {
            sequence_assets.base_path = first_path.parent().map(|p| p.to_path_buf());
        }

        // Enqueue frames for throttled loading instead of blasting them all
        // at once.  `drain_load_queue` will issue the actual
        // `asset_server.load()` calls spread over subsequent frames.
        let indices = frames_to_load(&timeline.sections, n);
        info!("Enqueuing {} of {} frames for loading", indices.len(), n);
        for idx in indices {
            load_queue.push(idx);
        }

        info!(
            "Asset loading queued: {} {:?} frames (of {} total) from {:?}",
            load_queue.len(),
            format,
            sequence_assets.total_frames,
            sequence_assets.base_path
        );
    }
}

/// Watches for timeline section changes and reconciles the set of loaded frame
/// assets to match.
///
/// **Only runs when the [`Timeline`] resource has changed** (Bevy change
/// detection), avoiding O(n) work every frame during steady-state playback.
///
/// Frames entering a section are pushed into the [`LoadQueue`]; frames leaving
/// all sections have their handle dropped immediately so the asset server can
/// reclaim memory.
fn sync_section_loads(
    mut sequence_assets: ResMut<SequenceAssets>,
    timeline: Res<Timeline>,
    mut load_queue: ResMut<LoadQueue>,
    mut prev_sections: Local<Vec<super::LoadSection>>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    // Early-out: nothing to do if no sequence loaded.
    if sequence_assets.total_frames == 0 {
        return;
    }
    // Only re-sync when sections actually changed (not on every playhead tick).
    if timeline.sections == *prev_sections {
        return;
    }
    *prev_sections = timeline.sections.clone();
    let Some(_format) = sequence_assets.format else {
        return;
    };

    // Clear stale queue entries — frames from previously-active sections
    // would otherwise be picked up by drain_load_queue and re-loaded even
    // though their section is now inactive.
    load_queue.clear();

    let rss_before = rss_mb();

    let needed = frames_to_load(&timeline.sections, sequence_assets.total_frames);

    let mut loads_enqueued = 0usize;

    // Enqueue frames that are in sections but don't have a handle yet
    for &idx in &needed {
        // For GLB, check gltf_handles too — the frame is "requested" once
        // either the mesh handles or the intermediate Gltf handle exists.
        let has_handle = sequence_assets.frame_handles[idx].is_some()
            || sequence_assets.gltf_handles[idx].is_some();
        if !has_handle {
            load_queue.push(idx);
            loads_enqueued += 1;
        }
    }

    // Unload frames that are outside all sections (drop handle, reclaim memory).
    // Build a fast lookup of needed indices.
    let mut is_needed = vec![false; sequence_assets.total_frames];
    for &idx in &needed {
        is_needed[idx] = true;
    }
    let mut unloads = 0usize;
    let mut handles_with_entities = 0usize;
    let mut meshes_removed = 0usize;
    for idx in 0..sequence_assets.total_frames {
        let has_handle = sequence_assets.frame_handles[idx].is_some()
            || sequence_assets.gltf_handles[idx].is_some();
        if !is_needed[idx] && has_handle {
            if sequence_assets.frame_entities[idx].is_some() {
                handles_with_entities += 1;
            }
            // Remove all mesh asset IDs from reverse index and force-remove
            // from Assets<Mesh> immediately.
            // Collect IDs first to avoid borrowing sequence_assets immutably
            // and mutably at the same time.
            let mesh_ids: Vec<AssetId<Mesh>> = sequence_assets.frame_handles[idx]
                .as_ref()
                .map(|handles| handles.iter().map(|h| h.id()).collect())
                .unwrap_or_default();
            for id in mesh_ids {
                sequence_assets.asset_id_to_frame.remove(&id);
                if meshes.remove(id).is_some() {
                    meshes_removed += 1;
                }
            }
            // Also remove the intermediate Gltf handle if present.
            if let Some(gltf_handle) = sequence_assets.gltf_handles[idx].take() {
                sequence_assets.gltf_id_to_frame.remove(&gltf_handle.id());
            }
            sequence_assets.frame_handles[idx] = None;
            if sequence_assets.loaded_frames[idx] {
                sequence_assets.loaded_frames[idx] = false;
                sequence_assets.loaded_count = sequence_assets.loaded_count.saturating_sub(1);
            }
            unloads += 1;
        }
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
        sequence_assets.loading = true;
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

/// Drains up to [`LOADS_PER_FRAME`] items from the [`LoadQueue`], issuing
/// `asset_server.load()` for each and populating the reverse
/// `AssetId → frame` index for O(1) lookups in [`track_asset_loading`].
fn drain_load_queue(
    mut load_queue: ResMut<LoadQueue>,
    mut sequence_assets: ResMut<SequenceAssets>,
    asset_server: Res<AssetServer>,
) {
    if load_queue.is_empty() {
        return;
    }
    let Some(format) = sequence_assets.format else {
        return;
    };

    let mut loaded_this_frame = 0usize;

    while let Some(idx) = load_queue.queue.pop_front() {
        load_queue.pending.remove(&idx);
        if loaded_this_frame >= LOADS_PER_FRAME {
            break;
        }

        // Skip if already has a handle (mesh or intermediate Gltf).
        if sequence_assets.frame_handles[idx].is_some()
            || sequence_assets.gltf_handles[idx].is_some()
        {
            continue;
        }

        // Guard against out-of-bounds (e.g. sequence was reset).
        if idx >= sequence_assets.frame_paths.len() {
            continue;
        }

        let filename = sequence_assets.frame_paths[idx]
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
            .to_owned();

        match format {
            MeshFileFormat::Stl => {
                let handle = load_stl_mesh_handle(&asset_server, &filename);
                // Populate the reverse index for O(1) event lookup.
                sequence_assets.asset_id_to_frame.insert(handle.id(), idx);
                sequence_assets.frame_handles[idx] = Some(vec![handle]);
            }
            MeshFileFormat::Gltf | MeshFileFormat::Glb => {
                let gltf_handle = load_gltf_handle(&asset_server, &filename);
                sequence_assets
                    .gltf_id_to_frame
                    .insert(gltf_handle.id(), idx);
                sequence_assets.gltf_handles[idx] = Some(gltf_handle);
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

/// System that watches for loaded [`Gltf`] assets and extracts all mesh
/// primitives from them, populating [`SequenceAssets::frame_handles`].
///
/// This is phase 2 of multi-mesh GLB loading.  When a `Gltf` asset fires
/// `LoadedWithDependencies`, all its sub-assets (meshes, materials, etc.)
/// are already loaded, so we can immediately extract `Handle<Mesh>` from
/// every primitive and mark the frame as loaded.
fn extract_gltf_meshes(
    mut events: MessageReader<AssetEvent<Gltf>>,
    mut sequence_assets: ResMut<SequenceAssets>,
    mut frame_loaded_events: MessageWriter<FrameLoadedEvent>,
    gltf_assets: Res<Assets<Gltf>>,
    gltf_mesh_assets: Res<Assets<GltfMesh>>,
    meshes: Res<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut memory_budget: ResMut<MemoryBudget>,
    sequence_manager: Res<SequenceManager>,
) {
    if !sequence_assets.loading {
        return;
    }

    for event in events.read() {
        let AssetEvent::LoadedWithDependencies { id } = event else {
            continue;
        };

        let Some(&frame_idx) = sequence_assets.gltf_id_to_frame.get(id) else {
            continue;
        };

        // Access the loaded Gltf asset to extract mesh primitives.
        let gltf_handle = match &sequence_assets.gltf_handles[frame_idx] {
            Some(h) => h.clone(),
            None => continue,
        };
        let Some(gltf) = gltf_assets.get(&gltf_handle) else {
            warn!(
                "Gltf asset for frame {} finished loading but not found in Assets<Gltf>",
                frame_idx
            );
            continue;
        };

        // Collect all mesh primitive handles.
        let mut mesh_handles: Vec<Handle<Mesh>> = Vec::new();
        for gltf_mesh_handle in &gltf.meshes {
            if let Some(gltf_mesh) = gltf_mesh_assets.get(gltf_mesh_handle) {
                for primitive in &gltf_mesh.primitives {
                    mesh_handles.push(primitive.mesh.clone());
                }
            }
        }

        if mesh_handles.is_empty() {
            warn!("Frame {} GLB has no mesh primitives", frame_idx);
            continue;
        }

        let mesh_count = mesh_handles.len();

        // Register all mesh asset IDs in the reverse index.
        for handle in &mesh_handles {
            sequence_assets
                .asset_id_to_frame
                .insert(handle.id(), frame_idx);
        }

        // Store the mesh handles.
        sequence_assets.frame_handles[frame_idx] = Some(mesh_handles.clone());

        // Mark frame as loaded.
        if !sequence_assets.loaded_frames[frame_idx] {
            sequence_assets.loaded_frames[frame_idx] = true;
            sequence_assets.loaded_count += 1;
        }

        frame_loaded_events.write(FrameLoadedEvent {
            frame_index: frame_idx,
            success: true,
        });

        // Create shared material on first loaded frame.
        if sequence_assets.shared_material.is_none() {
            sequence_assets.shared_material = Some(materials.add(StandardMaterial {
                base_color: Color::srgb(0.3, 0.5, 0.8),
                perceptual_roughness: 0.4,
                metallic: 0.1,
                cull_mode: None, // Double-sided rendering
                ..default()
            }));
        }

        // Estimate frame size from first loaded mesh for memory budgeting.
        if sequence_assets.estimated_frame_bytes == 0 {
            let mut total_vertices = 0usize;
            for handle in &mesh_handles {
                if let Some(mesh) = meshes.get(handle) {
                    total_vertices += mesh.count_vertices();
                }
            }
            if total_vertices > 0 {
                let estimated_bytes = estimate_mesh_gpu_bytes_from_vertices(total_vertices);
                sequence_assets.estimated_frame_bytes = estimated_bytes;
                memory_budget.update_frame_estimate(estimated_bytes);
                info!(
                    "Frame size estimate (GLB, {} meshes): {} vertices ≈ {:.1} MB, max {} frames in budget",
                    mesh_count,
                    total_vertices,
                    estimated_bytes as f64 / 1_048_576.0,
                    memory_budget.max_frames
                );
            }
        }

        // Cache coordinate transform on first loaded frame.
        if frame_idx == 0 {
            if let Some(sequence) = sequence_manager.current_sequence() {
                sequence_assets.coord_transform = sequence.source_orientation.to_transform();
                debug!(
                    "Applying coordinate transformation: {}",
                    sequence.source_orientation.description()
                );
            }
        }

        // Log progress at intervals.
        let requested_count = sequence_assets
            .gltf_handles
            .iter()
            .filter(|h| h.is_some())
            .count();
        if sequence_assets.loaded_count % 10 == 0 || sequence_assets.loaded_count == requested_count
        {
            info!(
                "Loading progress (GLB): {}/{} frames loaded ({} meshes in frame {})",
                sequence_assets.loaded_count, requested_count, mesh_count, frame_idx,
            );
        }
    }
}

/// System that tracks asset loading progress.
///
/// Uses the `asset_id_to_frame` reverse index for O(1) lookups instead of
/// scanning the full handle list on every event.
///
/// Does NOT spawn entities — that is handled by `manage_frame_window`.
/// Tracks STL mesh loading via `AssetEvent<Mesh>`.
///
/// GLB frames are handled by [`extract_gltf_meshes`] instead — their mesh
/// asset IDs are registered in `asset_id_to_frame` *after* the Gltf loads,
/// but by that time the `AssetEvent<Mesh>` has already fired.  So this
/// system only processes STL frames (whose mesh IDs are registered up-front
/// in `drain_load_queue`).
fn track_asset_loading(
    mut events: MessageReader<AssetEvent<Mesh>>,
    mut sequence_assets: ResMut<SequenceAssets>,
    mut frame_loaded_events: MessageWriter<FrameLoadedEvent>,
    asset_server: Res<AssetServer>,
    sequence_manager: Res<SequenceManager>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    meshes: Res<Assets<Mesh>>,
    mut memory_budget: ResMut<MemoryBudget>,
) {
    if !sequence_assets.loading {
        return;
    }

    // Count how many frames we actually requested (have a handle or gltf handle)
    let requested_count = sequence_assets
        .frame_handles
        .iter()
        .zip(sequence_assets.gltf_handles.iter())
        .filter(|(fh, gh)| fh.is_some() || gh.is_some())
        .count();

    for event in events.read() {
        match event {
            AssetEvent::LoadedWithDependencies { id } => {
                // O(1) lookup via reverse index — only STL frames are registered here.
                let Some(&index) = sequence_assets.asset_id_to_frame.get(id) else {
                    continue;
                };
                // Skip if this frame was already marked loaded (e.g. by extract_gltf_meshes).
                if sequence_assets.loaded_frames[index] {
                    continue;
                }
                let handle = match sequence_assets.frame_handles[index].as_ref() {
                    Some(handles) => match handles.first() {
                        Some(h) => h.clone(),
                        None => continue,
                    },
                    None => continue,
                };

                sequence_assets.loaded_count += 1;
                sequence_assets.loaded_frames[index] = true;

                frame_loaded_events.write(FrameLoadedEvent {
                    frame_index: index,
                    success: true,
                });

                // Create shared material on first loaded frame
                if sequence_assets.shared_material.is_none() {
                    sequence_assets.shared_material = Some(materials.add(StandardMaterial {
                        base_color: Color::srgb(0.3, 0.5, 0.8),
                        perceptual_roughness: 0.4,
                        metallic: 0.1,
                        cull_mode: None, // Double-sided rendering
                        ..default()
                    }));
                }

                // Estimate frame size from first loaded mesh for memory budgeting
                if sequence_assets.estimated_frame_bytes == 0 {
                    if let Some(mesh) = meshes.get(&handle) {
                        let vertex_count = mesh.count_vertices();
                        let estimated_bytes = estimate_mesh_gpu_bytes_from_vertices(vertex_count);
                        sequence_assets.estimated_frame_bytes = estimated_bytes;
                        memory_budget.update_frame_estimate(estimated_bytes);
                        info!(
                            "Frame size estimate: {} vertices ≈ {:.1} MB, max {} frames in budget",
                            vertex_count,
                            estimated_bytes as f64 / 1_048_576.0,
                            memory_budget.max_frames
                        );
                    }
                }

                // Cache coordinate transform on first loaded frame
                if index == 0 {
                    if let Some(sequence) = sequence_manager.current_sequence() {
                        sequence_assets.coord_transform =
                            sequence.source_orientation.to_transform();
                        debug!(
                            "Applying coordinate transformation: {}",
                            sequence.source_orientation.description()
                        );
                    }
                }

                // Log progress at intervals
                if sequence_assets.loaded_count % 10 == 0
                    || sequence_assets.loaded_count == requested_count
                {
                    info!(
                        "Loading progress: {}/{} frames loaded",
                        sequence_assets.loaded_count, requested_count,
                    );
                }
            }
            _ => {}
        }
    }

    // Check if all requested frames are loaded
    if sequence_assets.loaded_count >= requested_count
        && requested_count > 0
        && sequence_assets.loading
    {
        sequence_assets.loading = false;
        info!("════════════════════════════════════════════════════════════════");
        info!(
            "✓ SECTION LOADING COMPLETE: {}/{} frames loaded (budget-limited)",
            sequence_assets.loaded_count, sequence_assets.total_frames,
        );
        info!("════════════════════════════════════════════════════════════════");
    }

    // Check for failed loads with detailed information
    if sequence_assets.loading {
        let mut failed_count = 0;
        let mut failed_indices = Vec::new();

        // Check mesh handles (STL)
        for (index, handle_opt) in sequence_assets.frame_handles.iter().enumerate() {
            let Some(handles) = handle_opt else { continue };
            let Some(handle) = handles.first() else {
                continue;
            };
            let state = asset_server.load_state(handle.id());
            match state {
                LoadState::Failed(ref err) => {
                    failed_count += 1;
                    failed_indices.push(index);
                    if failed_count <= 3 {
                        error!("Frame {} load failed: {:?}", index, err);
                    }
                }
                LoadState::NotLoaded => {
                    if requested_count <= 10 || index % 20 == 0 {
                        debug!("Frame {} still loading...", index);
                    }
                }
                _ => {}
            }
        }

        // Check gltf handles (GLB)
        for (index, handle_opt) in sequence_assets.gltf_handles.iter().enumerate() {
            let Some(handle) = handle_opt else { continue };
            if sequence_assets.loaded_frames[index] {
                continue; // Already extracted
            }
            let state = asset_server.load_state(handle.id());
            match state {
                LoadState::Failed(ref err) => {
                    failed_count += 1;
                    failed_indices.push(index);
                    if failed_count <= 3 {
                        error!("Frame {} GLB load failed: {:?}", index, err);
                    }
                }
                _ => {}
            }
        }

        if failed_count > 0 {
            warn!(
                "{} frames failed to load (indices: {:?}...)",
                failed_count,
                &failed_indices[..failed_indices.len().min(5)]
            );
        }
    }
}

/// System that manages the ring-buffer of spawned frame entities.
///
/// This is the **sole owner of visibility** for sequence mesh entities.
/// Runs in `PostUpdate` so that `timeline.playhead` is finalised for this
/// tick (all `Update` writers have run).
///
/// Entity spawns are capped at [`SPAWNS_PER_FRAME`] to avoid stalling the
/// render thread with a sudden batch of GPU uploads.
fn manage_frame_window(
    mut commands: Commands,
    mut sequence_assets: ResMut<SequenceAssets>,
    timeline: Res<Timeline>,
    render_config: Res<RenderConfig>,
    mut visibility_query: Query<&mut Visibility, With<SequenceMeshDisplay>>,
    mut prev_windows: Local<Vec<(usize, usize)>>,
    mut memory_budget: ResMut<MemoryBudget>,
    mut meshes: ResMut<Assets<Mesh>>,
) {
    if sequence_assets.total_frames == 0 {
        return;
    }
    // Need shared material before we can spawn anything
    let Some(material) = sequence_assets.shared_material.clone() else {
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
        sequence_assets.total_frames,
    );

    let transform = sequence_assets.coord_transform;
    let frame_bytes = sequence_assets.estimated_frame_bytes;

    // Helper: check if `idx` falls inside any of the given window ranges.
    let in_windows = |idx: usize, windows: &[(usize, usize)]| -> bool {
        windows.iter().any(|&(lo, hi)| idx >= lo && idx < hi)
    };

    // 1. Despawn entities that were in old windows but are NOT in new windows.
    //    Despawns are NOT throttled — freeing memory is always welcome.
    //    We also demote the mesh back to MAIN_WORLD only so Bevy's render
    //    asset system releases the GPU buffer.
    let rss_before_despawn = rss_mb();
    let mut despawns = 0usize;
    let mut demote_ok = 0usize;
    let mut demote_fail = 0usize;
    for &(old_lo, old_hi) in prev_windows.iter() {
        for idx in old_lo..old_hi {
            if in_windows(idx, &new_windows) {
                continue; // still needed
            }
            if let Some(entities) = sequence_assets.frame_entities[idx].take() {
                for entity in &entities {
                    if let Ok(mut vis) = visibility_query.get_mut(*entity) {
                        *vis = Visibility::Hidden;
                    }
                    commands.entity(*entity).despawn();
                }
                let has_handle = sequence_assets.frame_handles[idx].is_some();
                if sequence_assets.demote_from_gpu(idx, &mut meshes) {
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
    }
    if despawns > 0 {
        let rss_after_despawn = rss_mb();
        info!(
            "manage_frame_window: despawned {} entities (demote ok: {}, fail: {}) | RSS {:.1} → {:.1} MB (Δ {:.1})",
            despawns, demote_ok, demote_fail,
            rss_before_despawn, rss_after_despawn, rss_after_despawn - rss_before_despawn,
        );
    }

    // 2. Spawn entities that are in new windows but were NOT in old windows.
    //    Capped at SPAWNS_PER_FRAME to spread GPU upload work.
    let mut spawns_this_frame = 0usize;
    for &(win_lo, win_hi) in &new_windows {
        for idx in win_lo..win_hi {
            if spawns_this_frame >= SPAWNS_PER_FRAME {
                break;
            }
            if sequence_assets.frame_entities[idx].is_some() {
                continue; // already spawned
            }
            if !sequence_assets.is_frame_loaded(idx) {
                continue; // asset not ready yet
            }
            let Some(mesh_handles) = sequence_assets.get_frame_meshes(idx) else {
                continue;
            };
            let mesh_handles: Vec<Handle<Mesh>> = mesh_handles.to_vec();

            sequence_assets.promote_to_gpu(idx, &mut meshes);

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

            debug!(
                "Spawned frame {} — {} entities (window [{}, {}))",
                idx,
                entities.len(),
                win_lo,
                win_hi
            );

            sequence_assets.frame_entities[idx] = Some(entities);

            if frame_bytes > 0 {
                memory_budget.record_frame_loaded(frame_bytes);
            }

            spawns_this_frame += 1;
        }
    }

    // Log window changes
    if new_windows != *prev_windows {
        let active_handles = sequence_assets
            .frame_handles
            .iter()
            .zip(sequence_assets.gltf_handles.iter())
            .filter(|(fh, gh)| fh.is_some() || gh.is_some())
            .count();
        let live_entities = sequence_assets
            .frame_entities
            .iter()
            .filter(|e| e.is_some())
            .count();
        info!(
            "Frame windows changed: {:?} → {:?}, current frame: {} | handles: {}, entities: {}, RSS: {:.1} MB",
            *prev_windows, new_windows, current,
            active_handles, live_entities, rss_mb(),
        );
    }

    // 3. Set visibility on all entities across all windows.
    //    Only the current frame is Visible; everything else is Hidden.
    for &(win_lo, win_hi) in &new_windows {
        for idx in win_lo..win_hi {
            if let Some(Some(entities)) = sequence_assets.frame_entities.get(idx) {
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

    sequence_assets.displayed_frame = Some(current);
    *prev_windows = new_windows;
}

// ---------------------------------------------------------------------------
// Public helpers
// ---------------------------------------------------------------------------

/// Helper function to trigger loading a sequence from discovered frames
#[allow(dead_code)]
pub fn load_discovered_sequence(
    sequence: &super::Sequence,
    load_events: &mut MessageWriter<LoadSequenceRequest>,
) {
    let frame_paths: Vec<PathBuf> = sequence.frames.iter().map(|f| f.path.clone()).collect();

    if frame_paths.is_empty() {
        warn!("Cannot load sequence with no frames");
        return;
    }

    debug!(
        "Requesting load of sequence '{}' with {} frames",
        sequence.name,
        frame_paths.len()
    );

    load_events.write(LoadSequenceRequest { frame_paths });
}

/// Get loading statistics for UI display
#[derive(Debug, Clone)]
pub struct LoadingStats {
    pub total_frames: usize,
    pub loaded_frames: usize,
    pub loading: bool,
    pub progress_percent: f32,
}

impl LoadingStats {
    pub fn from_assets(assets: &SequenceAssets) -> Self {
        Self {
            total_frames: assets.total_frames,
            loaded_frames: assets.loaded_count,
            loading: assets.loading,
            progress_percent: assets.progress() * 100.0,
        }
    }
}
