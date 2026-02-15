//! Mesh sequence loading using Bevy's native asset system
//!
//! This module provides loading functionality for mesh sequences supporting
//! multiple file formats:
//!
//! - **glTF / GLB**: Loaded via Bevy's built-in `bevy_gltf` crate using
//!   [`GltfAssetLabel`] to extract the first mesh primitive.
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

use bevy::asset::{AssetEvent, LoadState};
use bevy::gltf::GltfAssetLabel;
use bevy::prelude::*;
use std::path::PathBuf;

use super::{compute_spawn_windows, frames_to_load, SequenceManager, Timeline};

use crate::app::ui::state::RenderConfig;
use crate::lib::memory_budget::{estimate_mesh_gpu_bytes_from_vertices, MemoryBudget};

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

/// Plugin for mesh sequence loading
pub struct SequenceLoaderPlugin;

impl Plugin for SequenceLoaderPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SequenceAssets>()
            .add_message::<FrameLoadedEvent>()
            .add_message::<LoadSequenceRequest>()
            .add_systems(
                Update,
                (
                    handle_load_requests,
                    sync_section_loads,
                    track_asset_loading,
                )
                    .chain(),
            )
            .add_systems(PostUpdate, manage_frame_window);
    }
}

/// Resource that holds all loaded mesh handles for the current sequence
#[derive(Resource, Default)]
pub struct SequenceAssets {
    /// Handles to loaded mesh frames (indexed by frame number).
    /// `None` means the asset has not been requested from the server yet.
    pub frame_handles: Vec<Option<Handle<Mesh>>>,
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
    /// Spawned entity per frame index (`None` = not currently spawned)
    pub frame_entities: Vec<Option<Entity>>,
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

    /// Get the mesh handle for a specific frame
    pub fn get_frame(&self, index: usize) -> Option<&Handle<Mesh>> {
        self.frame_handles.get(index).and_then(|opt| opt.as_ref())
    }

    /// Check if a specific frame's asset has finished loading
    pub fn is_frame_loaded(&self, index: usize) -> bool {
        self.loaded_frames.get(index).copied().unwrap_or(false)
    }

    /// Get the entity for the currently displayed frame
    pub fn displayed_entity(&self) -> Option<Entity> {
        self.displayed_frame
            .and_then(|idx| self.frame_entities.get(idx))
            .and_then(|opt| *opt)
    }

    /// Compute the window range `[lo, hi)` centered on `center` with the given
    /// half-width, clamped to `[0, total_frames)`.
    pub fn window_range(&self, center: usize, window_size: usize) -> (usize, usize) {
        let half = window_size / 2;
        let lo = center.saturating_sub(half);
        let hi = (center + half + 1).min(self.total_frames);
        (lo, hi)
    }
}

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

/// Component to mark the mesh display entity
#[derive(Component)]
pub struct SequenceMeshDisplay;

