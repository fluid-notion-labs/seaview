//! Systems for managing night lighting
//!
//! This module contains Bevy systems that handle spawning, updating, and
//! despawning night lights based on the NightLightingConfig resource.

use bevy::prelude::*;
use std::f32::consts::FRAC_PI_2;

use super::{
    EnvironmentLightConfig, GlobalLight, NightLight, NightLightMarker, NightLightingConfig,
};
use crate::lib::mesh_info::MeshDimensions;

/// Convert the UI cone angle (full angle in degrees) to a valid SpotLight outer_angle
/// (half-angle in radians, clamped to PI/2 max).
fn cone_to_outer_angle(cone_angle_degrees: f32) -> f32 {
    let half_angle_rad = (cone_angle_degrees * 0.5).to_radians();
    half_angle_rad.min(FRAC_PI_2)
}

/// Compute inner_angle as 80% of outer_angle
fn cone_to_inner_angle(cone_angle_degrees: f32) -> f32 {
    cone_to_outer_angle(cone_angle_degrees) * 0.8
}

/// Compute the XZ placement bounds from the mesh AABB and the coverage percentage.
///
/// `coverage_pct` of 100 means the lights exactly span the mesh footprint.
/// Values above 100 extend beyond the mesh; below 100 cover a smaller area.
/// Falls back to a default ±100 square when no mesh dimensions are available.
fn compute_placement_bounds(mesh_dims: &MeshDimensions, coverage_pct: f32) -> (Vec2, Vec2) {
    let scale = coverage_pct / 100.0;

    if let (Some(mn), Some(mx)) = (mesh_dims.min, mesh_dims.max) {
        // Centre of the mesh on the XZ plane
        let cx = (mn.x + mx.x) * 0.5;
        let cz = (mn.z + mx.z) * 0.5;

        // Half-extents scaled by coverage
        let hx = (mx.x - mn.x) * 0.5 * scale;
        let hz = (mx.z - mn.z) * 0.5 * scale;

        (Vec2::new(cx - hx, cz - hz), Vec2::new(cx + hx, cz + hz))
    } else {
        // Fallback when no mesh is loaded
        let fallback = 100.0 * scale;
        (
            Vec2::new(-fallback, -fallback),
            Vec2::new(fallback, fallback),
        )
    }
}

/// System that updates night lights based on configuration changes
///
/// This system:
/// - Spawns new lights when config changes
/// - Updates existing light positions, angles, and properties
/// - Despawns lights when they're no longer needed
pub fn update_night_lights(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    config: Res<NightLightingConfig>,
    mesh_dims: Res<MeshDimensions>,
    existing_lights: Query<(Entity, &NightLight)>,
    existing_markers: Query<(Entity, &NightLightMarker)>,
) {
    // If lighting is disabled, despawn all lights and markers
    if !config.enabled {
        for (entity, _) in existing_lights.iter() {
            commands.entity(entity).despawn();
        }
        for (entity, _) in existing_markers.iter() {
            commands.entity(entity).despawn();
        }
        return;
    }

    // Don't spawn lights until mesh dimensions are computed
    // This avoids positioning at the fallback ±100 location
    if mesh_dims.min.is_none() || mesh_dims.max.is_none() {
        return;
    }

    // Only update if config or mesh dims have changed
    if !config.is_changed() && !mesh_dims.is_changed() {
        return;
    }

    // Get current number of lights and markers
    let current_count = existing_lights.iter().count();
    let target_count = config.num_lights;
    let marker_count = existing_markers.iter().count();

    // If count changed, despawn all and respawn
    // (In the future, we could be smarter about only adding/removing what's needed)
    if current_count != target_count {
        for (entity, _) in existing_lights.iter() {
            commands.entity(entity).despawn();
        }
        for (entity, _) in existing_markers.iter() {
            commands.entity(entity).despawn();
        }

        spawn_lights(
            &mut commands,
            &mut meshes,
            &mut materials,
            &config,
            &mesh_dims,
        );
    } else {
        // Update existing lights' properties
        for (entity, light) in existing_lights.iter() {
            update_light_transform(&mut commands, entity, light.index, &config, &mesh_dims);
        }

        // Update marker visibility, size, and positions
        update_markers(
            &mut commands,
            &mut meshes,
            &mut materials,
            &existing_markers,
            &config,
            &mesh_dims,
            marker_count,
        );
    }
}

/// Spawn all lights according to current configuration
fn spawn_lights(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    config: &NightLightingConfig,
    mesh_dims: &MeshDimensions,
) {
    let (bounds_min, bounds_max) = compute_placement_bounds(mesh_dims, config.coverage_pct);

    // Calculate light positions using the configured algorithm
    let positions =
        config
            .placement_algorithm
            .calculate_positions(config.num_lights, bounds_min, bounds_max);

    // Spawn each light
    for (index, pos) in positions.iter().enumerate() {
        let transform = calculate_light_transform(*pos, config.height, config.cone_angle);

        commands.spawn((
            SpotLight {
                intensity: config.intensity,
                color: config.color,
                range: config.range,
                outer_angle: cone_to_outer_angle(config.cone_angle),
                inner_angle: cone_to_inner_angle(config.cone_angle),
                shadows_enabled: true,
                ..default()
            },
            transform,
            NightLight { index },
        ));

        // Spawn marker sphere if enabled
        if config.show_markers {
            spawn_marker(
                commands,
                meshes,
                materials,
                *pos,
                config.height,
                index,
                config,
            );
        }
    }
}

