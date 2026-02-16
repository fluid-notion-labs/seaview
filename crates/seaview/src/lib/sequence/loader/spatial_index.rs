//! Spatial index for loaded mesh frames.
//!
//! This module maintains a [`SpatialIndex`] resource that holds a
//! [`SpatialMesh`](crate::lib::spatial::SpatialMesh) for each loaded frame,
//! enabling spatial queries (ray casting, point projection, AABB) against the
//! mesh geometry.
//!
//! The [`build_spatial_meshes`] system watches for [`FrameLoadedEvent`]s and
//! builds the spatial acceleration structures from the now-ready Bevy `Mesh`
//! assets.  BVH construction is currently synchronous; for very large meshes
//! (>2M triangles, ~20-40 ms) this could be moved to an async task pool in
//! the future.

use bevy::mesh::Mesh;
use bevy::prelude::*;
use std::time::Instant;

use super::assets::FrameStore;
use super::FrameLoadedEvent;
use crate::lib::spatial::{SpatialMesh, SpatialMeshError};

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

/// Per-frame spatial acceleration structures, indexed in parallel with
/// [`FrameStore::frame_handles`].
///
/// Each frame may contain multiple meshes (e.g. multi-object GLB files), so
/// the inner type is `Vec<SpatialMesh>`.
#[derive(Resource, Default)]
pub struct SpatialIndex {
    /// Spatial meshes per frame.  `None` means the spatial mesh has not been
    /// built yet (either the frame isn't loaded, or build failed).
    meshes: Vec<Option<Vec<SpatialMesh>>>,

    /// Number of frames that have a spatial mesh built.
    built_count: usize,

    /// Cumulative build time across all frames (for diagnostics).
    total_build_time_ms: f64,
}

impl SpatialIndex {
    /// Reset for a new sequence with `n` frames.
    pub fn reset(&mut self, num_frames: usize) {
        self.meshes.clear();
        self.meshes.reserve(num_frames);
        for _ in 0..num_frames {
            self.meshes.push(None);
        }
        self.built_count = 0;
        self.total_build_time_ms = 0.0;
    }

    /// Get the spatial meshes for a frame, if built.
    pub fn get(&self, frame_index: usize) -> Option<&[SpatialMesh]> {
        self.meshes
            .get(frame_index)
            .and_then(|opt| opt.as_ref())
            .map(|v| v.as_slice())
    }

    /// Get the first spatial mesh for a frame (convenience for single-mesh frames).
    pub fn get_first(&self, frame_index: usize) -> Option<&SpatialMesh> {
        self.get(frame_index).and_then(|meshes| meshes.first())
    }

    /// Check whether a spatial mesh has been built for this frame.
    pub fn is_built(&self, frame_index: usize) -> bool {
        self.meshes
            .get(frame_index)
            .is_some_and(|opt| opt.is_some())
    }

    /// Number of frames with spatial meshes built.
    pub fn built_count(&self) -> usize {
        self.built_count
    }

    /// Total number of frame slots.
    pub fn total_slots(&self) -> usize {
        self.meshes.len()
    }

    /// Cumulative BVH build time in milliseconds (for diagnostics).
    pub fn total_build_time_ms(&self) -> f64 {
        self.total_build_time_ms
    }

    /// Store spatial meshes for a frame.  Returns `true` if this was a new
    /// entry (frame didn't already have spatial data).
    fn insert(&mut self, frame_index: usize, spatial_meshes: Vec<SpatialMesh>) -> bool {
        if frame_index >= self.meshes.len() {
            return false;
        }
        let is_new = self.meshes[frame_index].is_none();
        self.meshes[frame_index] = Some(spatial_meshes);
        if is_new {
            self.built_count += 1;
        }
        is_new
    }

    /// Record build time for diagnostics.
    fn record_build_time(&mut self, ms: f64) {
        self.total_build_time_ms += ms;
    }
}

// ---------------------------------------------------------------------------
// Pure function
// ---------------------------------------------------------------------------