// ---------------------------------------------------------------------------
// Helper: derive the asset path string for a given frame file
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
pub fn load_mesh_handle(
    asset_server: &AssetServer,
    filename: &str,
    format: MeshFileFormat,
) -> Handle<Mesh> {
    let asset_path = format!("seq://{}", filename);

    match format {
        MeshFileFormat::Stl => {
            // The StlLoader registered for ".stl" will produce a Mesh directly.
            asset_server.load(asset_path)
        }
        MeshFileFormat::Gltf | MeshFileFormat::Glb => {
            // Use GltfAssetLabel to extract Mesh 0 / Primitive 0.
            asset_server.load(
                GltfAssetLabel::Primitive {
                    mesh: 0,
                    primitive: 0,
                }
                .from_asset(asset_path),
            )
        }
    }
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

/// System that handles sequence load requests
fn handle_load_requests(
    mut commands: Commands,
    mut load_requests: MessageReader<LoadSequenceRequest>,
    mut sequence_assets: ResMut<SequenceAssets>,
    asset_server: Res<AssetServer>,
    mut timeline: ResMut<Timeline>,
    memory_budget: Res<MemoryBudget>,
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
            if let Some(entity) = entity_opt {
                commands.entity(*entity).despawn();
            }
        }

        // Reset assets for new sequence
        sequence_assets.reset();
        let n = request.frame_paths.len();
        sequence_assets.total_frames = n;
        sequence_assets.loading = true;
        sequence_assets.format = Some(format);
        // Pre-allocate vecs — handles start as None (loaded on demand)
        sequence_assets.frame_handles = vec![None; n];
        sequence_assets.frame_entities = vec![None; n];
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

        // Only load frames within the timeline's load sections (budget-aware).
        let indices = frames_to_load(&timeline.sections, n);
        info!(
            "Loading {} of {} frames within budget sections",
            indices.len(),
            n
        );
        for idx in &indices {
            let path = &request.frame_paths[*idx];
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            trace!(
                "Frame {}: Loading '{}' as {:?} from path {:?}",
                idx,
                filename,
                format,
                path
            );

            let handle = load_mesh_handle(&asset_server, filename, format);
            sequence_assets.frame_handles[*idx] = Some(handle);
        }

        info!(
            "Asset loading started: {} {:?} frames (of {} total) from {:?}",
            indices.len(),
            format,
            sequence_assets.total_frames,
            sequence_assets.base_path
        );
    }
}

