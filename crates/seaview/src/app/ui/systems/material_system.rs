//! Material system for Seaview
//!
//! This module provides a system that reactively applies MaterialConfig
//! changes to the sequence mesh's StandardMaterial.

use bevy::prelude::*;
use crate::app::ui::state::MaterialConfig;
use crate::lib::sequence::loader::SequenceMeshDisplay;

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
                material.emissive = (config.emissive.to_linear() * config.emissive_intensity).into();
            } else {
                material.emissive = Color::BLACK.into();
            }

            // Double-sided rendering
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
