//! Mesh information module for Seaview
//!
//! Computes and caches axis-aligned bounding box (AABB) dimensions for the
//! currently loaded mesh. Results are persisted to `seaview.toml` so the UI
//! can display dimensions immediately on subsequent launches.

use bevy::prelude::*;
use bevy::mesh::VertexAttributeValues;

use super::sequence::loader::SequenceMeshDisplay;
use super::settings::{MeshBoundsSettings, SettingsResource};

/// Plugin that registers mesh-info systems and resources.
pub struct MeshInfoPlugin;

impl Plugin for MeshInfoPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MeshDimensions>()
            .add_message::<RecomputeMeshBounds>()
            .add_systems(
                Update,
                (
                    compute_bounds_on_first_load,
                    handle_recompute_request,
                ),
            );
    }
}

/// Event that triggers a recomputation of the mesh AABB (e.g. from a UI button).
#[derive(Message)]
pub struct RecomputeMeshBounds;

/// Resource holding the current mesh's bounding-box information.
///
/// `None` means no mesh has been measured yet.
#[derive(Resource, Default, Debug, Clone)]
pub struct MeshDimensions {
    /// Axis-aligned minimum corner (metres)
    pub min: Option<Vec3>,
    /// Axis-aligned maximum corner (metres)
    pub max: Option<Vec3>,
    /// Dimensions (max − min) per axis (metres)
    pub dimensions: Option<Vec3>,
    /// Whether we already attempted auto-computation for this load
    pub computed: bool,
}

impl MeshDimensions {
    /// Populate from a `MeshBoundsSettings` (loaded from seaview.toml).
    pub fn from_settings(s: &MeshBoundsSettings) -> Self {
        Self {
            min: Some(Vec3::from_array(s.min)),
            max: Some(Vec3::from_array(s.max)),
            dimensions: Some(Vec3::from_array(s.dimensions)),
            computed: true,
        }
    }

    /// Convert to `MeshBoundsSettings` for serialisation.
    pub fn to_settings(&self) -> Option<MeshBoundsSettings> {
        match (self.min, self.max, self.dimensions) {
            (Some(mn), Some(mx), Some(dim)) => Some(MeshBoundsSettings {
                min: mn.to_array(),
                max: mx.to_array(),
                dimensions: dim.to_array(),
            }),
            _ => None,
        }
    }

    /// Reset so the next frame triggers recomputation.
    pub fn invalidate(&mut self) {
        self.min = None;
        self.max = None;
        self.dimensions = None;
        self.computed = false;
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Automatically compute bounds the first time a `SequenceMeshDisplay` entity
/// appears and we haven't computed yet.
fn compute_bounds_on_first_load(
    mut dims: ResMut<MeshDimensions>,
    mesh_query: Query<&Mesh3d, With<SequenceMeshDisplay>>,
    meshes: Res<Assets<Mesh>>,
    mut settings_res: ResMut<SettingsResource>,
) {
    if dims.computed {
        return;
    }

    let Ok(mesh_handle) = mesh_query.single() else {
        return;
    };

    let Some(mesh) = meshes.get(&mesh_handle.0) else {
        return;
    };

    if let Some((mn, mx)) = compute_aabb(mesh) {
        let d = mx - mn;
        debug!(
            "Mesh AABB computed — min: ({:.2}, {:.2}, {:.2}), max: ({:.2}, {:.2}, {:.2}), dims: {:.2} × {:.2} × {:.2} m",
            mn.x, mn.y, mn.z, mx.x, mx.y, mx.z, d.x, d.y, d.z
        );
        dims.min = Some(mn);
        dims.max = Some(mx);
        dims.dimensions = Some(d);
        dims.computed = true;

        // Persist to seaview.toml
        save_bounds_to_settings(&dims, &mut settings_res);
    }
}

/// Respond to an explicit recompute request (e.g. the UI button).
fn handle_recompute_request(
    mut events: MessageReader<RecomputeMeshBounds>,
    mut dims: ResMut<MeshDimensions>,
    mesh_query: Query<&Mesh3d, With<SequenceMeshDisplay>>,
    meshes: Res<Assets<Mesh>>,
    mut settings_res: ResMut<SettingsResource>,
) {
    // Drain all pending events; we only need to recompute once.
    let mut should_recompute = false;
    for _event in events.read() {
        should_recompute = true;
    }
    if !should_recompute {
        return;
    }

    let Ok(mesh_handle) = mesh_query.single() else {
        warn!("Recompute requested but no mesh entity found");
        return;
    };

    let Some(mesh) = meshes.get(&mesh_handle.0) else {
        warn!("Recompute requested but mesh asset not loaded yet");
        return;
    };

    if let Some((mn, mx)) = compute_aabb(mesh) {
        let d = mx - mn;
        debug!(
            "Mesh AABB recomputed — dims: {:.2} × {:.2} × {:.2} m",
            d.x, d.y, d.z
        );
        dims.min = Some(mn);
        dims.max = Some(mx);
        dims.dimensions = Some(d);
        dims.computed = true;

        save_bounds_to_settings(&dims, &mut settings_res);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Walk all position vertices and return (min, max).
fn compute_aabb(mesh: &Mesh) -> Option<(Vec3, Vec3)> {
    let positions = mesh.attribute(Mesh::ATTRIBUTE_POSITION)?;
    match positions {
        VertexAttributeValues::Float32x3(verts) => {
            if verts.is_empty() {
                return None;
            }
            let mut mn = Vec3::new(f32::MAX, f32::MAX, f32::MAX);
            let mut mx = Vec3::new(f32::MIN, f32::MIN, f32::MIN);
            for v in verts {
                let p = Vec3::new(v[0], v[1], v[2]);
                mn = mn.min(p);
                mx = mx.max(p);
            }
            Some((mn, mx))
        }
        _ => {
            warn!("Mesh positions are not Float32x3 — cannot compute AABB");
            None
        }
    }
}

/// Persist the current bounds into the settings resource and flush to disk.
fn save_bounds_to_settings(dims: &MeshDimensions, settings_res: &mut SettingsResource) {
    if let Some(bounds) = dims.to_settings() {
        settings_res.settings.mesh = Some(bounds);
        if let Err(e) = settings_res.save() {
            warn!("Failed to save mesh bounds to seaview.toml: {}", e);
        } else {
            debug!("Mesh bounds cached to seaview.toml");
        }
    }
}
