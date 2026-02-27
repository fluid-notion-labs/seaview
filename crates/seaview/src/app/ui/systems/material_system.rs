//! Material system for Seaview
//!
//! This module provides systems that reactively apply MaterialConfig
//! changes to the sequence mesh's StandardMaterial, and sync wireframe
//! overlay state to Bevy's `WireframeConfig`.

use crate::app::ui::state::{MaterialConfig, RenderConfig};
use crate::lib::sequence::loader::SequenceMeshDisplay;
use bevy::pbr::wireframe::WireframeConfig;
use bevy::prelude::*;

/// System that applies MaterialConfig changes to the mesh material in real time
pub fn apply_material_config(
    config: Res<MaterialConfig>,
    mesh_query: Query<&MeshMaterial3d<StandardMaterial>, With<SequenceMeshDisplay>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !config.is_changed() {
        return;
    }

    for material_handle in mesh_query.iter() {
        if let Some(material) = materials.get_mut(&material_handle.0) {
            // Apply opacity to base color alpha
            let mut color = config.base_color;
            color.set_alpha(config.opacity);
            material.base_color = color;
            material.perceptual_roughness = config.perceptual_roughness;
            material.metallic = config.metallic;
            material.reflectance = config.reflectance;

            // Emissive: combine color with intensity
            if config.emissive_intensity > 0.0 {
                material.emissive =
                    (config.emissive.to_linear() * config.emissive_intensity).into();
            } else {
                material.emissive = Color::BLACK.into();
            }

            // Double-sided rendering
            material.double_sided = config.double_sided;
            material.cull_mode = if config.double_sided {
                None
            } else {
                Some(bevy::render::render_resource::Face::Back)
            };

            // Auto-derive alpha mode from opacity
            material.alpha_mode = if config.opacity < 1.0 {
                AlphaMode::Blend
            } else {
                AlphaMode::Opaque
            };
        }
    }
}

/// System that syncs `RenderConfig::wireframe_enabled` to Bevy's `WireframeConfig::global`.
///
/// Only writes when the value actually changes, to avoid unnecessary change-detection triggers.
pub fn sync_wireframe_config(
    render_config: Res<RenderConfig>,
    mut wireframe_config: ResMut<WireframeConfig>,
) {
    if !render_config.is_changed() {
        return;
    }

    if wireframe_config.global != render_config.wireframe_enabled {
        wireframe_config.global = render_config.wireframe_enabled;
        debug!(
            "Wireframe overlay {}",
            if render_config.wireframe_enabled {
                "enabled"
            } else {
                "disabled"
            }
        );
    }
}