/// Update transform for an existing light
fn update_light_transform(
    commands: &mut Commands,
    entity: Entity,
    index: usize,
    config: &NightLightingConfig,
    mesh_dims: &MeshDimensions,
) {
    let (bounds_min, bounds_max) = compute_placement_bounds(mesh_dims, config.coverage_pct);

    // Calculate light positions
    let positions =
        config
            .placement_algorithm
            .calculate_positions(config.num_lights, bounds_min, bounds_max);

    if let Some(pos) = positions.get(index) {
        let transform = calculate_light_transform(*pos, config.height, config.cone_angle);

        commands.entity(entity).insert((
            transform,
            SpotLight {
                intensity: config.intensity,
                color: config.color,
                range: config.range,
                outer_angle: cone_to_outer_angle(config.cone_angle),
                inner_angle: cone_to_inner_angle(config.cone_angle),
                shadows_enabled: true,
                ..default()
            },
        ));
    }
}

/// Calculate transform for a light at the given XZ position
fn calculate_light_transform(pos: Vec2, height: f32, _cone_angle: f32) -> Transform {
    let translation = Vec3::new(pos.x, height, pos.y);

    // Point straight down — looking_at a point directly below
    Transform::from_translation(translation).looking_at(Vec3::new(pos.x, 0.0, pos.y), Vec3::Z)
}

/// Spawn a marker sphere at the light position
fn spawn_marker(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    pos: Vec2,
    height: f32,
    index: usize,
    config: &NightLightingConfig,
) {
    let sphere_mesh = meshes.add(Sphere::new(config.marker_size));
    let material = materials.add(StandardMaterial {
        base_color: config.color,
        emissive: (config.color.to_linear() * 2.0).into(),
        ..default()
    });

    commands.spawn((
        Mesh3d(sphere_mesh),
        MeshMaterial3d(material),
        Transform::from_xyz(pos.x, height, pos.y),
        NightLightMarker { light_index: index },
    ));
}

/// Update marker visibility, size, and positions
fn update_markers(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
    existing_markers: &Query<(Entity, &NightLightMarker)>,
    config: &NightLightingConfig,
    mesh_dims: &MeshDimensions,
    marker_count: usize,
) {
    if !config.show_markers {
        // Hide markers by despawning them
        for (entity, _) in existing_markers.iter() {
            commands.entity(entity).despawn();
        }
    } else if marker_count == 0 && config.num_lights > 0 {
        // Markers were toggled on - spawn them
        let (bounds_min, bounds_max) = compute_placement_bounds(mesh_dims, config.coverage_pct);

        let positions = config.placement_algorithm.calculate_positions(
            config.num_lights,
            bounds_min,
            bounds_max,
        );

        for (index, pos) in positions.iter().enumerate() {
            spawn_marker(
                commands,
                meshes,
                materials,
                *pos,
                config.height,
                index,
                config,
            );
        }
    } else {
        let (bounds_min, bounds_max) = compute_placement_bounds(mesh_dims, config.coverage_pct);

        // Calculate light positions
        let positions = config.placement_algorithm.calculate_positions(
            config.num_lights,
            bounds_min,
            bounds_max,
        );

        // Update marker meshes, positions, and materials
        let new_sphere = meshes.add(Sphere::new(config.marker_size));
        for (entity, marker) in existing_markers.iter() {
            if let Some(pos) = positions.get(marker.light_index) {
                commands.entity(entity).insert((
                    Mesh3d(new_sphere.clone()),
                    Transform::from_xyz(pos.x, config.height, pos.y),
                ));
            }
        }
    }
}

/// System that applies the environment (global) light configuration.
///
/// Reads [`EnvironmentLightConfig`] and updates the directional/point light
/// visibility, ambient colour/brightness, and directional illuminance.
pub fn apply_environment_light(
    config: Res<EnvironmentLightConfig>,
    mut global_lights: Query<(&mut Visibility, Option<&mut DirectionalLight>), With<GlobalLight>>,
    mut ambient: ResMut<GlobalAmbientLight>,
) {
    if !config.is_changed() {
        return;
    }

    let visibility = if config.enabled {
        Visibility::Inherited
    } else {
        Visibility::Hidden
    };

    for (mut vis, dir_light) in global_lights.iter_mut() {
        *vis = visibility;
        // Update directional light properties when present.
        if let Some(mut dl) = dir_light {
            dl.illuminance = config.directional_illuminance;
            dl.color = config.directional_color;
        }
    }

    // Ambient light
    if config.enabled {
        ambient.color = config.ambient_color;
        ambient.brightness = config.ambient_brightness;
    } else {
        ambient.brightness = 0.0;
    }
}
