//! Lighting system for night simulation
//!
//! This module provides configurable lighting for visualizing fluid simulations
//! under night-time conditions. It supports multiple light placement algorithms,
//! adjustable height, and cone angles.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};

pub mod placement;
pub mod rig;
pub mod rig_sync;
pub mod systems;

pub use placement::PlacementAlgorithm;
pub use rig::{
    auto_group_name, compute_placement_bounds, Light, LightGroup, LightGroupGenConfig, LightingRig,
};
pub use rig_sync::{LightEntityMap, RigLight, RigLightMarker};

/// Units for light intensity display and input
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LightIntensityUnit {
    /// Lumens (lm) - total luminous flux, Bevy's native unit for spot/point lights
    Lumens,
    /// Candela (cd) - luminous intensity in a direction
    Candela,
    /// Kilolumens (klm) - thousands of lumens, useful for very bright lights
    Kilolumens,
}

impl LightIntensityUnit {
    /// All available units
    pub fn all() -> &'static [LightIntensityUnit] {
        &[
            LightIntensityUnit::Lumens,
            LightIntensityUnit::Candela,
            LightIntensityUnit::Kilolumens,
        ]
    }

    /// Human-readable name
    pub fn name(&self) -> &'static str {
        match self {
            LightIntensityUnit::Lumens => "Lumens (lm)",
            LightIntensityUnit::Candela => "Candela (cd)",
            LightIntensityUnit::Kilolumens => "Kilolumens (klm)",
        }
    }

    /// Short unit symbol
    pub fn symbol(&self) -> &'static str {
        match self {
            LightIntensityUnit::Lumens => "lm",
            LightIntensityUnit::Candela => "cd",
            LightIntensityUnit::Kilolumens => "klm",
        }
    }

    /// Convert a value in this unit to lumens (Bevy's native unit)
    /// For Candela, requires the cone angle to compute solid angle
    pub fn to_lumens(&self, value: f32, cone_angle_degrees: f32) -> f32 {
        match self {
            LightIntensityUnit::Lumens => value,
            LightIntensityUnit::Kilolumens => value * 1000.0,
            LightIntensityUnit::Candela => {
                // Lumens = Candela × solid angle (steradians)
                // For a cone: Ω = 2π(1 - cos(θ/2))
                let half_angle_rad = (cone_angle_degrees / 2.0).to_radians();
                let solid_angle = 2.0 * std::f32::consts::PI * (1.0 - half_angle_rad.cos());
                value * solid_angle
            }
        }
    }

    /// Convert lumens to a value in this unit
    /// For Candela, requires the cone angle to compute solid angle
    pub fn from_lumens(&self, lumens: f32, cone_angle_degrees: f32) -> f32 {
        match self {
            LightIntensityUnit::Lumens => lumens,
            LightIntensityUnit::Kilolumens => lumens / 1000.0,
            LightIntensityUnit::Candela => {
                let half_angle_rad = (cone_angle_degrees / 2.0).to_radians();
                let solid_angle = 2.0 * std::f32::consts::PI * (1.0 - half_angle_rad.cos());
                if solid_angle > 0.0 {
                    lumens / solid_angle
                } else {
                    0.0
                }
            }
        }
    }
}

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

/// Resource that configures the night lighting system
#[derive(Resource, Clone, Debug, Serialize, Deserialize)]
pub struct NightLightingConfig {
    /// Number of lights to spawn
    pub num_lights: usize,

    /// Height of lights above sea level (in meters)
    pub height: f32,

    /// Full cone angle for spotlights (in degrees).
    /// Internally halved to produce the SpotLight outer_angle (half-angle), clamped to 90°.
    pub cone_angle: f32,

    /// Placement algorithm for distributing lights
    pub placement_algorithm: PlacementAlgorithm,

    /// Whether the lighting system is enabled
    pub enabled: bool,

    /// Light intensity in LUMENS (Bevy's native unit).
    /// Use `intensity_display_value` and `intensity_unit` for UI display/input.
    pub intensity: f32,

    /// Display value for intensity in the selected unit
    #[serde(skip)]
    pub intensity_display_value: f32,

    /// Unit for displaying/inputting intensity
    pub intensity_unit: LightIntensityUnit,

    /// Light color
    pub color: Color,

    /// Light range (distance)
    pub range: f32,

    /// Whether to show visual markers for light positions
    pub show_markers: bool,

    /// Size of the marker spheres (in meters)
    pub marker_size: f32,

    /// Coverage percentage of mesh XZ extent (100 = exactly covers the mesh footprint,
    /// 150 = extends 50% beyond the mesh in each direction, etc.)
    pub coverage_pct: f32,
}

impl NightLightingConfig {
    /// Update the internal lumens value from the display value and unit
    pub fn sync_intensity_to_lumens(&mut self) {
        self.intensity = self
            .intensity_unit
            .to_lumens(self.intensity_display_value, self.cone_angle);
    }

    /// Update the display value from the internal lumens value
    pub fn sync_intensity_from_lumens(&mut self) {
        self.intensity_display_value = self
            .intensity_unit
            .from_lumens(self.intensity, self.cone_angle);
    }

    /// Change the display unit and update the display value accordingly
    pub fn set_intensity_unit(&mut self, new_unit: LightIntensityUnit) {
        self.intensity_unit = new_unit;
        self.sync_intensity_from_lumens();
    }
}

impl Default for NightLightingConfig {
    fn default() -> Self {
        let intensity_lumens = 800_000.0;
        let cone_angle = 60.0;
        let unit = LightIntensityUnit::Kilolumens;
        let display_value = unit.from_lumens(intensity_lumens, cone_angle);

        Self {
            num_lights: 9,
            height: 50.0,
            cone_angle,
            placement_algorithm: PlacementAlgorithm::UniformGrid,
            enabled: true,
            intensity: intensity_lumens,
            intensity_display_value: display_value,
            intensity_unit: unit,
            color: Color::srgb(1.0, 0.95, 0.9), // Warm white
            range: 500.0,
            show_markers: true,
            marker_size: 0.5,
            coverage_pct: 120.0,
        }
    }
}

/// Marker component for night lighting spotlights
#[derive(Component)]
pub struct NightLight {
    /// Index of this light in the grid
    pub index: usize,
}

/// Marker component for the visual sphere representing a light source
#[derive(Component)]
pub struct NightLightMarker {
    /// Index of the associated light
    pub light_index: usize,
}

/// Plugin that manages the night lighting system
pub struct NightLightingPlugin;

impl Plugin for NightLightingPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(NightLightingConfig::default())
            .insert_resource(EnvironmentLightConfig::default())
            .insert_resource(LightingRig::default())
            .insert_resource(LightEntityMap::default())
            .add_systems(
                Update,
                (
                    systems::update_night_lights,
                    systems::apply_environment_light,
                    rig_sync::sync_lighting_rig,
                ),
            );
    }
}
