//! Asset loading progress tracking.
//!
//! This module contains the system that tracks asset loading progress:
//!
//! - **[`track_asset_loading`]** — tracks mesh loading via `AssetEvent<Mesh>`
//!   for **all** formats (STL, OBJ, GLB, glTF).  With loadfu, every format
//!   produces `Handle<Mesh>` directly — there is no longer a two-phase glTF
//!   extraction step.
//!
//! Also contains the pure helper [`default_material_properties`].

use bevy::asset::{AssetEvent, LoadState};
use bevy::mesh::Mesh;
use bevy::prelude::*;

use super::assets::{AssetIndex, FrameStore, LoaderConfig};
use super::FrameLoadedEvent;
use crate::lib::memory_budget::{estimate_mesh_gpu_bytes_from_vertices, MemoryBudget};
use crate::lib::sequence::SequenceManager;

// ---------------------------------------------------------------------------
// Pure functions
// ---------------------------------------------------------------------------

/// Default material properties used for sequence mesh entities.
///
/// Extracted so both the loader and spawn systems share the same definition.
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

/// System that tracks asset loading progress for all formats.
///
/// Uses the `asset_id_to_frame` reverse index for O(1) lookups instead of
/// scanning the full handle list on every event.
///
/// With loadfu, **all** formats (STL, OBJ, GLB, glTF) produce `Handle<Mesh>`
/// directly and register their asset IDs up-front in `drain_load_queue`.
/// There is no longer a separate glTF extraction phase.
///
/// Does NOT spawn entities — that is handled by `manage_frame_window`.
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

    // Count how many frames we actually requested (have a handle).
    let requested_count = frame_store
        .frame_handles
        .iter()
        .filter(|h| h.is_some())
        .count();

    for event in events.read() {
        match event {
            AssetEvent::LoadedWithDependencies { id } => {
                // O(1) lookup via reverse index.
                let Some(&index) = asset_index.asset_id_to_frame.get(id) else {
                    continue;
                };
                // Skip if this frame was already marked loaded.
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

                // Estimate frame size from first loaded mesh for memory budgeting,
                // and flag degenerate frames (runt files with near-zero geometry).
                if let Some(mesh) = meshes.get(&handle) {
                    let vertex_count = mesh.count_vertices();

                    // Set baseline estimate from the first real frame
                    if frame_store.estimated_frame_bytes == 0 {
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

                    // Flag degenerate frames: <10% of expected vertex count
                    let expected_verts = frame_store.estimated_frame_bytes as usize / 32; // rough: 32 bytes/vert
                    if expected_verts > 0 && vertex_count < expected_verts / 10 {
                        if index < frame_store.degenerate_frames.len() {
                            frame_store.degenerate_frames[index] = true;
                        } else {
                            // Grow the vec if watch mode added frames without resizing
                            frame_store.degenerate_frames.resize(index + 1, false);
                            frame_store.degenerate_frames[index] = true;
                        }
                        warn!(
                            "⚠ Degenerate frame {}: {} vertices (expected ~{}) — will be skipped",
                            index, vertex_count, expected_verts,
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
                    debug!(
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
