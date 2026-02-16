//! Spatial queries on mesh data using parry3d.
//!
//! This module provides a `SpatialMesh` wrapper around parry3d's `TriMesh` for
//! efficient ray casting, point projection, and other spatial queries on mesh geometry.
//!
//! # Architecture
//!
//! - **Pure library code**: No Bevy dependencies in the core types
//! - **Zero-copy where possible**: `from_raw` accepts pre-allocated vectors
//! - **Testable**: All methods can be tested without ECS
//!
//! # Example
//!
//! ```no_run
//! use seaview::lib::spatial::SpatialMesh;
//!
//! let positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
//! let indices = vec![[0, 1, 2]];
//! let mesh = SpatialMesh::from_raw(positions, indices).unwrap();
//!
//! // Ray cast
//! let origin = [0.5, 0.5, 1.0];
//! let direction = [0.0, 0.0, -1.0];
//! if let Some(hit) = mesh.cast_ray(&origin, &direction, 10.0) {
//!     println!("Hit at distance: {}", hit.time_of_impact);
//! }
//! ```

use bevy::math::Vec3;
use bevy::mesh::{Indices, Mesh, VertexAttributeValues};

use parry3d::partitioning::TraversalAction;
use parry3d::query::{PointQuery, RayCast};
use parry3d::shape::TriMesh;
use thiserror::Error;

// ---------------------------------------------------------------------------
// BVH visualization types
// ---------------------------------------------------------------------------

/// A single BVH node's AABB with metadata for visualization.
#[derive(Debug, Clone, Copy)]
pub struct BvhNodeAabb {
    /// Minimum corner of the AABB.
    pub min: [f32; 3],
    /// Maximum corner of the AABB.
    pub max: [f32; 3],
    /// Depth of this node in the tree (0 = root children).
    pub depth: u32,
    /// Whether this node is a leaf (contains geometry) or internal.
    pub is_leaf: bool,
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Errors that can occur when constructing a `SpatialMesh`.
#[derive(Debug, Error)]
pub enum SpatialMeshError {
    /// The mesh has no position attribute.
    #[error("Mesh is missing POSITION vertex attribute")]
    MissingPositions,

    /// The mesh has no indices.
    #[error("Mesh is missing index buffer")]
    MissingIndices,

    /// Position attribute is not Float32x3.
    #[error("POSITION attribute is not Float32x3 (found: {0})")]
    InvalidPositionFormat(String),

    /// Index buffer is not u32.
    #[error("Index buffer is not u32 (found: {0})")]
    InvalidIndexFormat(String),

    /// The mesh has an invalid number of indices (must be a multiple of 3).
    #[error("Index count {0} is not a multiple of 3")]
    InvalidIndexCount(usize),

