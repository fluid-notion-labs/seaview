//! Mesh sequence loading using Bevy's native asset system
//!
//! This module provides loading functionality for mesh sequences supporting
//! multiple file formats via **loadfu**:
//!
//! - **STL**: Loaded directly as [`Mesh`] assets via loadfu's `StlLoader`.
//! - **OBJ**: Loaded directly as [`Mesh`] assets via loadfu's `ObjLoader`.
//! - **glTF / GLB**: Loaded directly as [`Mesh`] assets via loadfu's
//!   `GlbLoader`, which merges all primitives into a single mesh by default.
//! - **Meshlet** (`--features meshlet`): Pre-baked `.meshlet` files produced
//!   by the `manifold` pipeline.  See below.
//!
//! File format is detected by extension at load time.  A single sequence must
//! contain only one format — mixing is not supported.
//!
//! ## Mesh vs Meshlet — two separate render paths
//!
//! Standard mesh formats (STL/OBJ/GLB/glTF) and meshlet sequences are
//! fundamentally different asset types in Bevy and cannot share the same
//! component or handle type:
//!
//! | | Standard mesh | Meshlet |
//! |---|---|---|
//! | Asset type | `Mesh` | `MeshletMesh` |
//! | Handle type | `Handle<Mesh>` | `Handle<MeshletMesh>` |
//! | Spawn component | `Mesh3d(handle)` | `MeshletMesh3d(handle)` |
//! | Renderer | Forward / Deferred PBR | Virtual geometry (Nanite-style) |
//! | Material constraint | Any `Material` | Must impl `meshlet_mesh_fragment_shader` |
//! | Camera requirement | Any | `Msaa::Off` (already set) |
//! | Memory cache | `MeshCache` (refcounted) | Not cached — asset server owns lifetime |
//! | GPU promote step | `promote_to_gpu()` | Not needed — lz4 decode is async in asset server |
//!
//! ### Consequence for this module
//!
//! Because the handle and component types differ, the two paths branch at every
//! major seam in the loader:
//!
//! - **[`SequenceKind`]** — detected from extension; `Meshlet` short-circuits
//!   before `loadfu::MeshFormat` is consulted.
//! - **[`FrameStore`] / [`SeriesEntityState`]** — carry a separate
//!   `meshlet_handles` field (`#[cfg(feature = "meshlet")]`) alongside the
//!   existing `frame_handles: Vec<Option<Vec<Handle<Mesh>>>>`.  The two are
//!   mutually exclusive for a given sequence.
//! - **`drain_load_queue`** — calls `load_meshlet_handle` and writes into
//!   `meshlet_handles`; skips `MeshCache` entirely.
//! - **`manage_frame_window`** — spawns `MeshletMesh3d` entities on the meshlet
//!   path; the standard `Mesh3d` path is skipped.
//! - **`loading_tracker`** — currently tracks `Handle<Mesh>` load events only;
//!   meshlet load tracking is a TODO (meshlet frames are considered loaded
//!   immediately on handle issue for now).
//!
//! When adding new loader features, check each of these seams.
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

// ---------------------------------------------------------------------------
// SequenceKind
// ---------------------------------------------------------------------------

/// The kind of a discovered sequence — either a standard mesh format (handled
/// by loadfu → `Handle<Mesh>`) or a pre-baked meshlet sequence (handled by
/// Bevy's virtual geometry renderer → `Handle<MeshletMesh>`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SequenceKind {
    Mesh(MeshFormat),
    /// Pre-baked `.meshlet` files for Bevy's virtual geometry renderer.
    /// Only available when compiled with `--features meshlet`.
    Meshlet,
}

impl SequenceKind {
    /// Detect from the extension of a single path.
    pub fn from_path(path: &std::path::Path) -> Option<Self> {
        let ext = path.extension()?.to_str()?.to_lowercase();
        if ext == "meshlet_mesh" {
            return Some(SequenceKind::Meshlet);
        }
        MeshFormat::from_path(path).map(SequenceKind::Mesh)
    }

