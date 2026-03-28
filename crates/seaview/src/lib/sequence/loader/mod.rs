//! Mesh sequence loading using Bevy's native asset system
//!
//! This module provides loading functionality for mesh sequences supporting
//! multiple file formats via **loadfu**:
//!
//! - **STL**: Loaded directly as [`Mesh`] assets via loadfu's `StlLoader`.
//! - **OBJ**: Loaded directly as [`Mesh`] assets via loadfu's `ObjLoader`.
//! - **glTF / GLB**: Loaded directly as [`Mesh`] assets via loadfu's
//!   `GlbLoader`, which merges all primitives into a single mesh by default.
//!   No more two-phase `bevy_gltf` delegation.
//!
//! All formats produce `Handle<Mesh>` directly — format detection is handled
//! by [`loadfu::MeshFormat`].
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
//!   map.  Write-heavy, never read by UI.
//!
//! This separation lets Bevy schedule UI reads of [`FrameStore`] in parallel
//! with loader writes to [`AssetIndex`].

mod assets;
mod frame_window;
mod load_queue;
mod loading_tracker;
pub(crate) mod mesh_cache;
pub(crate) mod series_frame_store;
pub(crate) mod spatial_index;

// ── Public re-exports ─────────────────────────────────────────────────────
pub(crate) use assets::AssetIndex;
pub use assets::{FrameStore, LoaderConfig, LoadingStats};
pub use frame_window::{compute_window_delta, WindowDelta};
pub use load_queue::LoadQueue;
pub use loading_tracker::default_material_properties;
pub use mesh_cache::{CacheEntry, MeshCache};
pub use series_frame_store::{SeriesEntityState, SeriesFrameStore};
pub use spatial_index::SpatialIndex;

use bevy::mesh::Mesh;
use bevy::prelude::*;
use loadfu::MeshFormat;
use std::path::PathBuf;

use super::{frames_to_load, LoadSection, SeriesManagerRes, Timeline};
use crate::lib::memory_budget::MemoryBudget;
use crate::lib::series::SeriesId;
use crate::lib::settings::SettingsResource;

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
/// Returns 0.0 when the `debug_mem` feature is disabled or on non-Linux.
#[cfg(feature = "debug_mem")]
fn rss_mb() -> f64 {
    std::fs::read_to_string("/proc/self/statm")
        .ok()
        .and_then(|s| s.split_whitespace().nth(1)?.parse::<u64>().ok())
        .map(|pages| pages as f64 * 4096.0 / 1_048_576.0)
        .unwrap_or(0.0)
}