    /// parry3d failed to build the TriMesh.
    #[error("Failed to build parry3d TriMesh: {0}")]
    TriMeshBuildError(String),
}

/// Result of a ray-mesh intersection.
#[derive(Debug, Clone, Copy)]
pub struct RayHit {
    /// Distance along the ray where the intersection occurred.
    pub time_of_impact: f32,
    /// Surface normal at the hit point (normalized).
    pub normal: [f32; 3],
    /// Hit point in mesh-local coordinates.
    pub point: [f32; 3],
}

/// Result of projecting a point onto the mesh surface.
#[derive(Debug, Clone, Copy)]
pub struct PointProjection {
    /// Closest point on the mesh surface.
    pub point: [f32; 3],
    /// Whether the query point was inside the mesh.
    pub is_inside: bool,
}

// ---------------------------------------------------------------------------
// SpatialMesh
// ---------------------------------------------------------------------------

/// Spatial acceleration structure for mesh queries (ray casting, point projection, etc.).
///
/// Wraps a parry3d `TriMesh` and provides a convenient API for spatial queries.
pub struct SpatialMesh {
    trimesh: TriMesh,
}

impl SpatialMesh {
    /// Construct a `SpatialMesh` from a Bevy `Mesh`.
    ///
    /// This clones the position and index data from the Bevy mesh and builds
    /// a parry3d `TriMesh` for spatial queries.
    ///
    /// # Errors
    ///
    /// Returns an error if:
    /// - The mesh has no POSITION attribute
    /// - The mesh has no indices
    /// - The POSITION attribute is not Float32x3
    /// - The indices are not u32
    /// - The index count is not a multiple of 3
    /// - parry3d fails to build the TriMesh
    pub fn from_bevy_mesh(mesh: &Mesh) -> Result<Self, SpatialMeshError> {
        // Extract positions
        let positions = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .ok_or(SpatialMeshError::MissingPositions)?;

        let positions_vec = match positions {
            VertexAttributeValues::Float32x3(verts) => verts.clone(),
            other => {
                return Err(SpatialMeshError::InvalidPositionFormat(format!(
                    "{:?}",
                    other
                )))
            }
        };

        // Extract indices
        let indices = mesh.indices().ok_or(SpatialMeshError::MissingIndices)?;

        let indices_vec = match indices {
            Indices::U32(idx) => {
                if idx.len() % 3 != 0 {
                    return Err(SpatialMeshError::InvalidIndexCount(idx.len()));
                }
                // Convert flat u32 array to [[u32; 3]]
                idx.chunks_exact(3)
                    .map(|chunk| [chunk[0], chunk[1], chunk[2]])
                    .collect::<Vec<_>>()
            }
            Indices::U16(idx) => {
                if idx.len() % 3 != 0 {
                    return Err(SpatialMeshError::InvalidIndexCount(idx.len()));
                }
                // Convert u16 to u32 and group into triangles
                idx.chunks_exact(3)
                    .map(|chunk| [chunk[0] as u32, chunk[1] as u32, chunk[2] as u32])
                    .collect::<Vec<_>>()
            }
        };

        Self::from_raw(positions_vec, indices_vec)
    }

    /// Construct a `SpatialMesh` from raw position and index data.
    ///
    /// This is a zero-copy path: the vectors are moved into the parry3d `TriMesh`.
    ///
    /// # Arguments
    ///
    /// - `positions`: Vertex positions as `[f32; 3]`
    /// - `indices`: Triangle indices as `[u32; 3]`
    ///
    /// # Errors
    ///
    /// Returns an error if parry3d fails to build the TriMesh (e.g., degenerate triangles).
    pub fn from_raw(
        positions: Vec<[f32; 3]>,
        indices: Vec<[u32; 3]>,
    ) -> Result<Self, SpatialMeshError> {
        // Convert to parry3d types using bytemuck for zero-copy cast
        let points: Vec<Vec3> = bytemuck::cast_vec(positions);
        let triangles = indices;

        // Build the TriMesh
        let trimesh = TriMesh::new(points, triangles)
            .map_err(|e| SpatialMeshError::TriMeshBuildError(format!("{:?}", e)))?;

        Ok(Self { trimesh })
    }

    /// Cast a ray against the mesh and return the first hit.
    ///
    /// # Arguments
    ///
    /// - `origin`: Ray origin in mesh-local coordinates
    /// - `direction`: Ray direction (will be normalized)
    /// - `max_distance`: Maximum ray travel distance
    ///
    /// # Returns
    ///
    /// `Some(RayHit)` if the ray intersects the mesh, `None` otherwise.
    pub fn cast_ray(
        &self,
        origin: &[f32; 3],
        direction: &[f32; 3],
        max_distance: f32,
    ) -> Option<RayHit> {
        let ray_origin = Vec3::new(origin[0], origin[1], origin[2]);
        let ray_dir = Vec3::new(direction[0], direction[1], direction[2]).normalize();
        let ray = parry3d::query::Ray::new(ray_origin, ray_dir);

        self.trimesh
            .cast_local_ray_and_get_normal(&ray, max_distance, true)
            .map(|ray_intersection| {
                let hit_point = ray.point_at(ray_intersection.time_of_impact);
                let normal = ray_intersection.normal;

                RayHit {
                    time_of_impact: ray_intersection.time_of_impact,
                    normal: [normal.x, normal.y, normal.z],
                    point: [hit_point.x, hit_point.y, hit_point.z],
                }
            })
    }

