//! Lighting v2 entity sync system
//!
//! Synchronises the [`LightingRig`] resource to Bevy `SpotLight` entities.
//! Each [`Light`](super::rig::Light) in the rig maps to one `SpotLight`
//! entity and (optionally) one marker-sphere entity.
//!
//! Runs every frame; the system is cheap when nothing changes because it
//! early-exits on `!rig.is_changed()`.

use bevy::prelude::*;
use std::collections::HashMap;
use std::f32::consts::FRAC_PI_2;
use uuid::Uuid;

use super::rig::LightingRig;

// ---------------------------------------------------------------------------
// Components & resources
// ---------------------------------------------------------------------------

/// Marker component for SpotLight entities managed by the v2 lighting rig.
#[derive(Component)]
pub struct RigLight {
    pub light_id: Uuid,
}

/// Marker component for the visual sphere representing a v2 rig light.
#[derive(Component)]
pub struct RigLightMarker {
    pub light_id: Uuid,
}

/// Resource that maps rig light UUIDs to their Bevy entities.
///
/// Populated and maintained exclusively by [`sync_lighting_rig`].
#[derive(Resource, Default, Debug)]
pub struct LightEntityMap {
    /// `Light.id` → `SpotLight` entity
    pub lights: HashMap<Uuid, Entity>,
    /// `Light.id` → marker sphere entity
    pub markers: HashMap<Uuid, Entity>,
}

// ---------------------------------------------------------------------------
// Cone-angle helpers (shared with v1 but kept local to avoid coupling)
// ---------------------------------------------------------------------------

/// Full cone angle (degrees) → SpotLight `outer_angle` (half-angle, radians,
/// clamped to π/2).
fn cone_to_outer_angle(cone_angle_degrees: f32) -> f32 {
    let half_angle_rad = (cone_angle_degrees * 0.5).to_radians();
    half_angle_rad.min(FRAC_PI_2)
}

/// Inner angle = 80 % of outer angle for a smooth fall-off.
fn cone_to_inner_angle(cone_angle_degrees: f32) -> f32 {
    cone_to_outer_angle(cone_angle_degrees) * 0.8
}

/// Build a downward-looking transform for a light at the given XZ + height.
fn light_transform(pos: Vec2, height: f32) -> Transform {
    let translation = Vec3::new(pos.x, height, pos.y);
    Transform::from_translation(translation).looking_at(Vec3::new(pos.x, 0.0, pos.y), Vec3::Z)
}

// ---------------------------------------------------------------------------
// Sync system
// ---------------------------------------------------------------------------

/// System that keeps Bevy `SpotLight` entities in sync with the
/// [`LightingRig`] resource.
///
/// * **Spawn** — new lights get a `SpotLight` entity and a marker sphere.
/// * **Update** — existing lights have their transform, SpotLight params, and
///   visibility patched in place.
/// * **Despawn** — lights that no longer exist in the rig have their entities
///   removed.
///
/// The system early-exits when neither the rig nor the mesh dimensions have
/// changed, keeping per-frame cost near zero.
pub fn sync_lighting_rig(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    rig: Res<LightingRig>,
    mut entity_map: ResMut<LightEntityMap>,
) {
    // -- Early-exit when nothing changed ----------------------------------
    if !rig.is_changed() {
        return;
    }

    // -- Collect the set of light IDs currently in the rig ----------------
    let mut live_ids: HashMap<Uuid, (bool, &super::rig::Light)> = HashMap::new();
    for group in &rig.groups {
        for light in &group.lights {
            live_ids.insert(light.id, (group.visible, light));
        }
    }

    // -- Despawn entities for lights that no longer exist ------------------
    let stale_light_ids: Vec<Uuid> = entity_map
        .lights
        .keys()
        .filter(|id| !live_ids.contains_key(id))
        .copied()
        .collect();

    for id in &stale_light_ids {
        if let Some(entity) = entity_map.lights.remove(id) {
            commands.entity(entity).despawn();
        }
        if let Some(entity) = entity_map.markers.remove(id) {
            commands.entity(entity).despawn();
        }
    }

    // -- Shared marker mesh/material (created lazily) ---------------------
    let mut marker_mesh: Option<Handle<Mesh>> = None;
    let mut marker_material: Option<Handle<StandardMaterial>> = None;
    let marker_radius: f32 = 0.5;

    let get_marker_handles = |meshes: &mut Assets<Mesh>,
                              materials: &mut Assets<StandardMaterial>,
                              mesh_h: &mut Option<Handle<Mesh>>,
                              mat_h: &mut Option<Handle<StandardMaterial>>|
     -> (Handle<Mesh>, Handle<StandardMaterial>) {
        let m = mesh_h
            .get_or_insert_with(|| meshes.add(Sphere::new(marker_radius)))
            .clone();
        let mat = mat_h
            .get_or_insert_with(|| {
                materials.add(StandardMaterial {
                    base_color: Color::srgb(1.0, 0.95, 0.9),
                    emissive: LinearRgba::new(2.0, 1.9, 1.8, 1.0),
                    ..default()
                })
            })
            .clone();
        (m, mat)
    };

    // -- Spawn / update each light ----------------------------------------
    for (light_id, (group_visible, light)) in &live_ids {
        let effective_visible = *group_visible && light.visible;
        let visibility = if effective_visible {
            Visibility::Inherited
        } else {
            Visibility::Hidden
        };

        let transform = light_transform(light.position, light.height);

        let spot = SpotLight {
            intensity: light.intensity,
            color: light.color,
            range: light.range,
            outer_angle: cone_to_outer_angle(light.cone_angle),
            inner_angle: cone_to_inner_angle(light.cone_angle),
            shadow_maps_enabled: true,
            ..default()
        };

        if let Some(&entity) = entity_map.lights.get(light_id) {
            // -- Update existing entity -----------------------------------
            commands.entity(entity).insert((spot, transform, visibility));
        } else {
            // -- Spawn new entity -----------------------------------------
            let entity = commands
                .spawn((
                    spot,
                    transform,
                    visibility,
                    RigLight {
                        light_id: *light_id,
                    },
                ))
                .id();
            entity_map.lights.insert(*light_id, entity);
        }

        // -- Marker sphere ------------------------------------------------
        let marker_transform = Transform::from_xyz(light.position.x, light.height, light.position.y);

        if let Some(&marker_entity) = entity_map.markers.get(light_id) {
            commands
                .entity(marker_entity)
                .insert((marker_transform, visibility));
        } else {
            let (mesh_h, mat_h) = get_marker_handles(
                &mut meshes,
                &mut materials,
                &mut marker_mesh,
                &mut marker_material,
            );
            let marker_entity = commands
                .spawn((
                    Mesh3d(mesh_h),
                    MeshMaterial3d(mat_h),
                    marker_transform,
                    visibility,
                    RigLightMarker {
                        light_id: *light_id,
                    },
                ))
                .id();
            entity_map.markers.insert(*light_id, marker_entity);
        }
    }
}

/// Despawn **all** v2 rig entities and clear the entity map.
///
/// Useful when tearing down the lighting rig (e.g. on scene unload).
pub fn despawn_all_rig_lights(
    mut commands: Commands,
    mut entity_map: ResMut<LightEntityMap>,
) {
    for (_id, entity) in entity_map.lights.drain() {
        commands.entity(entity).despawn();
    }
    for (_id, entity) in entity_map.markers.drain() {
        commands.entity(entity).despawn();
    }
}
