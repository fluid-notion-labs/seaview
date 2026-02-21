//! Lighting system for Seaview
//!
//! This module provides configurable lighting for visualizing fluid simulations.
//! The v2 lighting rig uses named groups with per-light parameters, managed by
//! [`LightingRig`].

use bevy::prelude::*;

pub mod environment_map;
pub mod placement;
pub mod presets;
pub mod rig;
pub mod rig_sync;
pub mod systems;

pub use environment_map::{
    EnvironmentMapActive, EnvironmentMapCatalog, EnvironmentMapConfig, EnvironmentMapEntry,
    EnvironmentMapSource, PendingHdrCubemaps, discover_hdr_files, process_pending_hdr_cubemaps,
    sync_environment_map,
};
pub use placement::PlacementAlgorithm;
pub use presets::{EnvironmentPreset, MaterialPreset};
pub use rig::{
    auto_group_name, compute_placement_bounds, Light, LightGroup, LightGroupGenConfig, LightingRig,
};
pub use rig_sync::{LightEntityMap, RigLight, RigLightMarker};

/// Marker component for global scene lights (directional, point, ambient)
/// so they can be toggled on/off from the UI.
#[derive(Component)]
pub struct GlobalLight;

/// Configuration for the environment (global) lighting: directional lights,
/// point lights, and ambient light.  Lives in its own resource so the
/// "Environment Light" UI section is independent of the spot-light rig.
#[derive(Resource, Clone, Debug)]
pub struct EnvironmentLightConfig {
    /// Master toggle — hides/shows directional + point + ambient lights.
    pub enabled: bool,

    /// Ambient light colour.
    pub ambient_color: Color,

    /// Ambient light brightness (Bevy `GlobalAmbientLight.brightness`).
    pub ambient_brightness: f32,

    /// Directional light illuminance (lux).
    pub directional_illuminance: f32,

    /// Directional light colour.
    pub directional_color: Color,
}

impl Default for EnvironmentLightConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            ambient_color: Color::WHITE,
            ambient_brightness: 500.0,
            directional_illuminance: 10000.0,
            directional_color: Color::WHITE,
        }
    }
}

/// Plugin that manages the lighting systems.
///
/// Registers the v2 [`LightingRig`] resource, entity sync system, and
/// the environment (global) light system.
pub struct NightLightingPlugin;

impl Plugin for NightLightingPlugin {
    fn build(&self, app: &mut App) {
        // Build the environment map catalog with known KTX2 maps
        let mut catalog = EnvironmentMapCatalog::default();
        catalog.register("Pisa", "pisa");

        app.insert_resource(EnvironmentLightConfig::default())
            .insert_resource(LightingRig::default())
            .insert_resource(LightEntityMap::default())
            .insert_resource(EnvironmentMapConfig::default())
            .insert_resource(catalog)
            .init_resource::<PendingHdrCubemaps>()
            .add_systems(Startup, discover_hdr_environment_maps)
            .add_systems(
                Update,
                (
                    systems::apply_environment_light,
                    rig_sync::sync_lighting_rig,
                    sync_environment_map,
                    process_pending_hdr_cubemaps,
                ),
            );
    }
}

/// Startup system: scan the `environment_maps/` asset directory for `.hdr`
/// and `.exr` files and register them in the [`EnvironmentMapCatalog`].
fn discover_hdr_environment_maps(mut catalog: ResMut<EnvironmentMapCatalog>) {
    // Replicate Bevy's asset base path resolution:
    //   BEVY_ASSET_ROOT > CARGO_MANIFEST_DIR > exe parent directory
    // Then join "assets" (Bevy's default file_path).
    let base = if let Ok(dir) = std::env::var("BEVY_ASSET_ROOT") {
        std::path::PathBuf::from(dir)
    } else if let Ok(dir) = std::env::var("CARGO_MANIFEST_DIR") {
        std::path::PathBuf::from(dir)
    } else {
        std::env::current_exe()
            .map(|p| p.parent().map(|p| p.to_owned()).unwrap_or_default())
            .unwrap_or_default()
    };

    let assets_dir = base.join("assets");

    let discovered = discover_hdr_files(&assets_dir);
    let count = discovered.len();

    for (name, asset_path) in discovered {
        info!("Discovered HDR environment map: '{}' → {}", name, asset_path);
        catalog.register_hdr(name, asset_path);
    }

    if count > 0 {
        info!(
            "Environment map catalog: {} entries ({} discovered HDR/EXR)",
            catalog.entries.len(),
            count,
        );
    }
}