/// Build `SpatialMesh` instances from all Bevy `Mesh` handles for a single
/// frame.
///
/// Returns `Ok(meshes)` with one `SpatialMesh` per handle, or `Err` with the
/// list of per-mesh errors (partial successes are included in the `Ok` path
/// only if *all* meshes succeed).
pub fn build_spatial_meshes_for_frame(
    mesh_handles: &[Handle<Mesh>],
    mesh_assets: &Assets<Mesh>,
) -> Result<Vec<SpatialMesh>, Vec<(usize, SpatialMeshError)>> {
    let mut spatial_meshes = Vec::with_capacity(mesh_handles.len());
    let mut errors = Vec::new();

    for (i, handle) in mesh_handles.iter().enumerate() {
        let Some(mesh) = mesh_assets.get(handle) else {
            errors.push((
                i,
                SpatialMeshError::MissingPositions, // mesh asset not found
            ));
            continue;
        };

        match SpatialMesh::from_bevy_mesh(mesh) {
            Ok(sm) => spatial_meshes.push(sm),
            Err(e) => errors.push((i, e)),
        }
    }

    if errors.is_empty() {
        Ok(spatial_meshes)
    } else {
        Err(errors)
    }
}

// ---------------------------------------------------------------------------
// System
// ---------------------------------------------------------------------------

