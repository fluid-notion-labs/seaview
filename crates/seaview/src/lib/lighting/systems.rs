//! Environment light system
//!
//! Applies the [`EnvironmentLightConfig`] resource to the global scene lights
//! (directional + ambient). The v1 spot-light system (`update_night_lights`)
//! has been removed — spot lights are now managed by the v2 rig sync system
//! in [`super::rig_sync`].

use bevy::prelude::*;

use super::{EnvironmentLightConfig, GlobalLight};

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
