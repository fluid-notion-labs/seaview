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

use bevy::asset::{AssetEvent, LoadState};
use bevy::gltf::GltfAssetLabel;
use bevy::prelude::*;
use std::path::PathBuf;

use super::SequenceManager;

use crate::app::ui::state::RenderConfig;

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
                (handle_load_requests, track_asset_loading).chain(),
            )
            .add_systems(
                PostUpdate,
                manage_frame_window,
            );
    }
}

/// Resource that holds all loaded mesh handles for the current sequence
#[derive(Resource, Default)]
pub struct SequenceAssets {
    /// Handles to all loaded mesh frames (indexed by frame number)
    pub frame_handles: Vec<Handle<Mesh>>,
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
}

impl SequenceAssets {
    /// Reset the sequence assets for a new sequence
    pub fn reset(&mut self) {
        self.frame_handles.clear();
        self.loaded_frames.clear();
        self.loaded_count = 0;
        self.total_frames = 0;
        self.loading = false;
        self.displayed_frame = None;
        self.format = None;
        self.frame_entities.clear();
        self.shared_material = None;
        self.coord_transform = Transform::default();
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
        self.frame_handles.get(index)
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
fn load_mesh_handle(
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
) {
    for request in load_requests.read() {
        debug!("Loading sequence with {} frames", request.frame_paths.len());

        // Detect file format from the first frame.
        let format = match detect_sequence_format(&request.frame_paths) {
            Some(fmt) => {
                debug!("Detected sequence format: {:?}", fmt);
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
        // Pre-allocate vecs
        sequence_assets.frame_entities = vec![None; n];
        sequence_assets.loaded_frames = vec![false; n];

        // Store base path from first frame
        if let Some(first_path) = request.frame_paths.first() {
            sequence_assets.base_path = first_path.parent().map(|p| p.to_path_buf());
        }

        // Load all frames using Bevy's asset server.
        // Bevy handles parallel / async loading automatically.
        for (idx, path) in request.frame_paths.iter().enumerate() {
            let filename = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown");

            trace!(
                "Frame {}: Loading '{}' as {:?} from path {:?}",
                idx, filename, format, path
            );

            let handle = load_mesh_handle(&asset_server, filename, format);
            sequence_assets.frame_handles.push(handle);
        }

        debug!("Mesh display entity will be created when first frame loads");

        debug!(
            "Started loading {} {:?} frames from {:?}",
            sequence_assets.total_frames, format, sequence_assets.base_path
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
) {
    if !sequence_assets.loading {
        return;
    }

    for event in events.read() {
        match event {
            AssetEvent::LoadedWithDependencies { id } => {
                // Find which frame this corresponds to
                for (index, handle) in sequence_assets.frame_handles.iter().enumerate() {
                    if handle.id() == *id {
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
                        let progress = sequence_assets.progress();
                        if sequence_assets.loaded_count % 10 == 0
                            || sequence_assets.loaded_count == sequence_assets.total_frames
                        {
                            debug!(
                                "Loading progress: {}/{} ({:.1}%)",
                                sequence_assets.loaded_count,
                                sequence_assets.total_frames,
                                progress * 100.0
                            );
                        }

                        break;
                    }
                }
            }
            _ => {}
        }
    }

    // Check if loading is complete
    if sequence_assets.is_fully_loaded() && sequence_assets.loading {
        sequence_assets.loading = false;
        info!(
            "════════════════════════════════════════════════════════════════"
        );
        info!(
            "✓ SEQUENCE LOADING COMPLETE: {} frames loaded",
            sequence_assets.loaded_count,
        );
        info!(
            "════════════════════════════════════════════════════════════════"
        );
    }

    // Check for failed loads with detailed information
    if sequence_assets.loading {
        let mut failed_count = 0;
        let mut failed_indices = Vec::new();

        for (index, handle) in sequence_assets.frame_handles.iter().enumerate() {
            let state = asset_server.load_state(handle.id());
            match state {
                LoadState::Failed(ref err) => {
                    failed_count += 1;
                    failed_indices.push(index);
                    if failed_count <= 3 {
                        // Log first 3 failures in detail
                        error!("Frame {} load failed: {:?}", index, err);
                    }
                }
                LoadState::NotLoaded => {
                    if sequence_assets.frame_handles.len() <= 10 || index % 20 == 0 {
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

/// System that manages the ring-buffer of spawned frame entities.
///
/// This is the **sole owner of visibility** for sequence mesh entities.
/// Runs in `PostUpdate` so that `sequence_manager.current_frame` is finalised
/// for this tick (all `Update` writers have run).
///
/// Keeps at most `RenderConfig::frame_window_size` entities spawned, centred on
/// the current frame. Uses O(delta) despawn/spawn by tracking the previous
/// window range — only entities entering or leaving the window are touched.
/// Sets visibility directly via query so changes take effect this frame.
fn manage_frame_window(
    mut commands: Commands,
    mut sequence_assets: ResMut<SequenceAssets>,
    sequence_manager: Res<SequenceManager>,
    render_config: Res<RenderConfig>,
    mut visibility_query: Query<&mut Visibility, With<SequenceMeshDisplay>>,
    mut prev_window: Local<(usize, usize)>,
) {
    if sequence_assets.total_frames == 0 {
        return;
    }
    // Need shared material before we can spawn anything
    let Some(material) = sequence_assets.shared_material.clone() else {
        return;
    };

    let current = sequence_manager.current_frame;
    let window = render_config.frame_window_size;
    let (win_lo, win_hi) = sequence_assets.window_range(current, window);
    let transform = sequence_assets.coord_transform;

    let (old_lo, old_hi) = *prev_window;

    // 1. Despawn entities that left the window (O(delta), not O(total_frames)).
    //    Two ranges to check: old left side that fell off, old right side that fell off.
    //    Left eviction: [old_lo, min(old_hi, win_lo))
    //    Right eviction: [max(old_lo, win_hi), old_hi)
    let evict_left_end = old_hi.min(win_lo);
    for idx in old_lo..evict_left_end {
        if let Some(entity) = sequence_assets.frame_entities[idx].take() {
            if let Ok(mut vis) = visibility_query.get_mut(entity) {
                *vis = Visibility::Hidden;
            }
            commands.entity(entity).despawn();
        }
    }
    let evict_right_start = old_lo.max(win_hi);
    for idx in evict_right_start..old_hi {
        if let Some(entity) = sequence_assets.frame_entities[idx].take() {
            if let Ok(mut vis) = visibility_query.get_mut(entity) {
                *vis = Visibility::Hidden;
            }
            commands.entity(entity).despawn();
        }
    }

    // 2. Spawn entities inside the window that are loaded but not yet spawned.
    //    Newly-spawned entities won't be visible to queries until Commands are
    //    applied, but they are spawned with the correct Visibility component.
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
        trace!("Spawned frame {} entity {:?} (window [{}, {}))", idx, entity, win_lo, win_hi);
    }

    // 3. Set visibility on all already-existing entities in the window.
    //    Only the current frame is Visible; everything else is Hidden.
    //    This is a direct query mutation — takes effect this frame, no flicker.
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

    sequence_assets.displayed_frame = Some(current);
    *prev_window = (win_lo, win_hi);
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