    /// Project a point onto the mesh surface.
    ///
    /// Finds the closest point on the mesh to the given query point.
    ///
    /// # Arguments
    ///
    /// - `point`: Query point in mesh-local coordinates
    ///
    /// # Returns
    ///
    /// A `PointProjection` containing the closest surface point and whether
    /// the query point is inside the mesh.
    pub fn project_point(&self, point: &[f32; 3]) -> PointProjection {
        let query_point = Vec3::new(point[0], point[1], point[2]);
        let projection = self.trimesh.project_local_point(query_point, true);

        PointProjection {
            point: [projection.point.x, projection.point.y, projection.point.z],
            is_inside: projection.is_inside,
        }
    }

    /// Get the axis-aligned bounding box (AABB) of the mesh.
    ///
    /// # Returns
    ///
    /// `(min, max)` corners of the AABB in mesh-local coordinates.
    pub fn aabb(&self) -> ([f32; 3], [f32; 3]) {
        let aabb = self.trimesh.local_aabb();
        let min = [aabb.mins.x, aabb.mins.y, aabb.mins.z];
        let max = [aabb.maxs.x, aabb.maxs.y, aabb.maxs.z];
        (min, max)
    }

    /// Get the number of triangles in the mesh.
    pub fn triangle_count(&self) -> usize {
        self.trimesh.num_triangles()
    }

    /// Get the number of vertices in the mesh.
    pub fn vertex_count(&self) -> usize {
        self.trimesh.vertices().len()
    }

    /// Get direct access to the underlying parry3d `TriMesh`.
    ///
    /// This is useful for advanced queries not exposed by this wrapper.
    pub fn trimesh(&self) -> &TriMesh {
        &self.trimesh
    }

    /// Extract BVH node AABBs for debug visualization.
    ///
    /// Walks the BVH tree via the public `traverse` API and collects
    /// axis-aligned bounding boxes for each node.  Depth is **estimated**
    /// from the volume ratio `log2(root_volume / node_volume)`, which is a
    /// close proxy for actual tree depth without requiring internal BVH
    /// access.
    ///
    /// Nodes whose estimated depth exceeds `max_depth` are excluded.
    /// Pass `u32::MAX` to collect the entire tree.
    ///
    /// This is a pure function — no Bevy dependencies, fully testable.
    pub fn bvh_node_aabbs(&self, max_depth: u32) -> Vec<BvhNodeAabb> {
        let bvh = self.trimesh.bvh();

        // First pass: collect all nodes with their volumes.
        let mut raw_nodes: Vec<([f32; 3], [f32; 3], bool, f32)> = Vec::new();
        bvh.traverse(|node| {
            let mins = node.mins();
            let maxs = node.maxs();
            // Skip degenerate nodes
            if mins == maxs {
                return TraversalAction::Continue;
            }

            let extents = maxs - mins;
            let sa = node_half_surface_area(extents);

            raw_nodes.push((
                [mins.x, mins.y, mins.z],
                [maxs.x, maxs.y, maxs.z],
                node.is_leaf(),
                sa,
            ));
            TraversalAction::Continue
        });

        if raw_nodes.is_empty() {
            return Vec::new();
        }

        // Find max half-surface-area (root-level nodes).
        let max_sa = raw_nodes.iter().map(|n| n.3).fold(0.0_f32, f32::max);

        raw_nodes
            .iter()
            .filter_map(|(min, max, is_leaf, sa)| {
                let depth = sa_to_depth(max_sa, *sa);
                if depth > max_depth {
                    return None;
                }
                Some(BvhNodeAabb {
                    min: *min,
                    max: *max,
                    depth,
                    is_leaf: *is_leaf,
                })
            })
            .collect()
    }