#[cfg(not(feature = "debug_mem"))]
#[inline(always)]
fn rss_mb() -> f64 {
    0.0
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
            .init_resource::<MeshCache>()
            .init_resource::<SeriesFrameStore>()
            .add_message::<FrameLoadedEvent>()
            .add_message::<LoadSequenceRequest>()
            .add_systems(
                Update,
                (
                    handle_load_requests,
                    load_queue::sync_section_loads,
                    load_queue::drain_load_queue,
                    loading_tracker::track_asset_loading,
                )
                    .chain(),
            )
            .add_systems(
                PostUpdate,
                (
                    frame_window::manage_frame_window,
                    frame_window::sync_series_transform,
                ),
            );
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
    /// The series this sequence belongs to.
    pub series_id: SeriesId,
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

/// Load a mesh file as a `Handle<Mesh>` via the asset server.
///
/// With loadfu, **all** formats (STL, OBJ, GLB, glTF) produce `Handle<Mesh>`
/// directly — the asset server dispatches to the appropriate loadfu loader
/// based on file extension.  No two-phase loading is needed for any format.
///
/// GLB/glTF files are merged into a single mesh by default
/// (`GlbLoaderSettings { merge: true }`).
///
/// Uses the `env://` asset source (rooted at `/`) so that frames from any
/// absolute path on disk load correctly — including series in directories
/// different from the initially-registered `seq://` source.
pub fn load_mesh_handle(asset_server: &AssetServer, path: &std::path::Path) -> Handle<Mesh> {
    let abs = path
        .canonicalize()
        .unwrap_or_else(|_| path.to_path_buf());
    let rel = abs
        .strip_prefix("/")
        .unwrap_or(&abs)
        .to_string_lossy();
    let asset_path = format!("env://{}", rel);
    asset_server.load(asset_path)
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
fn detect_sequence_format(paths: &[PathBuf]) -> Option<MeshFormat> {
    let first = paths.first()?;
    let format = MeshFormat::from_path(first)?;

    // Warn about mixed extensions (we don't support mixing formats).
    for (i, p) in paths.iter().enumerate().skip(1) {
        if let Some(other_format) = MeshFormat::from_path(p) {
            if other_format != format {
                warn!(
                    "Frame {} has format {:?} but sequence format is {:?} – it may fail to load",
                    i, other_format, format
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
/// Each [`LoadSequenceRequest`] carries a `series_id`.  The per-series asset
/// state (handles, paths, load progress) is registered into [`SeriesFrameStore`]
/// keyed by that ID.  [`FrameStore`] is kept in sync as an active-series mirror
/// for legacy UI and spatial-index consumers.
///
/// Entity spawns are NOT triggered here — that happens in
/// [`frame_window::manage_frame_window`] once assets finish loading.
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
    mut mesh_cache: ResMut<MeshCache>,
    mut series_fs: ResMut<SeriesFrameStore>,
    series_mgr: Res<SeriesManagerRes>,
    settings_res: Res<SettingsResource>,
) {
    for request in load_requests.read() {
        let series_id = request.series_id;
        let n = request.frame_paths.len();

        info!(
            "═══ LOADING SEQUENCE [{}]: {} frames ═══",
            series_id,
            n,
        );

        // Detect file format from the first frame.
        let format = match detect_sequence_format(&request.frame_paths) {
            Some(fmt) => {
                info!("Detected format: {}", fmt.label());
                fmt
            }
            None => {
                let ext = request
                    .frame_paths
                    .first()
                    .and_then(|p| p.extension())
                    .and_then(|e| e.to_str())
                    .unwrap_or("<none>");
                error!(
                    "Unsupported mesh file extension '{}' – cannot load sequence [{}]",
                    ext, series_id,
                );
                continue;
            }
        };

        // Despawn any existing entities for this series before replacing its state.
        let old_entities = series_fs.remove(&series_id);
        for entity in old_entities {
            commands.entity(entity).despawn();
        }

        // Also clear stale FrameStore mirror entities if this is the active series.
        if series_mgr.active_id() == Some(series_id) {
            for entity_opt in frame_store.frame_entities.iter() {
                if let Some(entities) = entity_opt {
                    for entity in entities {
                        commands.entity(*entity).despawn();
                    }
                }
            }
            frame_store.reset();
            loader_config.reset();
            asset_index.reset();
            load_queue.clear();
            mesh_cache.reset();
        }

        // Register the series with its paths and format in SeriesFrameStore.
        // If it already existed (just removed above) this will insert fresh state.
        series_fs.register(series_id, request.frame_paths.clone(), Some(format));

        // ── Active-series mirror ──────────────────────────────────────────
        // Keep FrameStore in sync for the active series so existing UI / spatial
        // index consumers continue to work unchanged.
        if series_mgr.active_id() == Some(series_id) {
            if let Some(ref mut si) = spatial_index {
                si.reset(n);
            }
            frame_store.total_frames = n;
            frame_store.loading = true;
            frame_store.format = Some(format);
            frame_store.frame_handles = vec![None; n];
            frame_store.frame_entities = vec![None; n];
            frame_store.loaded_frames = vec![false; n];
            frame_store.degenerate_frames = vec![false; n];
            frame_store.frame_paths = request.frame_paths.clone();

            if let Some(first_path) = request.frame_paths.first() {
                frame_store.base_path = first_path.parent().map(|p| p.to_path_buf());
            }

            let max_frames = if memory_budget.max_frames < usize::MAX {
                memory_budget.max_frames
            } else {
                pre_estimate_max_frames(&request.frame_paths, &memory_budget, 5).unwrap_or(n)
            };
            timeline.init_sequence(n, max_frames);

            let restored = restore_saved_timeline(
                &mut timeline,
                &series_mgr,
                &settings_res,
                &frame_store,
                n,
                max_frames,
            );
            if restored {
                info!(
                    "Timeline restored: {} section(s), playhead={} (total: {}, budget max: {})",
                    timeline.sections.len(),
                    timeline.playhead,
                    n,
                    max_frames,
                );
            } else {
                info!(
                    "Timeline initialized (default): frames 0..{} (total: {}, budget max: {})",
                    timeline.section().end,
                    n,
                    max_frames,
                );
            }

            let indices = frames_to_load(&timeline.sections, n);
            info!("Enqueuing {} of {} frames for loading", indices.len(), n);
            for idx in indices {
                load_queue.push(idx);
            }

            info!(
                "Asset loading queued: {} {} frames (of {} total) from {:?}",
                load_queue.len(),
                format.label(),
                n,
                frame_store.base_path
            );
        } else {
            // Non-active series: enqueue loads directly into the series-scoped
            // load queue.  Sections drive which frames are needed, but are
            // always capped to the memory budget (series may have been added
            // with usize::MAX max_frames before the budget was known).
            let max_frames = if memory_budget.max_frames < usize::MAX {
                memory_budget.max_frames
            } else {
                pre_estimate_max_frames(&request.frame_paths, &memory_budget, 5).unwrap_or(n)
            };
            let sections: Vec<LoadSection> = series_mgr
                .get(series_id)
                .map(|s| s.timeline.sections.clone())
                .unwrap_or_default();
            let indices: Vec<usize> = if sections.is_empty() {
                // Default: load up to max_frames from the start.
                (0..max_frames.min(n)).collect()
            } else {
                // Respect the budget — don't load more frames than the budget allows.
                frames_to_load(&sections, n)
                    .into_iter()
                    .take(max_frames)
                    .collect()
            };
            info!(
                "[{}] Enqueuing {} of {} frames (non-active series)",
                series_id,
                indices.len(),
                n,
            );
            // Push into the shared LoadQueue — drain_load_queue reads per-series
            // paths from SeriesFrameStore so the right handles land in the right slot.
            if let Some(state) = series_fs.get_mut(&series_id) {
                for idx in &indices {
                    if *idx < state.frame_paths.len() {
                        load_queue.push_series(*idx, series_id);
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Saved-state restoration helper
// ---------------------------------------------------------------------------

/// Try to restore saved load sections and playhead into the timeline,
/// returning `true` if saved state was applied.
///
/// Resolution order:
/// 1. Active series in [`SeriesManagerRes`] (if `restore_from_saved` already
///    ran and populated per-series sections).
/// 2. Top-level `load_sections` / `load_section` from [`SettingsResource`]
///    (legacy single-series path, or if the series manager hasn't been
///    populated yet).
///
/// Sections are clamped to `total_frames` and filtered to non-empty.
fn restore_saved_timeline(
    timeline: &mut Timeline,
    series_mgr: &SeriesManagerRes,
    settings_res: &SettingsResource,
    frame_store: &FrameStore,
    total_frames: usize,
    max_frames: usize,
) -> bool {
    use crate::lib::sequence::LoadSection;

    // --- Strategy 1: active series in SeriesManager --------------------------
    if let Some(active) = series_mgr.active() {
        // Match by base_dir to make sure we're restoring for the right sequence.
        let base_matches = frame_store
            .base_path
            .as_deref()
            .map(|bp| bp == active.base_dir)
            .unwrap_or(false);

        if base_matches && !active.timeline.sections.is_empty() {
            let mut sections: Vec<LoadSection> = active
                .timeline
                .sections
                .iter()
                .cloned()
                .map(|mut s| {
                    s.clamp(total_frames, max_frames);
                    s
                })
                .filter(|s| s.frame_count() > 0)
                .collect();

            if !sections.is_empty() {
                sections.sort_by_key(|s| s.start);
                timeline.sections = sections;
                let max_playhead = total_frames.saturating_sub(1);
                if active.timeline.playhead > 0 {
                    timeline.playhead = active.timeline.playhead.min(max_playhead);
                }
                return true;
            }
        }
    }

    // --- Strategy 2: top-level settings (legacy / fallback) ------------------
    let saved_sections = settings_res.settings.get_load_sections();
    if !saved_sections.is_empty() {
        let mut sections: Vec<LoadSection> = saved_sections
            .into_iter()
            .map(|mut s| {
                s.clamp(total_frames, max_frames);
                s
            })
            .filter(|s| s.frame_count() > 0)
            .collect();

        if !sections.is_empty() {
            sections.sort_by_key(|s| s.start);
            timeline.sections = sections;
            // Legacy path doesn't have a separate playhead in settings,
            // so leave the default (0) from init_sequence.
            return true;
        }
    }

    false
}

// ---------------------------------------------------------------------------
// Public helpers
// ---------------------------------------------------------------------------

/// Helper function to trigger loading a sequence from discovered frames
#[allow(dead_code)]
pub fn load_discovered_sequence(
    series_id: SeriesId,
    sequence: &super::Sequence,
    load_events: &mut MessageWriter<LoadSequenceRequest>,
) {
    let frame_paths: Vec<PathBuf> = sequence.frames.iter().map(|f| f.path.clone()).collect();

    if frame_paths.is_empty() {
        warn!("Cannot load sequence with no frames");
        return;
    }

    debug!(
        "Requesting load of sequence '{}' ({}) with {} frames",
        sequence.name,
        series_id,
        frame_paths.len()
    );

    load_events.write(LoadSequenceRequest { series_id, frame_paths });
}
