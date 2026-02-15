//! Custom Bevy AssetLoader for STL (STereoLithography) files.
//!
//! Supports both binary and ASCII STL formats via the `stl_io` crate.
//! Produces indexed Bevy `Mesh` assets with positions, normals, and u32 indices.

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext, RenderAssetUsages};
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use thiserror::Error;

/// Settings for the STL asset loader.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StlLoaderSettings {
    /// If true, recompute smooth vertex normals from geometry instead of
    /// using the per-face normals stored in the STL file.
    /// Defaults to false (use per-vertex normals derived from face normals).
    #[serde(default)]
    pub recompute_normals: bool,
}

/// Errors that can occur when loading an STL file.
#[derive(Debug, Error)]
pub enum StlLoaderError {
    #[error("IO error reading STL data: {0}")]
    Io(#[from] std::io::Error),

    #[error("STL file contains no triangles")]
    EmptyMesh,
}

/// Bevy [`AssetLoader`] that reads `.stl` files and produces [`Mesh`] assets.
///
/// The loader reads the entire file into memory, parses it with `stl_io`
/// (auto-detecting binary vs ASCII), builds an indexed triangle-list mesh,
/// and returns it as a Bevy [`Mesh`] with positions, normals, and u32 indices.
#[derive(Default, Debug, Clone, Copy, bevy::reflect::TypePath)]
pub struct StlLoader;

impl AssetLoader for StlLoader {
    type Asset = Mesh;
    type Settings = StlLoaderSettings;
    type Error = StlLoaderError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        settings: &StlLoaderSettings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Mesh, StlLoaderError> {
        // Read entire file into memory so we can hand it to stl_io
        // (which requires Read + Seek).
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;

        let mut cursor = Cursor::new(&bytes);
        let indexed_mesh = stl_io::read_stl(&mut cursor)?;

        if indexed_mesh.faces.is_empty() {
            return Err(StlLoaderError::EmptyMesh);
        }

        let mesh = if settings.recompute_normals {
            build_mesh_auto_normals(&indexed_mesh)
        } else {
            build_mesh_with_stl_normals(&indexed_mesh)
        };

        Ok(mesh)
    }

    fn extensions(&self) -> &[&str] {
        &["stl"]
    }
}

/// Build a Bevy [`Mesh`] using per-vertex normals computed by averaging the
/// face normals of all faces that share each vertex (smooth shading).
///
/// This uses the indexed representation directly: each unique vertex position
/// from the STL gets one entry, and Bevy's `compute_normals` produces smooth
/// normals from the triangle connectivity.
fn build_mesh_auto_normals(stl: &stl_io::IndexedMesh) -> Mesh {
    let positions: Vec<[f32; 3]> = stl
        .vertices
        .iter()
        .map(|v| {
            let arr: [f32; 3] = (*v).into();
            arr
        })
        .collect();

    let indices: Vec<u32> = stl
        .faces
        .iter()
        .flat_map(|face| face.vertices.iter().map(|&idx| idx as u32))
        .collect();

    let mut mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_indices(Indices::U32(indices));

    // Let Bevy compute smooth normals from the indexed geometry.
    mesh.compute_normals();

    mesh
}

/// Build a Bevy [`Mesh`] using the per-face normals stored in the STL file,
/// expanded to per-vertex normals.
///
/// STL stores one normal per triangle. To feed this into Bevy's indexed mesh
/// where normals are per-vertex, we accumulate (add) each face normal onto
/// every vertex the face references, then normalize. Vertices shared by
/// faces with similar normals end up smooth; vertices at hard edges (where
/// face normals diverge) get a reasonable average.
fn build_mesh_with_stl_normals(stl: &stl_io::IndexedMesh) -> Mesh {
    let num_vertices = stl.vertices.len();
    let positions: Vec<[f32; 3]> = stl
        .vertices
        .iter()
        .map(|v| {
            let arr: [f32; 3] = (*v).into();
            arr
        })
        .collect();

    // Accumulate face normals onto each vertex.
    let mut normals = vec![[0.0f32; 3]; num_vertices];

    for face in &stl.faces {
        let n: [f32; 3] = face.normal.into();
        for &vi in &face.vertices {
            normals[vi][0] += n[0];
            normals[vi][1] += n[1];
            normals[vi][2] += n[2];
        }
    }

    // Normalize accumulated normals. Fall back to +Y for degenerate cases.
    for normal in &mut normals {
        let len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
        if len > f32::EPSILON {
            normal[0] /= len;
            normal[1] /= len;
            normal[2] /= len;
        } else {
            *normal = [0.0, 1.0, 0.0];
        }
    }

    let indices: Vec<u32> = stl
        .faces
        .iter()
        .flat_map(|face| face.vertices.iter().map(|&idx| idx as u32))
        .collect();

    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, normals)
    .with_inserted_indices(Indices::U32(indices))
}