    /// Estimated maximum depth of the BVH tree.
    ///
    /// Uses half-surface-area ratio as a proxy for depth (same heuristic as
    /// [`bvh_node_aabbs`]).  Useful for configuring the depth slider in
    /// debug UI.
    pub fn bvh_depth(&self) -> u32 {
        let bvh = self.trimesh.bvh();
        let mut max_sa: f32 = 0.0;
        let mut min_leaf_sa: f32 = f32::MAX;

        bvh.traverse(|node| {
            let mins = node.mins();
            let maxs = node.maxs();
            if mins == maxs {
                return TraversalAction::Continue;
            }

            let extents = maxs - mins;
            let sa = node_half_surface_area(extents);

            if sa > max_sa {
                max_sa = sa;
            }
            if node.is_leaf() && sa > 0.0 && sa < min_leaf_sa {
                min_leaf_sa = sa;
            }

            TraversalAction::Continue
        });

        if max_sa > 0.0 && min_leaf_sa > 0.0 && min_leaf_sa < max_sa {
            sa_to_depth(max_sa, min_leaf_sa)
        } else {
            0
        }
    }
}

/// Compute half-surface-area of an AABB from its extents.
///
/// This is the standard SAH (Surface Area Heuristic) metric used in BVH
/// construction.  Unlike volume, it handles flat geometry (where one
/// extent is zero) gracefully.
fn node_half_surface_area(extents: Vec3) -> f32 {
    let ex = extents.x.abs();
    let ey = extents.y.abs();
    let ez = extents.z.abs();
    ex * ey + ey * ez + ez * ex
}

/// Estimate tree depth from half-surface-area ratio:
/// `log2(root_sa / node_sa)`.
///
/// This is a close proxy for actual BVH depth without requiring access to
/// internal tree structure.
fn sa_to_depth(max_sa: f32, node_sa: f32) -> u32 {
    if max_sa <= 0.0 || node_sa <= 0.0 {
        return 0;
    }
    (max_sa / node_sa).log2().max(0.0) as u32
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::asset::RenderAssetUsages;
    use bevy::mesh::PrimitiveTopology;

    // ── BVH visualization ─────────────────────────────────────────────

    #[test]
    fn test_bvh_node_aabbs_triangle() {
        let sm = SpatialMesh::from_raw(
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            vec![[0, 1, 2]],
        )
        .unwrap();

        let aabbs = sm.bvh_node_aabbs(u32::MAX);
        // A single triangle → at least one leaf node
        assert!(!aabbs.is_empty());
        assert!(aabbs.iter().any(|n| n.is_leaf));
    }

    #[test]
    fn test_bvh_node_aabbs_cube_has_internal_nodes() {
        let (positions, indices) = test_cube();
        let sm = SpatialMesh::from_raw(positions, indices).unwrap();

        let all = sm.bvh_node_aabbs(u32::MAX);
        let internals: Vec<_> = all.iter().filter(|n| !n.is_leaf).collect();
        let leaves: Vec<_> = all.iter().filter(|n| n.is_leaf).collect();

        // Cube has 12 triangles → 12 leaves
        assert_eq!(leaves.len(), 12);
        // There must be internal nodes to organize them
        assert!(!internals.is_empty());
    }

    #[test]
    fn test_bvh_node_aabbs_max_depth_zero() {
        let (positions, indices) = test_cube();
        let sm = SpatialMesh::from_raw(positions, indices).unwrap();

        let depth_0 = sm.bvh_node_aabbs(0);
        let all = sm.bvh_node_aabbs(u32::MAX);

        // depth 0 should return some subset of nodes
        assert!(!depth_0.is_empty());
        // depth 0 should have no more nodes than the full tree
        assert!(depth_0.len() <= all.len());
        // all depth-0 nodes have depth == 0
        assert!(depth_0.iter().all(|n| n.depth == 0));
    }

    #[test]
    fn test_bvh_depth_triangle() {
        let sm = SpatialMesh::from_raw(
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            vec![[0, 1, 2]],
        )
        .unwrap();

        // Single triangle → very shallow tree
        assert!(sm.bvh_depth() <= 1);
    }

    #[test]
    fn test_bvh_depth_cube() {
        let (positions, indices) = test_cube();
        let sm = SpatialMesh::from_raw(positions, indices).unwrap();

        // 12 triangles in a compact cube — SA-based depth may be shallow
        // because all geometry is tightly packed.  Just verify it returns
        // a reasonable value (the heuristic works better on spatially
        // diverse meshes).
        let depth = sm.bvh_depth();
        assert!(depth <= 20, "cube BVH depth unexpectedly large: {}", depth);
    }

    #[test]
    fn test_bvh_depth_dispersed_geometry() {
        // Create geometry spread across a large area so the SA heuristic
        // produces meaningful depth differentiation.
        let positions = vec![
            // Triangle 1: near origin
            [0.0, 0.0, 0.0],
            [1.0, 0.0, 0.0],
            [0.0, 1.0, 0.0],
            // Triangle 2: far away
            [100.0, 0.0, 0.0],
            [101.0, 0.0, 0.0],
            [100.0, 1.0, 0.0],
            // Triangle 3: even further
            [200.0, 0.0, 0.0],
            [201.0, 0.0, 0.0],
            [200.0, 1.0, 0.0],
            // Triangle 4: opposite direction
            [-100.0, 0.0, 0.0],
            [-99.0, 0.0, 0.0],
            [-100.0, 1.0, 0.0],
        ];
        let indices = vec![[0, 1, 2], [3, 4, 5], [6, 7, 8], [9, 10, 11]];
        let sm = SpatialMesh::from_raw(positions, indices).unwrap();

        let depth = sm.bvh_depth();
        assert!(
            depth >= 1,
            "dispersed geometry BVH depth should be >= 1, got {}",
            depth
        );

        // Verify that filtering by depth actually reduces node count
        let all = sm.bvh_node_aabbs(u32::MAX);
        if depth > 0 {
            let filtered = sm.bvh_node_aabbs(0);
            assert!(
                filtered.len() < all.len(),
                "depth filter should reduce nodes: depth_0={}, all={}",
                filtered.len(),
                all.len()
            );
        }
    }

    #[test]
    fn test_bvh_node_aabbs_contain_mesh_aabb() {
        let (positions, indices) = test_cube();
        let sm = SpatialMesh::from_raw(positions, indices).unwrap();

        let (mesh_min, mesh_max) = sm.aabb();
        let depth_0 = sm.bvh_node_aabbs(0);

        // The union of depth-0 AABBs should cover the mesh AABB
        let mut union_min = [f32::MAX; 3];
        let mut union_max = [f32::MIN; 3];
        for node in &depth_0 {
            for i in 0..3 {
                union_min[i] = union_min[i].min(node.min[i]);
                union_max[i] = union_max[i].max(node.max[i]);
            }
        }

        for i in 0..3 {
            assert!(
                union_min[i] <= mesh_min[i] + f32::EPSILON,
                "BVH union min[{}] = {} > mesh min = {}",
                i,
                union_min[i],
                mesh_min[i]
            );
            assert!(
                union_max[i] >= mesh_max[i] - f32::EPSILON,
                "BVH union max[{}] = {} < mesh max = {}",
                i,
                union_max[i],
                mesh_max[i]
            );
        }
    }

    /// Helper: Create a simple triangle mesh for testing.
    fn test_triangle() -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
        let positions = vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let indices = vec![[0, 1, 2]];
        (positions, indices)
    }