    pub fn label(&self) -> &'static str {
        match self {
            SequenceKind::Mesh(f) => f.label(),
            SequenceKind::Meshlet => "meshlet_mesh",
        }
    }

    pub fn is_meshlet(&self) -> bool {
        matches!(self, SequenceKind::Meshlet)
    }
}

use super::{frames_to_load, LoadSection, SeriesManagerRes, Timeline};
use crate::lib::memory_budget::MemoryBudget;
use crate::lib::series::SeriesId;
use crate::lib::settings::SettingsResource;

// ---------------------------------------------------------------------------
// Shared section-mapping helper
// ---------------------------------------------------------------------------

/// Proportionally map `source_sections` from a timeline with
/// `source_total` frames onto a target with `target_total` frames.
///
/// Used so that non-active series inherit the active timeline's visible
/// region rather than loading their full default `[0, N)` range.
pub(crate) fn map_sections_proportional(
    source_sections: &[LoadSection],
    source_total: usize,
    target_total: usize,
) -> Vec<LoadSection> {
    if source_total == 0 || target_total == 0 || source_sections.is_empty() {
        return Vec::new();
    }
    let ratio = target_total as f64 / source_total as f64;
    source_sections
        .iter()
        .map(|s| {
            let start = ((s.start as f64) * ratio).round() as usize;
            let end = ((s.end as f64) * ratio).round() as usize;
            LoadSection::new(start.min(target_total), end.min(target_total))
        })
        .filter(|s| s.frame_count() > 0)
        .collect()
}

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
            .init_resource::<PendingLoadRequests>()
            .add_message::<FrameLoadedEvent>()
            .add_message::<LoadSequenceRequest>()
            .add_systems(
                Update,
                (
                    flush_pending_loads,
                    handle_load_requests,
                    load_queue::sync_section_loads,
                    load_queue::drain_load_queue,
                    loading_tracker::track_asset_loading,
                    #[cfg(feature = "meshlet")]
                    loading_tracker::track_meshlet_loading,
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

/// Resource-based queue for load requests originating from the egui pass.
///
/// The egui UI runs in `EguiPrimaryContextPass` (after `Update`), so
/// `MessageWriter<LoadSequenceRequest>` messages written there are not
/// visible to the `handle_load_requests` system until the *next* frame's
/// event buffer swap — which may be too late if `sync_section_loads` clears
/// the load queue in between.
///
/// Instead, UI code pushes into this resource and [`flush_pending_loads`]
/// (which runs at the start of the `Update` chain) drains it into the
/// normal message channel where `handle_load_requests` picks it up in the
/// same tick.
#[derive(Resource, Default)]
pub struct PendingLoadRequests {
    pub queue: Vec<LoadSequenceRequest>,
}

/// Drain [`PendingLoadRequests`] into the `LoadSequenceRequest` message
/// channel so they are processed in the same `Update` tick.
fn flush_pending_loads(
    mut pending: ResMut<PendingLoadRequests>,
    mut load_events: MessageWriter<LoadSequenceRequest>,
) {
    for request in pending.queue.drain(..) {
        load_events.write(request);
    }
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

/// Load a `.meshlet` file as a `Handle<MeshletMesh>` via the asset server.
///
/// Requires the `meshlet` feature. The `.meshlet` extension is registered by
/// Bevy's `MeshletMeshLoader` (added by `MeshletPlugin`). The binary format is
/// lz4-compressed and byte-compatible with Bevy's `MeshletMeshSaver`.
#[cfg(feature = "meshlet")]
pub fn load_meshlet_handle(
    asset_server: &AssetServer,
    path: &std::path::Path,
) -> Handle<bevy::pbr::experimental::meshlet::MeshletMesh> {
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

/// Detect sequence kind from the first path, warn on mixed extensions.
fn detect_sequence_kind(paths: &[PathBuf]) -> Option<SequenceKind> {
    let first = paths.first()?;
    let kind = SequenceKind::from_path(first)?;

    for (i, p) in paths.iter().enumerate().skip(1) {
        if let Some(other) = SequenceKind::from_path(p) {
            if other != kind {
                warn!(
                    "Frame {} has kind {:?} but sequence kind is {:?} – it may fail to load",
                    i, other, kind
                );
            }
        }
    }

    Some(kind)
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

        // Detect sequence kind from the first frame.
        let format = match detect_sequence_kind(&request.frame_paths) {
            Some(kind) => {
                info!("Detected format: {}", kind.label());
                kind
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

            // Use the active timeline's sections (proportionally mapped to this
            // series' frame count) so that newly-added series respect the
            // visible region the user has set, rather than loading all frames.
            let sections: Vec<LoadSection> = {
                let own_sections: Vec<LoadSection> = series_mgr
                    .get(series_id)
                    .map(|s| s.timeline.sections.clone())
                    .unwrap_or_default();

                // Check if the series has custom (non-default) sections.
                // A default section covers [0, min(total, budget)) — if that's
                // all we have, inherit from the active timeline instead.
                let is_default = own_sections.len() == 1
                    && own_sections[0].start == 0
                    && own_sections[0].end >= n.min(max_frames);

                if !is_default && !own_sections.is_empty() {
                    own_sections
                } else {
                    let mapped = map_sections_proportional(
                        &timeline.sections,
                        timeline.total_frames,
                        n,
                    );
                    if mapped.is_empty() { own_sections } else { mapped }
                }
            };

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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn map_sections_proportional_same_size() {
        let sections = vec![LoadSection::new(100, 200)];
        let result = map_sections_proportional(&sections, 1000, 1000);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 100);
        assert_eq!(result[0].end, 200);
    }

    #[test]
    fn map_sections_proportional_half_size() {
        let sections = vec![LoadSection::new(100, 200)];
        let result = map_sections_proportional(&sections, 1000, 500);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 50);
        assert_eq!(result[0].end, 100);
    }

    #[test]
    fn map_sections_proportional_double_size() {
        let sections = vec![LoadSection::new(100, 200)];
        let result = map_sections_proportional(&sections, 1000, 2000);
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].start, 200);
        assert_eq!(result[0].end, 400);
    }

    #[test]
    fn map_sections_proportional_clamps_to_target() {
        let sections = vec![LoadSection::new(900, 1000)];
        let result = map_sections_proportional(&sections, 1000, 500);
        assert_eq!(result.len(), 1);
        assert!(result[0].end <= 500);
    }

    #[test]
    fn map_sections_proportional_empty_input() {
        assert!(map_sections_proportional(&[], 1000, 500).is_empty());
    }

    #[test]
    fn map_sections_proportional_zero_source() {
        let sections = vec![LoadSection::new(0, 100)];
        assert!(map_sections_proportional(&sections, 0, 500).is_empty());
    }

    #[test]
    fn map_sections_proportional_zero_target() {
        let sections = vec![LoadSection::new(0, 100)];
        assert!(map_sections_proportional(&sections, 1000, 0).is_empty());
    }

    #[test]
    fn map_sections_proportional_multi_section() {
        let sections = vec![
            LoadSection::new(0, 100),
            LoadSection::new(500, 600),
        ];
        let result = map_sections_proportional(&sections, 10000, 5000);
        assert_eq!(result.len(), 2);
        assert_eq!(result[0].start, 0);
        assert_eq!(result[0].end, 50);
        assert_eq!(result[1].start, 250);
        assert_eq!(result[1].end, 300);
    }

    #[test]
    fn map_sections_proportional_real_world() {
        // Active: 10839 frames, section [3302, 6940)
        // New series: 10225 frames
        let sections = vec![LoadSection::new(3302, 6940)];
        let result = map_sections_proportional(&sections, 10839, 10225);
        assert_eq!(result.len(), 1);
        // Should load ~3434 frames, not all 10225
        let loaded = result[0].frame_count();
        assert!(loaded < 4000, "Should load ~3434 frames, got {}", loaded);
        assert!(loaded > 3000, "Should load ~3434 frames, got {}", loaded);
    }
}
