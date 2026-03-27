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
use super::mesh_cache::MeshCache;
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

/// Build a `StandardMaterial` using values from the current material config.
///
/// Called when creating the shared material so it starts with the user's
/// persisted settings (base color, roughness, metallic, etc.) instead of
/// hardcoded defaults.
pub fn material_from_config(
    base_color: Color,
    opacity: f32,
    perceptual_roughness: f32,
    metallic: f32,
    reflectance: f32,
    emissive: Color,
    emissive_intensity: f32,
    double_sided: bool,
) -> StandardMaterial {
    let mut color = base_color;
    color.set_alpha(opacity);

    let emissive_linear = if emissive_intensity > 0.0 {
        (emissive.to_linear() * emissive_intensity).into()
    } else {
        Color::BLACK.into()
    };

    let cull_mode = if double_sided {
        None
    } else {
        Some(bevy::render::render_resource::Face::Back)
    };

    let alpha_mode = if opacity < 1.0 {
        AlphaMode::Blend
    } else {
        AlphaMode::Opaque
    };

    StandardMaterial {
        base_color: color,
        perceptual_roughness,
        metallic,
        reflectance,
        emissive: emissive_linear,
        cull_mode,
        double_sided,
        alpha_mode,
        ..default()
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// System that tracks asset loading progress for all formats.
///
/// Uses the `asset_id_to_frame` reverse index for O(1) lookups.  Each entry
/// maps `AssetId<Mesh> → (SeriesId, frame_index)`.  Loaded state is written
/// into [`SeriesFrameStore`] for the owning series, and mirrored into
/// [`FrameStore`] for the active series (backward compat with UI consumers).
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
    material_config: Res<crate::app::ui::state::MaterialConfig>,
    mut mesh_cache: ResMut<MeshCache>,
    mut series_fs: ResMut<super::series_frame_store::SeriesFrameStore>,
    series_mgr: Res<crate::lib::sequence::SeriesManagerRes>,
) {
    let active_id = series_mgr.active_id();

    // Count requested frames for active-series progress logging.
    let requested_count = frame_store
        .frame_handles
        .iter()
        .filter(|h| h.is_some())
        .count();

    for event in events.read() {
        match event {
            AssetEvent::LoadedWithDependencies { id } => {
                // O(1) lookup: which (series, frame) does this asset belong to?
                let Some(&(series_id, index)) = asset_index.asset_id_to_frame.get(id) else {
                    continue;
                };

                // Resolve the owning series state.
                // Sentinel u64::MAX means the load was issued via the legacy
                // active-series FrameStore path.
                let is_active_sentinel = series_id == crate::lib::series::SeriesId::from_raw(u64::MAX);
                let effective_series = if is_active_sentinel { active_id } else { Some(series_id) };

                // Write into SeriesFrameStore for the owning series.
                if let Some(sid) = effective_series {
                    if let Some(state) = series_fs.get_mut(&sid) {
                        if index < state.loaded_frames.len() && !state.loaded_frames[index] {
                            state.loaded_frames[index] = true;
                            state.loaded_count += 1;

                            // Propagate frame-size estimate into the series state.
                            if state.estimated_frame_bytes == 0 {
                                if let Some(handle_vec) = state.frame_handles.get(index).and_then(|h| h.as_ref()) {
                                    if let Some(first_handle) = handle_vec.first() {
                                        if let Some(mesh) = meshes.get(first_handle) {
                                            let est = estimate_mesh_gpu_bytes_from_vertices(mesh.count_vertices());
                                            state.estimated_frame_bytes = est;
                                        }
                                    }
                                }
                            }

                            // Flag degenerate frames in the series state.
                            let expected_verts = state.estimated_frame_bytes as usize / 32;
                            if expected_verts > 0 {
                                if let Some(handle_vec) = state.frame_handles.get(index).and_then(|h| h.as_ref()) {
                                    if let Some(first_handle) = handle_vec.first() {
                                        if let Some(mesh) = meshes.get(first_handle) {
                                            if mesh.count_vertices() < expected_verts / 10 {
                                                if index < state.degenerate_frames.len() {
                                                    state.degenerate_frames[index] = true;
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                // Mirror into FrameStore for the active series only.
                let is_active = effective_series == active_id;
                if is_active {
                    if index < frame_store.loaded_frames.len() && !frame_store.loaded_frames[index] {
                        let handle = match frame_store.frame_handles.get(index).and_then(|h| h.as_ref()) {
                            Some(handles) => match handles.first() {
                                Some(h) => h.clone(),
                                None => continue,
                            },
                            None => {
                                // May have been routed through per-series path;
                                // grab from series_fs instead.
                                if let Some(sid) = effective_series {
                                    if let Some(state) = series_fs.get(&sid) {
                                        match state.frame_handles.get(index).and_then(|h| h.as_ref()).and_then(|v| v.first()) {
                                            Some(h) => h.clone(),
                                            None => continue,
                                        }
                                    } else { continue; }
                                } else { continue; }
                            }
                        };

                        frame_store.loaded_count += 1;
                        frame_store.loaded_frames[index] = true;

                        frame_loaded_events.write(FrameLoadedEvent {
                            frame_index: index,
                            success: true,
                        });

                        if let Some(cache_entry) = mesh_cache.get_by_asset_id_mut(id) {
                            cache_entry.loaded = true;
                        }

                        // Create shared material on first loaded frame.
                        if loader_config.shared_material.is_none() {
                            let mat = material_from_config(
                                material_config.base_color,
                                material_config.opacity,
                                material_config.perceptual_roughness,
                                material_config.metallic,
                                material_config.reflectance,
                                material_config.emissive,
                                material_config.emissive_intensity,
                                material_config.double_sided,
                            );
                            loader_config.shared_material = Some(materials.add(mat));
                        }

                        // Frame-size estimate and degenerate detection.
                        if let Some(mesh) = meshes.get(&handle) {
                            let vertex_count = mesh.count_vertices();

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

                            if let Some(cache_entry) = mesh_cache.get_by_asset_id_mut(id) {
                                let estimated = estimate_mesh_gpu_bytes_from_vertices(mesh.count_vertices());
                                cache_entry.estimated_bytes = estimated;
                            }

                            let expected_verts = frame_store.estimated_frame_bytes as usize / 32;
                            if expected_verts > 0 && vertex_count < expected_verts / 10 {
                                if index < frame_store.degenerate_frames.len() {
                                    frame_store.degenerate_frames[index] = true;
                                } else {
                                    frame_store.degenerate_frames.resize(index + 1, false);
                                    frame_store.degenerate_frames[index] = true;
                                }
                                warn!(
                                    "⚠ Degenerate frame {}: {} vertices (expected ~{}) — will be skipped",
                                    index, vertex_count, expected_verts,
                                );
                            }
                        }

                        // Cache coordinate transform on first active-series frame.
                        if index == 0 {
                            if let Some(sequence) = sequence_manager.current_sequence() {
                                loader_config.coord_transform = sequence.source_orientation.to_transform();
                                debug!(
                                    "Applying coordinate transformation: {}",
                                    sequence.source_orientation.description()
                                );
                            }
                        }

                        if frame_store.loaded_count % 10 == 0 || frame_store.loaded_count == requested_count {
                            debug!(
                                "Loading progress: {}/{} frames loaded",
                                frame_store.loaded_count, requested_count,
                            );
                        }
                    }
                } else {
                    // Non-active series: emit FrameLoadedEvent and update MeshCache.
                    frame_loaded_events.write(FrameLoadedEvent {
                        frame_index: index,
                        success: true,
                    });
                    if let Some(cache_entry) = mesh_cache.get_by_asset_id_mut(id) {
                        cache_entry.loaded = true;
                    }
                }
            }
            _ => {}
        }
    }

    // Check if all active-series requested frames are loaded.
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