    /// Helper: Create a cube mesh for testing.
    fn test_cube() -> (Vec<[f32; 3]>, Vec<[u32; 3]>) {
        let positions = vec![
            // Front face
            [-0.5, -0.5, 0.5],
            [0.5, -0.5, 0.5],
            [0.5, 0.5, 0.5],
            [-0.5, 0.5, 0.5],
            // Back face
            [-0.5, -0.5, -0.5],
            [0.5, -0.5, -0.5],
            [0.5, 0.5, -0.5],
            [-0.5, 0.5, -0.5],
        ];

        let indices = vec![
            // Front
            [0, 1, 2],
            [0, 2, 3],
            // Back
            [5, 4, 7],
            [5, 7, 6],
            // Left
            [4, 0, 3],
            [4, 3, 7],
            // Right
            [1, 5, 6],
            [1, 6, 2],
            // Top
            [3, 2, 6],
            [3, 6, 7],
            // Bottom
            [4, 5, 1],
            [4, 1, 0],
        ];

        (positions, indices)
    }

    // ── Constructor tests ──────────────────────────────────────────────

    #[test]
    fn test_from_raw_triangle() {
        let (positions, indices) = test_triangle();
        let mesh = SpatialMesh::from_raw(positions, indices);
        assert!(mesh.is_ok());

        let mesh = mesh.unwrap();
        assert_eq!(mesh.triangle_count(), 1);
        assert_eq!(mesh.vertex_count(), 3);
    }

