//! Asset loading progress tracking and glTF mesh extraction.
//!
//! This module contains two systems that track asset loading progress:
//!
//! - **[`extract_gltf_meshes`]** — phase 2 of multi-mesh GLB loading.  When a
//!   `Gltf` asset fires `LoadedWithDependencies`, this system extracts every
//!   mesh primitive and populates [`FrameStore::frame_handles`].
//! - **[`track_asset_loading`]** — tracks STL mesh loading via
//!   `AssetEvent<Mesh>`, marks frames as loaded, creates the shared material,
//!   estimates frame size for memory budgeting, and detects completion.
//!
//! Also contains the pure helpers [`count_requested_frames`] and
//! [`default_material_properties`] shared by both systems.

use bevy::asset::{AssetEvent, LoadState};
use bevy::gltf::{Gltf, GltfMesh};
use bevy::mesh::Mesh;
use bevy::prelude::*;

use super::assets::{AssetIndex, FrameStore, LoaderConfig};
use super::FrameLoadedEvent;
use crate::lib::memory_budget::{estimate_mesh_gpu_bytes_from_vertices, MemoryBudget};
use crate::lib::sequence::SequenceManager;

// ---------------------------------------------------------------------------
// Pure functions
// ---------------------------------------------------------------------------

/// Count how many frames have been requested (have either a mesh handle or
/// an intermediate glTF handle).
///
/// Used by [`track_asset_loading`] and [`extract_gltf_meshes`] to determine
/// when loading is complete.
pub(crate) fn count_requested_frames(
    frame_handles: &[Option<Vec<Handle<Mesh>>>],
    gltf_handles: &[Option<Handle<Gltf>>],
) -> usize {
    frame_handles
        .iter()
        .zip(gltf_handles.iter())
        .filter(|(fh, gh)| fh.is_some() || gh.is_some())
        .count()
}

