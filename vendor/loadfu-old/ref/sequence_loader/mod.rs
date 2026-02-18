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
//! - **OBJ**: Loaded directly as [`Mesh`] assets via the custom
//!   [`ObjLoader`](crate::lib::asset_loaders::obj_loader::ObjLoader) registered
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
//!   server.  [`drain_load_queue`](load_queue::drain_load_queue) processes at
//!   most [`LOADS_PER_FRAME`] per tick.
//! - **[`manage_frame_window`](frame_window::manage_frame_window)** — caps
//!   entity spawns at [`SPAWNS_PER_FRAME`] per tick so the render thread
//!   doesn't choke on a sudden batch of new GPU uploads.
//! - **[`sync_section_loads`](load_queue::sync_section_loads)** — only
//!   recomputes the needed-frame set when the [`Timeline`] actually changed
//!   (Bevy change detection).
//!
//! ## Resource Split
//!
//! Frame data is divided into three focused resources:
//!
//! - **[`FrameStore`]** — public, read-heavy: frame handles, paths, entities,
//!   loading state, progress.  Read by UI and loader systems alike.
//! - **[`LoaderConfig`]** — semi-public: material handle and coordinate
//!   transform determined by the loader on first frame load.  Read by
//!   [`manage_frame_window`](frame_window::manage_frame_window) when spawning
//!   entities.
//! - **[`AssetIndex`]** — private to the loader: reverse `AssetId → frame`
//!   maps and intermediate glTF handles.  Write-heavy, never read by UI.
//!
//! This separation lets Bevy schedule UI reads of [`FrameStore`] in parallel
//! with loader writes to [`AssetIndex`].

mod assets;
mod frame_window;
mod load_queue;
mod loading_tracker;
pub(crate) mod spatial_index;

// ── Public re-exports ─────────────────────────────────────────────────────
pub(crate) use assets::AssetIndex;
pub use assets::{FrameStore, LoaderConfig, LoadingStats, MeshFileFormat};
pub use frame_window::{compute_window_delta, WindowDelta};
pub use load_queue::LoadQueue;
pub use loading_tracker::default_material_properties;
pub use spatial_index::SpatialIndex;

use bevy::asset::RenderAssetUsages;
use bevy::gltf::{Gltf, GltfLoaderSettings};
use bevy::mesh::Mesh;
use bevy::prelude::*;
use std::path::PathBuf;

use super::{frames_to_load, Timeline};
use crate::lib::memory_budget::MemoryBudget;

// ---------------------------------------------------------------------------
// Tuning constants
// ---------------------------------------------------------------------------

/// Maximum number of `asset_server.load()` calls issued per frame.
const LOADS_PER_FRAME: usize = 32;

/// Maximum number of mesh entities spawned per frame in
/// [`manage_frame_window`](frame_window::manage_frame_window).
const SPAWNS_PER_FRAME: usize = 8;

// ---------------------------------------------------------------------------
// Shared helpers (used by sub-modules via `super::`)
// ---------------------------------------------------------------------------

/// Read current process RSS in MB from `/proc/self/statm`.
/// Returns 0.0 on non-Linux or if the read fails.
fn rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
        .map(|pages| pages as f64 * 4096.0 / 1_048_576.0)
        .unwrap_or(0.0)
}

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// Plugin for mesh sequence loading
pub struct SequenceLoaderPlugin;

impl Plugin for SequenceLoaderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<FrameStore>()
            .init_resource::<LoaderConfig>()
            .init_resource::<AssetIndex>()
            .init_resource::<LoadQueue>()
            .add_message::<FrameLoadedEvent>()
            .add_message::<LoadSequenceRequest>()
            .add_systems(
                Update,
                (
                    handle_load_requests,
                    load_queue::sync_section_loads,
                    load_queue::drain_load_queue,
                    loading_tracker::extract_gltf_meshes,
                    loading_tracker::track_asset_loading,
                )
                    .chain(),
            )
            .add_systems(PostUpdate, frame_window::manage_frame_window);
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

/// Load a mesh file that produces a [`Mesh`] asset directly (STL, OBJ).
///
/// Returns a `Handle<Mesh>` directly — these formats produce exactly one
/// mesh per file, so no two-phase loading is needed (unlike glTF/GLB).
///
/// The asset server dispatches to the appropriate loader based on file
/// extension: [`StlLoader`](crate::lib::asset_loaders::StlLoader) for `.stl`,
/// [`ObjLoader`](crate::lib::asset_loaders::ObjLoader) for `.obj`.
pub fn load_direct_mesh_handle(asset_server: &AssetServer, filename: &str) -> Handle<Mesh> {
    let asset_path = format!("seq://{}", filename);
    // The appropriate loader (StlLoader or ObjLoader) will produce a Mesh directly.
    // Both loaders create meshes with MAIN_WORLD only.
    asset_server.load(asset_path)
}

/// Load a GLB/glTF file as a top-level [`Gltf`] asset.
///
/// This is phase 1 of multi-mesh loading.  The returned handle must be
/// watched for `LoadedWithDependencies`; at that point
/// [`extract_gltf_meshes`](loading_tracker::extract_gltf_meshes) pulls out
/// every mesh primitive.
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
/// Sets up [`FrameStore`], [`LoaderConfig`], [`AssetIndex`] and [`Timeline`]
/// for a new sequence but does **not** kick off any `asset_server.load()`
/// calls itself.  Instead the needed frame indices are pushed into the
/// [`LoadQueue`] and drained over subsequent frames by
/// [`drain_load_queue`](load_queue::drain_load_queue).
fn handle_load_requests(
    mut commands: Commands,
    mut load_requests: MessageReader<LoadSequenceRequest>,
    mut frame_store: ResMut<FrameStore>,
    mut loader_config: ResMut<LoaderConfig>,
    mut asset_index: ResMut<AssetIndex>,
    mut timeline: ResMut<Timeline>,
    memory_budget: Res<MemoryBudget>,
    mut load_queue: ResMut<LoadQueue>,
    mut spatial_index: Option<ResMut<SpatialIndex>>,
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
        for entity_opt in frame_store.frame_entities.iter() {
            if let Some(entities) = entity_opt {
                for entity in entities {
                    commands.entity(*entity).despawn();
                }
            }
        }

        // Reset all resources + queue for new sequence
        frame_store.reset();
        loader_config.reset();
        asset_index.reset();
        load_queue.clear();

        let n = request.frame_paths.len();
        if let Some(ref mut si) = spatial_index {
            si.reset(n);
        }
        frame_store.total_frames = n;
        frame_store.loading = true;
        frame_store.format = Some(format);
        // Pre-allocate vecs — handles start as None (loaded on demand)
        frame_store.frame_handles = vec![None; n];
        frame_store.frame_entities = vec![None; n];
        frame_store.loaded_frames = vec![false; n];
        asset_index.gltf_handles = vec![None; n];
        // Store all paths for on-demand loading
        frame_store.frame_paths = request.frame_paths.clone();

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
            frame_store.base_path = first_path.parent().map(|p| p.to_path_buf());
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
            frame_store.total_frames,
            frame_store.base_path
        );
    }
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