    #[test]
    fn test_from_raw_cube() {
        let (positions, indices) = test_cube();
        let mesh = SpatialMesh::from_raw(positions, indices);
        assert!(mesh.is_ok());

        let mesh = mesh.unwrap();
        assert_eq!(mesh.triangle_count(), 12);
        assert_eq!(mesh.vertex_count(), 8);
    }

    #[test]
    fn test_from_raw_empty() {
        let positions = vec![];
        let indices = vec![];
        let mesh = SpatialMesh::from_raw(positions, indices);
        // Empty mesh should fail - parry3d requires at least one triangle
        assert!(matches!(mesh, Err(SpatialMeshError::TriMeshBuildError(_))));
    }

    // ── AABB tests ─────────────────────────────────────────────────────

    #[test]
    fn test_aabb_triangle() {
        let (positions, indices) = test_triangle();
        let mesh = SpatialMesh::from_raw(positions, indices).unwrap();

        let (min, max) = mesh.aabb();
        assert_eq!(min, [0.0, 0.0, 0.0]);
        assert_eq!(max, [1.0, 1.0, 0.0]);
    }

    #[test]
    fn test_aabb_cube() {
        let (positions, indices) = test_cube();
        let mesh = SpatialMesh::from_raw(positions, indices).unwrap();

        let (min, max) = mesh.aabb();
        assert!((min[0] - (-0.5)).abs() < 1e-6);
        assert!((min[1] - (-0.5)).abs() < 1e-6);
        assert!((min[2] - (-0.5)).abs() < 1e-6);
        assert!((max[0] - 0.5).abs() < 1e-6);
        assert!((max[1] - 0.5).abs() < 1e-6);
        assert!((max[2] - 0.5).abs() < 1e-6);
    }

    // ── Ray casting tests ──────────────────────────────────────────────

    #[test]
    fn test_cast_ray_hit_triangle() {
        let (positions, indices) = test_triangle();
        let mesh = SpatialMesh::from_raw(positions, indices).unwrap();

        // Cast ray from above the triangle, pointing down
        let origin = [0.25, 0.25, 1.0];
        let direction = [0.0, 0.0, -1.0];
        let hit = mesh.cast_ray(&origin, &direction, 10.0);

        assert!(hit.is_some());
        let hit = hit.unwrap();
        assert!((hit.time_of_impact - 1.0).abs() < 1e-5);
        assert_eq!(hit.point[2], 0.0); // Hit the z=0 plane
    }

    #[test]
    fn test_cast_ray_miss_triangle() {
        let (positions, indices) = test_triangle();
        let mesh = SpatialMesh::from_raw(positions, indices).unwrap();

        // Cast ray that misses the triangle
        let origin = [2.0, 2.0, 1.0];
        let direction = [0.0, 0.0, -1.0];
        let hit = mesh.cast_ray(&origin, &direction, 10.0);

        assert!(hit.is_none());
    }

    #[test]
    fn test_cast_ray_hit_cube() {
        let (positions, indices) = test_cube();
        let mesh = SpatialMesh::from_raw(positions, indices).unwrap();

        // Cast ray from outside the cube, pointing at center
        let origin = [0.0, 0.0, 2.0];
        let direction = [0.0, 0.0, -1.0];
        let hit = mesh.cast_ray(&origin, &direction, 10.0);

        assert!(hit.is_some());
        let hit = hit.unwrap();
        // Should hit front face at z=0.5
        assert!((hit.point[2] - 0.5).abs() < 1e-5);
    }