/// System that builds [`SpatialMesh`] acceleration structures when frames
/// finish loading.
///
/// Listens for [`FrameLoadedEvent`]s emitted by the loading tracker and GLB
/// extractor, reads the mesh data from `Assets<Mesh>`, and populates the
/// [`SpatialIndex`] resource.
///
/// BVH construction is synchronous on the main thread.  Build times are
/// logged so we can decide whether to move to async later.
pub(super) fn build_spatial_meshes(
    mut events: MessageReader<FrameLoadedEvent>,
    frame_store: Res<FrameStore>,
    mesh_assets: Res<Assets<Mesh>>,
    mut spatial_index: ResMut<SpatialIndex>,
) {
    for event in events.read() {
        if !event.success {
            continue;
        }

        let idx = event.frame_index;

        // Skip if already built (e.g. duplicate event).
        if spatial_index.is_built(idx) {
            continue;
        }

        let Some(mesh_handles) = frame_store.get_frame_meshes(idx) else {
            debug!("SpatialIndex: frame {} loaded but no mesh handles yet", idx);
            continue;
        };

        let start = Instant::now();
        match build_spatial_meshes_for_frame(mesh_handles, &mesh_assets) {
            Ok(spatial_meshes) => {
                let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
                let total_tris: usize = spatial_meshes.iter().map(|sm| sm.triangle_count()).sum();
                let total_verts: usize = spatial_meshes.iter().map(|sm| sm.vertex_count()).sum();
                let mesh_count = spatial_meshes.len();

                spatial_index.record_build_time(elapsed_ms);
                spatial_index.insert(idx, spatial_meshes);

                if elapsed_ms > 5.0 || idx % 50 == 0 || spatial_index.built_count() <= 3 {
                    info!(
                        "SpatialIndex: frame {} built ({} meshes, {} verts, {} tris) in {:.1} ms \
                         [{}/{}]",
                        idx,
                        mesh_count,
                        total_verts,
                        total_tris,
                        elapsed_ms,
                        spatial_index.built_count(),
                        spatial_index.total_slots(),
                    );
                }
            }
            Err(errors) => {
                let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
                spatial_index.record_build_time(elapsed_ms);

                for (mesh_i, err) in &errors {
                    warn!(
                        "SpatialIndex: frame {} mesh {} failed: {}",
                        idx, mesh_i, err,
                    );
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lib::spatial::SpatialMesh;

    // ── SpatialIndex resource ─────────────────────────────────────────

    #[test]
    fn test_reset() {
        let mut index = SpatialIndex::default();
        index.reset(10);
        assert_eq!(index.total_slots(), 10);
        assert_eq!(index.built_count(), 0);
        assert!(!index.is_built(0));
    }

    #[test]
    fn test_insert_and_get() {
        let mut index = SpatialIndex::default();
        index.reset(5);

        let sm = SpatialMesh::from_raw(
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            vec![[0, 1, 2]],
        )
        .unwrap();

        assert!(index.insert(2, vec![sm]));
        assert!(index.is_built(2));
        assert!(!index.is_built(0));
        assert_eq!(index.built_count(), 1);

        let meshes = index.get(2).unwrap();
        assert_eq!(meshes.len(), 1);
        assert_eq!(meshes[0].triangle_count(), 1);
    }

    #[test]
    fn test_get_first() {
        let mut index = SpatialIndex::default();
        index.reset(3);

        let sm = SpatialMesh::from_raw(
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            vec![[0, 1, 2]],
        )
        .unwrap();

        index.insert(1, vec![sm]);

        assert!(index.get_first(1).is_some());
        assert!(index.get_first(0).is_none());
        assert!(index.get_first(99).is_none()); // out of bounds
    }

    #[test]
    fn test_insert_out_of_bounds() {
        let mut index = SpatialIndex::default();
        index.reset(3);

        let sm = SpatialMesh::from_raw(
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            vec![[0, 1, 2]],
        )
        .unwrap();

        assert!(!index.insert(99, vec![sm])); // out of bounds
        assert_eq!(index.built_count(), 0);
    }

    #[test]
    fn test_duplicate_insert_does_not_double_count() {
        let mut index = SpatialIndex::default();
        index.reset(3);

        let make_sm = || {
            SpatialMesh::from_raw(
                vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                vec![[0, 1, 2]],
            )
            .unwrap()
        };

        assert!(index.insert(0, vec![make_sm()])); // first insert → true
        assert!(!index.insert(0, vec![make_sm()])); // duplicate → false
        assert_eq!(index.built_count(), 1);
    }

    #[test]
    fn test_record_build_time() {
        let mut index = SpatialIndex::default();
        index.record_build_time(1.5);
        index.record_build_time(2.5);
        assert!((index.total_build_time_ms() - 4.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_multi_mesh_frame() {
        let mut index = SpatialIndex::default();
        index.reset(2);

        let sm1 = SpatialMesh::from_raw(
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            vec![[0, 1, 2]],
        )
        .unwrap();
        let sm2 = SpatialMesh::from_raw(
            vec![[2.0, 0.0, 0.0], [3.0, 0.0, 0.0], [2.0, 1.0, 0.0]],
            vec![[0, 1, 2]],
        )
        .unwrap();

        index.insert(0, vec![sm1, sm2]);
        let meshes = index.get(0).unwrap();
        assert_eq!(meshes.len(), 2);
        assert_eq!(index.get_first(0).unwrap().triangle_count(), 1);
    }

    // ── build_spatial_meshes_for_frame (pure function) ────────────────

    #[test]
    fn test_build_from_bevy_mesh_assets() {
        use bevy::mesh::{Indices, Mesh, PrimitiveTopology};

        let mut mesh_assets = Assets::<Mesh>::default();

        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, Default::default());
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        );
        mesh.insert_indices(Indices::U32(vec![0, 1, 2]));

        let handle = mesh_assets.add(mesh);

        let result = build_spatial_meshes_for_frame(&[handle], &mesh_assets);
        assert!(result.is_ok());
        let spatial = result.unwrap();
        assert_eq!(spatial.len(), 1);
        assert_eq!(spatial[0].triangle_count(), 1);
        assert_eq!(spatial[0].vertex_count(), 3);
    }

    #[test]
    fn test_build_missing_mesh_asset() {
        let mesh_assets = Assets::<Mesh>::default();
        let handle = Handle::<Mesh>::default();

        let result = build_spatial_meshes_for_frame(&[handle], &mesh_assets);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_empty_handles() {
        let mesh_assets = Assets::<Mesh>::default();

        let result = build_spatial_meshes_for_frame(&[], &mesh_assets);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }
}