/// System that tracks asset loading progress.
///
/// Does NOT spawn entities — that is handled by `manage_frame_window`.
/// This system only marks frames as loaded and creates the shared material.
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

    // Count how many frames we actually requested (have Some handle)
    let requested_count = sequence_assets
        .frame_handles
        .iter()
        .filter(|h| h.is_some())
        .count();

    for event in events.read() {
        match event {
            AssetEvent::LoadedWithDependencies { id } => {
                // Find which frame this corresponds to — skip None entries
                let found = sequence_assets
                    .frame_handles
                    .iter()
                    .enumerate()
                    .filter_map(|(i, opt)| opt.as_ref().map(|h| (i, h)))
                    .find(|(_, handle)| handle.id() == *id)
                    .map(|(index, handle)| (index, handle.clone()));

                if let Some((index, handle)) = found {
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
                            let estimated_bytes =
                                estimate_mesh_gpu_bytes_from_vertices(vertex_count);
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

        for (index, handle_opt) in sequence_assets.frame_handles.iter().enumerate() {
            let Some(handle) = handle_opt else { continue };
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

        if failed_count > 0 {
            warn!(
                "{} frames failed to load (indices: {:?}...)",
                failed_count,
                &failed_indices[..failed_indices.len().min(5)]
            );
        }
    }
}

/// Watches for timeline section changes and loads/unloads frame assets to
/// match.  Frames entering a section get an asset load kicked off; frames
/// leaving all sections have their handle dropped so the asset server can
/// reclaim the memory.
fn sync_section_loads(
    mut sequence_assets: ResMut<SequenceAssets>,
    asset_server: Res<AssetServer>,
    timeline: Res<Timeline>,
) {
    if sequence_assets.total_frames == 0 {
        return;
    }
    let Some(format) = sequence_assets.format else {
        return;
    };

    let needed = frames_to_load(&timeline.sections, sequence_assets.total_frames);

    let mut loads_triggered = 0usize;

    // Load frames that are in sections but don't have a handle yet
    for &idx in &needed {
        if sequence_assets.frame_handles[idx].is_none() {
            let path = &sequence_assets.frame_paths[idx];
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");
            let handle = load_mesh_handle(&asset_server, filename, format);
            sequence_assets.frame_handles[idx] = Some(handle);
            sequence_assets.loaded_frames[idx] = false;
            loads_triggered += 1;
        }
    }

    // Unload frames that are outside all sections (drop handle, reclaim memory)
    // Build a fast lookup of needed indices
    let mut is_needed = vec![false; sequence_assets.total_frames];
    for &idx in &needed {
        is_needed[idx] = true;
    }
    let mut unloads = 0usize;
    for idx in 0..sequence_assets.total_frames {
        if !is_needed[idx] && sequence_assets.frame_handles[idx].is_some() {
            sequence_assets.frame_handles[idx] = None;
            if sequence_assets.loaded_frames[idx] {
                sequence_assets.loaded_frames[idx] = false;
                sequence_assets.loaded_count = sequence_assets.loaded_count.saturating_sub(1);
            }
            unloads += 1;
        }
    }

    if loads_triggered > 0 || unloads > 0 {
        sequence_assets.loading = true;
        info!(
            "Section sync: {} loads triggered, {} handles dropped",
            loads_triggered, unloads
        );
    }
}

/// System that manages the ring-buffer of spawned frame entities.
///
/// This is the **sole owner of visibility** for sequence mesh entities.
/// Runs in `PostUpdate` so that `timeline.playhead` is finalised for this
/// tick (all `Update` writers have run).
///
/// Keeps at most `RenderConfig::frame_window_size` entities spawned, centred on
/// the current frame. Uses O(delta) despawn/spawn by tracking the previous
/// window range — only entities entering or leaving the window are touched.
/// Sets visibility directly via query so changes take effect this frame.
fn manage_frame_window(
    mut commands: Commands,
    mut sequence_assets: ResMut<SequenceAssets>,
    timeline: Res<Timeline>,
    render_config: Res<RenderConfig>,
    mut visibility_query: Query<&mut Visibility, With<SequenceMeshDisplay>>,
    mut prev_windows: Local<Vec<(usize, usize)>>,
    mut memory_budget: ResMut<MemoryBudget>,
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
    for &(old_lo, old_hi) in prev_windows.iter() {
        for idx in old_lo..old_hi {
            if in_windows(idx, &new_windows) {
                continue; // still needed
            }
            if let Some(entity) = sequence_assets.frame_entities[idx].take() {
                if let Ok(mut vis) = visibility_query.get_mut(entity) {
                    *vis = Visibility::Hidden;
                }
                commands.entity(entity).despawn();
                if frame_bytes > 0 {
                    memory_budget.record_frame_unloaded(frame_bytes);
                }
            }
        }
    }

    // 2. Spawn entities that are in new windows but were NOT in old windows.
    for &(win_lo, win_hi) in &new_windows {
        for idx in win_lo..win_hi {
            if sequence_assets.frame_entities[idx].is_some() {
                continue; // already spawned
            }
            if !sequence_assets.is_frame_loaded(idx) {
                continue; // asset not ready yet
            }
            let Some(handle) = sequence_assets.get_frame(idx).cloned() else {
                continue;
            };

            let visibility = if idx == current {
                Visibility::Visible
            } else {
                Visibility::Hidden
            };

            let entity = commands
                .spawn((
                    Mesh3d(handle),
                    MeshMaterial3d(material.clone()),
                    transform,
                    SequenceMeshDisplay,
                    visibility,
                    Name::new(format!("Frame {}", idx)),
                ))
                .id();

            sequence_assets.frame_entities[idx] = Some(entity);

            if frame_bytes > 0 {
                memory_budget.record_frame_loaded(frame_bytes);
            }

            debug!(
                "Spawned frame {} entity {:?} (window [{}, {}))",
                idx, entity, win_lo, win_hi
            );
        }
    }

    // Log window changes
    if new_windows != *prev_windows {
        debug!(
            "Frame windows changed: {:?} → {:?}, current frame: {}",
            *prev_windows, new_windows, current
        );
    }

    // 3. Set visibility on all entities across all windows.
    //    Only the current frame is Visible; everything else is Hidden.
    for &(win_lo, win_hi) in &new_windows {
        for idx in win_lo..win_hi {
            if let Some(Some(entity)) = sequence_assets.frame_entities.get(idx) {
                if let Ok(mut vis) = visibility_query.get_mut(*entity) {
                    let desired = if idx == current {
                        Visibility::Visible
                    } else {
                        Visibility::Hidden
                    };
                    if *vis != desired {
                        *vis = desired;
                    }
                }
            }
        }
    }

    sequence_assets.displayed_frame = Some(current);
    *prev_windows = new_windows;
}

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