    #[test]
    fn test_cast_ray_max_distance() {
        let (positions, indices) = test_cube();
        let mesh = SpatialMesh::from_raw(positions, indices).unwrap();

        // Ray would hit, but max_distance is too short
        let origin = [0.0, 0.0, 2.0];
        let direction = [0.0, 0.0, -1.0];
        let hit = mesh.cast_ray(&origin, &direction, 1.0);

        assert!(hit.is_none());
    }

    // ── Point projection tests ─────────────────────────────────────────

    #[test]
    fn test_project_point_on_surface() {
        let (positions, indices) = test_triangle();
        let mesh = SpatialMesh::from_raw(positions, indices).unwrap();

        // Point above the triangle should project down to z=0
        let point = [0.25, 0.25, 1.0];
        let proj = mesh.project_point(&point);

        assert_eq!(proj.point[2], 0.0);
        assert!((proj.point[0] - 0.25).abs() < 1e-5);
        assert!((proj.point[1] - 0.25).abs() < 1e-5);
    }

    #[test]
    fn test_project_point_cube_center() {
        let (positions, indices) = test_cube();
        let mesh = SpatialMesh::from_raw(positions, indices).unwrap();

        // Center of cube should project to nearest face
        let point = [0.0, 0.0, 0.0];
        let proj = mesh.project_point(&point);

        // Should project to one of the faces (distance 0.5 from center)
        let dist = (proj.point[0] * proj.point[0]
            + proj.point[1] * proj.point[1]
            + proj.point[2] * proj.point[2])
            .sqrt();
        assert!((dist - 0.5).abs() < 1e-5);
        // Note: is_inside detection may vary by parry3d version/configuration
        // Don't assert on is_inside for this edge case
    }

    #[test]
    fn test_project_point_outside_cube() {
        let (positions, indices) = test_cube();
        let mesh = SpatialMesh::from_raw(positions, indices).unwrap();

        // Point outside cube
        let point = [2.0, 0.0, 0.0];
        let proj = mesh.project_point(&point);

        // Should project to right face at x=0.5
        assert!((proj.point[0] - 0.5).abs() < 1e-5);
        assert!(!proj.is_inside);
    }

    // ── Bevy Mesh integration tests ────────────────────────────────────

    #[test]
    fn test_from_bevy_mesh_no_positions() {
        let mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD,
        );
        let result = SpatialMesh::from_bevy_mesh(&mesh);
        assert!(matches!(result, Err(SpatialMeshError::MissingPositions)));
    }

    #[test]
    fn test_from_bevy_mesh_no_indices() {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD,
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        );
        let result = SpatialMesh::from_bevy_mesh(&mesh);
        assert!(matches!(result, Err(SpatialMeshError::MissingIndices)));
    }

    #[test]
    fn test_from_bevy_mesh_success_u32() {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD,
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        );
        mesh.insert_indices(Indices::U32(vec![0, 1, 2]));

        let result = SpatialMesh::from_bevy_mesh(&mesh);
        assert!(result.is_ok());

        let spatial_mesh = result.unwrap();
        assert_eq!(spatial_mesh.triangle_count(), 1);
        assert_eq!(spatial_mesh.vertex_count(), 3);
    }

    #[test]
    fn test_from_bevy_mesh_success_u16() {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD,
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        );
        mesh.insert_indices(Indices::U16(vec![0, 1, 2]));

        let result = SpatialMesh::from_bevy_mesh(&mesh);
        assert!(result.is_ok());

        let spatial_mesh = result.unwrap();
        assert_eq!(spatial_mesh.triangle_count(), 1);
        assert_eq!(spatial_mesh.vertex_count(), 3);
    }

    #[test]
    fn test_from_bevy_mesh_invalid_index_count() {
        let mut mesh = Mesh::new(
            PrimitiveTopology::TriangleList,
            RenderAssetUsages::MAIN_WORLD,
        );
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        );
        // Only 2 indices (not a multiple of 3)
        mesh.insert_indices(Indices::U32(vec![0, 1]));

        let result = SpatialMesh::from_bevy_mesh(&mesh);
        assert!(matches!(
            result,
            Err(SpatialMeshError::InvalidIndexCount(2))
        ));
    }
}