/// Default material properties used for sequence mesh entities.
///
/// Extracted so both [`extract_gltf_meshes`] and [`track_asset_loading`]
/// share the same definition.
pub fn default_material_properties() -> StandardMaterial {
    StandardMaterial {
        base_color: Color::srgb(0.3, 0.5, 0.8),
        perceptual_roughness: 0.4,
        metallic: 0.1,
        cull_mode: None, // Double-sided rendering
        ..default()
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// System that watches for loaded [`Gltf`] assets and extracts all mesh
/// primitives from them, populating [`FrameStore::frame_handles`].
///
/// This is phase 2 of multi-mesh GLB loading.  When a `Gltf` asset fires
/// `LoadedWithDependencies`, all its sub-assets (meshes, materials, etc.)
/// are already loaded, so we can immediately extract `Handle<Mesh>` from
/// every primitive and mark the frame as loaded.
pub(super) fn extract_gltf_meshes(
    mut events: MessageReader<AssetEvent<Gltf>>,
    mut frame_store: ResMut<FrameStore>,
    mut loader_config: ResMut<LoaderConfig>,
    mut asset_index: ResMut<AssetIndex>,
    mut frame_loaded_events: MessageWriter<FrameLoadedEvent>,
    gltf_assets: Res<Assets<Gltf>>,
    gltf_mesh_assets: Res<Assets<GltfMesh>>,
    meshes: Res<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut memory_budget: ResMut<MemoryBudget>,
    sequence_manager: Res<SequenceManager>,
) {
    if !frame_store.loading {
        return;
    }

    for event in events.read() {
        let AssetEvent::LoadedWithDependencies { id } = event else {
            continue;
        };

        let Some(&frame_idx) = asset_index.gltf_id_to_frame.get(id) else {
            continue;
        };

        // Access the loaded Gltf asset to extract mesh primitives.
        let gltf_handle = match &asset_index.gltf_handles[frame_idx] {
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
            asset_index.asset_id_to_frame.insert(handle.id(), frame_idx);
        }

        // Store the mesh handles.
        frame_store.frame_handles[frame_idx] = Some(mesh_handles.clone());

        // Mark frame as loaded.
        if !frame_store.loaded_frames[frame_idx] {
            frame_store.loaded_frames[frame_idx] = true;
            frame_store.loaded_count += 1;
        }

        frame_loaded_events.write(FrameLoadedEvent {
            frame_index: frame_idx,
            success: true,
        });

        // Create shared material on first loaded frame.
        if loader_config.shared_material.is_none() {
            loader_config.shared_material = Some(materials.add(default_material_properties()));
        }

        // Estimate frame size from first loaded mesh for memory budgeting.
        if frame_store.estimated_frame_bytes == 0 {
            let mut total_vertices = 0usize;
            for handle in &mesh_handles {
                if let Some(mesh) = meshes.get(handle) {
                    total_vertices += mesh.count_vertices();
                }
            }
            if total_vertices > 0 {
                let estimated_bytes = estimate_mesh_gpu_bytes_from_vertices(total_vertices);
                frame_store.estimated_frame_bytes = estimated_bytes;
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
                loader_config.coord_transform = sequence.source_orientation.to_transform();
                debug!(
                    "Applying coordinate transformation: {}",
                    sequence.source_orientation.description()
                );
            }
        }

        // Log progress at intervals.
        let requested_count =
            count_requested_frames(&frame_store.frame_handles, &asset_index.gltf_handles);
        if frame_store.loaded_count % 10 == 0 || frame_store.loaded_count == requested_count {
            info!(
                "Loading progress (GLB): {}/{} frames loaded ({} meshes in frame {})",
                frame_store.loaded_count, requested_count, mesh_count, frame_idx,
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
pub(super) fn track_asset_loading(
    mut events: MessageReader<AssetEvent<Mesh>>,
    mut frame_store: ResMut<FrameStore>,
    mut loader_config: ResMut<LoaderConfig>,
    asset_index: Res<AssetIndex>,
    mut frame_loaded_events: MessageWriter<FrameLoadedEvent>,
    asset_server: Res<AssetServer>,
    sequence_manager: Res<SequenceManager>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    meshes: Res<Assets<Mesh>>,
    mut memory_budget: ResMut<MemoryBudget>,
) {
    if !frame_store.loading {
        return;
    }

    // Count how many frames we actually requested (have a handle or gltf handle)
    let requested_count =
        count_requested_frames(&frame_store.frame_handles, &asset_index.gltf_handles);

    for event in events.read() {
        match event {
            AssetEvent::LoadedWithDependencies { id } => {
                // O(1) lookup via reverse index — only STL frames are registered here.
                let Some(&index) = asset_index.asset_id_to_frame.get(id) else {
                    continue;
                };
                // Skip if this frame was already marked loaded (e.g. by extract_gltf_meshes).
                if frame_store.loaded_frames[index] {
                    continue;
                }
                let handle = match frame_store.frame_handles[index].as_ref() {
                    Some(handles) => match handles.first() {
                        Some(h) => h.clone(),
                        None => continue,
                    },
                    None => continue,
                };

                frame_store.loaded_count += 1;
                frame_store.loaded_frames[index] = true;

                frame_loaded_events.write(FrameLoadedEvent {
                    frame_index: index,
                    success: true,
                });

                // Create shared material on first loaded frame
                if loader_config.shared_material.is_none() {
                    loader_config.shared_material =
                        Some(materials.add(default_material_properties()));
                }

                // Estimate frame size from first loaded mesh for memory budgeting
                if frame_store.estimated_frame_bytes == 0 {
                    if let Some(mesh) = meshes.get(&handle) {
                        let vertex_count = mesh.count_vertices();
                        let estimated_bytes = estimate_mesh_gpu_bytes_from_vertices(vertex_count);
                        frame_store.estimated_frame_bytes = estimated_bytes;
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
                        loader_config.coord_transform = sequence.source_orientation.to_transform();
                        debug!(
                            "Applying coordinate transformation: {}",
                            sequence.source_orientation.description()
                        );
                    }
                }

                // Log progress at intervals
                if frame_store.loaded_count % 10 == 0 || frame_store.loaded_count == requested_count
                {
                    info!(
                        "Loading progress: {}/{} frames loaded",
                        frame_store.loaded_count, requested_count,
                    );
                }
            }
            _ => {}
        }
    }

    // Check if all requested frames are loaded
    if frame_store.loaded_count >= requested_count && requested_count > 0 && frame_store.loading {
        frame_store.loading = false;
        info!("════════════════════════════════════════════════════════════════");
        info!(
            "✓ SECTION LOADING COMPLETE: {}/{} frames loaded (budget-limited)",
            frame_store.loaded_count, frame_store.total_frames,
        );
        info!("════════════════════════════════════════════════════════════════");
    }

    // Check for failed loads with detailed information
    if frame_store.loading {
        let mut failed_count = 0;
        let mut failed_indices = Vec::new();

        // Check mesh handles (STL)
        for (index, handle_opt) in frame_store.frame_handles.iter().enumerate() {
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
        for (index, handle_opt) in asset_index.gltf_handles.iter().enumerate() {
            let Some(handle) = handle_opt else { continue };
            if frame_store.loaded_frames[index] {
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ── count_requested_frames ────────────────────────────────────────

    #[test]
    fn test_count_requested_none() {
        let fh: Vec<Option<Vec<Handle<Mesh>>>> = vec![None, None, None];
        let gh: Vec<Option<Handle<Gltf>>> = vec![None, None, None];
        assert_eq!(count_requested_frames(&fh, &gh), 0);
    }

    #[test]
    fn test_count_requested_empty() {
        let fh: Vec<Option<Vec<Handle<Mesh>>>> = vec![];
        let gh: Vec<Option<Handle<Gltf>>> = vec![];
        assert_eq!(count_requested_frames(&fh, &gh), 0);
    }

    // ── default_material_properties ───────────────────────────────────

    #[test]
    fn test_default_material_is_double_sided() {
        let mat = default_material_properties();
        assert_eq!(
            mat.cull_mode, None,
            "Default material should be double-sided"
        );
    }

    #[test]
    fn test_default_material_color() {
        let mat = default_material_properties();
        assert_eq!(mat.base_color, Color::srgb(0.3, 0.5, 0.8));
    }
}
