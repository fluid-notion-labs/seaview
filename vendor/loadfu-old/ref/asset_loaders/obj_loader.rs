//! Custom Bevy AssetLoader for OBJ (Wavefront) files.
//!
//! Supports OBJ files with optional normals and texture coordinates via the `tobj` crate.
//! Produces indexed Bevy `Mesh` assets with positions, normals, and optionally UVs.
//!
//! # Multi-object OBJ files
//!
//! OBJ files can contain multiple objects/groups. This loader combines all meshes
//! into a single Bevy `Mesh`. For multi-object support similar to GLB, a different
//! approach (labeled sub-assets) would be needed.
//!
//! # MTL files
//!
//! Material files (.mtl) are currently ignored. The loader produces geometry only;
//! materials are applied via Seaview's shared material system.

use bevy::asset::io::Reader;
use bevy::asset::{AssetLoader, LoadContext, RenderAssetUsages};
use bevy::mesh::{Indices, Mesh, PrimitiveTopology};
use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use thiserror::Error;

/// Settings for the OBJ asset loader.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjLoaderSettings {
    /// If true, recompute smooth vertex normals from geometry instead of
    /// using normals from the OBJ file (if present).
    /// Defaults to false (use OBJ normals when available).
    #[serde(default)]
    pub recompute_normals: bool,

    /// If true, triangulate non-triangle faces (quads, n-gons).
    /// Defaults to true since Bevy requires triangle meshes.
    #[serde(default = "default_triangulate")]
    pub triangulate: bool,
}

impl Default for ObjLoaderSettings {
    fn default() -> Self {
        Self {
            recompute_normals: false,
            triangulate: true,
        }
    }
}

fn default_triangulate() -> bool {
    true
}

/// Errors that can occur when loading an OBJ file.
#[derive(Debug, Error)]
pub enum ObjLoaderError {
    #[error("IO error reading OBJ data: {0}")]
    Io(#[from] std::io::Error),

    #[error("OBJ parse error: {0}")]
    Parse(#[from] tobj::LoadError),

    #[error("OBJ file contains no meshes")]
    EmptyFile,

    #[error("OBJ file contains no geometry (0 vertices)")]
    EmptyMesh,
}

/// Bevy [`AssetLoader`] that reads `.obj` files and produces [`Mesh`] assets.
///
/// The loader reads the entire file into memory, parses it with `tobj`,
/// merges all objects into a single indexed triangle-list mesh, and returns
/// it as a Bevy [`Mesh`] with positions, normals, and optional UVs.
#[derive(Default, Debug, Clone, Copy, bevy::reflect::TypePath)]
pub struct ObjLoader;

impl AssetLoader for ObjLoader {
    type Asset = Mesh;
    type Settings = ObjLoaderSettings;
    type Error = ObjLoaderError;

    async fn load(
        &self,
        reader: &mut dyn Reader,
        settings: &ObjLoaderSettings,
        _load_context: &mut LoadContext<'_>,
    ) -> Result<Mesh, ObjLoaderError> {
        // Read entire file into memory
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await?;

        let mut cursor = Cursor::new(&bytes);

        // tobj load options
        let load_options = tobj::LoadOptions {
            triangulate: settings.triangulate,
            single_index: true, // Use single index for all attributes (required for Bevy)
            ..Default::default()
        };

        // Load without MTL file (we use our own material system)
        let (models, _materials) = tobj::load_obj_buf(&mut cursor, &load_options, |_mtl_path| {
            // Ignore MTL files - return empty materials
            tobj::MTLLoadResult::Ok((Vec::new(), Default::default()))
        })?;

        if models.is_empty() {
            return Err(ObjLoaderError::EmptyFile);
        }

        let mesh = build_merged_mesh(&models, settings.recompute_normals)?;

        Ok(mesh)
    }

    fn extensions(&self) -> &[&str] {
        &["obj"]
    }
}

/// Build a single Bevy [`Mesh`] by merging all models from the OBJ file.
///
/// Each model's vertices are appended, with indices offset accordingly.
fn build_merged_mesh(
    models: &[tobj::Model],
    recompute_normals: bool,
) -> Result<Mesh, ObjLoaderError> {
    // First pass: count total vertices and indices
    let total_vertices: usize = models.iter().map(|m| m.mesh.positions.len() / 3).sum();
    let total_indices: usize = models.iter().map(|m| m.mesh.indices.len()).sum();

    if total_vertices == 0 {
        return Err(ObjLoaderError::EmptyMesh);
    }

    // Check if we have normals and UVs in the OBJ
    let has_normals = !recompute_normals
        && models
            .iter()
            .all(|m| m.mesh.normals.len() == m.mesh.positions.len());
    let has_uvs = models
        .iter()
        .all(|m| m.mesh.texcoords.len() / 2 == m.mesh.positions.len() / 3);

    // Allocate output buffers
    let mut positions: Vec<[f32; 3]> = Vec::with_capacity(total_vertices);
    let mut normals: Vec<[f32; 3]> = Vec::with_capacity(total_vertices);
    let mut uvs: Vec<[f32; 2]> = Vec::with_capacity(if has_uvs { total_vertices } else { 0 });
    let mut indices: Vec<u32> = Vec::with_capacity(total_indices);

    let mut vertex_offset: u32 = 0;

    for model in models {
        let mesh = &model.mesh;
        let num_vertices = mesh.positions.len() / 3;

        // Copy positions
        for i in 0..num_vertices {
            positions.push([
                mesh.positions[i * 3],
                mesh.positions[i * 3 + 1],
                mesh.positions[i * 3 + 2],
            ]);
        }

        // Copy normals (if present and not recomputing)
        if has_normals {
            for i in 0..num_vertices {
                normals.push([
                    mesh.normals[i * 3],
                    mesh.normals[i * 3 + 1],
                    mesh.normals[i * 3 + 2],
                ]);
            }
        }

        // Copy UVs (if present)
        if has_uvs {
            for i in 0..num_vertices {
                uvs.push([mesh.texcoords[i * 2], mesh.texcoords[i * 2 + 1]]);
            }
        }

        // Copy indices with offset
        for &idx in &mesh.indices {
            indices.push(idx + vertex_offset);
        }

        vertex_offset += num_vertices as u32;
    }

    // Build the Bevy mesh
    let mut bevy_mesh = Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::MAIN_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_indices(Indices::U32(indices));

    // Add UVs if present
    if has_uvs && !uvs.is_empty() {
        bevy_mesh.insert_attribute(Mesh::ATTRIBUTE_UV_0, uvs);
    }

    // Add or compute normals
    if has_normals {
        bevy_mesh.insert_attribute(Mesh::ATTRIBUTE_NORMAL, normals);
    } else {
        // Let Bevy compute smooth normals from the indexed geometry
        bevy_mesh.compute_normals();
    }

    Ok(bevy_mesh)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_settings() {
        let settings = ObjLoaderSettings::default();
        assert!(!settings.recompute_normals);
        assert!(settings.triangulate);
    }

    #[test]
    fn test_extensions() {
        let loader = ObjLoader;
        assert_eq!(loader.extensions(), &["obj"]);
    }
}
