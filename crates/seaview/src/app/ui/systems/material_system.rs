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
    _frame_store: Res<crate::lib::sequence::loader::FrameStore>,
    mesh_query: Query<&MeshMaterial3d<StandardMaterial>, With<SequenceMeshDisplay>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    if !config.is_changed() {
        return;
    }

    // Meshlet sequences require opaque, single-sided materials.
    #[cfg(feature = "meshlet")]
    let is_meshlet = _frame_store.format == Some(crate::lib::sequence::loader::SequenceKind::Meshlet);
    #[cfg(not(feature = "meshlet"))]
    let is_meshlet = false;

    for material_handle in mesh_query.iter() {
        if let Some(material) = materials.get_mut(&material_handle.0) {
            let mut color = config.base_color;
            color.set_alpha(config.opacity);
            material.base_color = color;
            material.perceptual_roughness = config.perceptual_roughness;
            material.metallic = config.metallic;
            material.reflectance = config.reflectance;

            if config.emissive_intensity > 0.0 {
                material.emissive =
                    (config.emissive.to_linear() * config.emissive_intensity).into();
            } else {
                material.emissive = Color::BLACK.into();
            }

            // Meshlet: force opaque + single-sided
            if is_meshlet {
                material.double_sided = false;
                material.cull_mode = Some(bevy::render::render_resource::Face::Back);
                material.alpha_mode = AlphaMode::Opaque;
            } else {
                material.double_sided = config.double_sided;
                material.cull_mode = if config.double_sided {
                    None
                } else {
                    Some(bevy::render::render_resource::Face::Back)
                };
                material.alpha_mode = if config.opacity < 1.0 {
                    AlphaMode::Blend
                } else {
                    AlphaMode::Opaque
                };
            }
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

/// System that toggles meshlet cluster debug visualization.
///
/// When enabled, replaces `MeshMaterial3d<StandardMaterial>` with
/// `MeshMaterial3d<MeshletDebugMaterial>` on all `SequenceMeshDisplay` entities,
/// coloring each meshlet cluster a distinct color for debugging.
///
/// Only compiled with `--features meshlet`.
#[cfg(feature = "meshlet")]
pub fn sync_meshlet_debug(
    render_config: Res<RenderConfig>,
    mut commands: Commands,
    loader_config: Res<crate::lib::sequence::loader::LoaderConfig>,
    std_query: Query<
        (Entity, &MeshMaterial3d<StandardMaterial>),
        With<SequenceMeshDisplay>,
    >,
    debug_query: Query<
        (Entity, &MeshMaterial3d<crate::app::meshlet_debug::MeshletDebugMaterial>),
        With<SequenceMeshDisplay>,
    >,
    debug_materials: Option<ResMut<Assets<crate::app::meshlet_debug::MeshletDebugMaterial>>>,
) {
    if !render_config.is_changed() {
        return;
    }

    // MeshletPlugin not loaded — nothing to toggle.
    let Some(mut debug_materials) = debug_materials else {
        return;
    };

    let Some(ref std_handle) = loader_config.shared_material else {
        return;
    };

    if render_config.meshlet_debug_enabled {
        // Swap StandardMaterial → MeshletDebugMaterial
        let debug_handle = debug_materials.add(crate::app::meshlet_debug::MeshletDebugMaterial::default());
        for (entity, _) in std_query.iter() {
            commands
                .entity(entity)
                .remove::<MeshMaterial3d<StandardMaterial>>()
                .insert(MeshMaterial3d(debug_handle.clone()));
        }
        debug!("Meshlet debug visualization enabled");
    } else {
        // Swap MeshletDebugMaterial → StandardMaterial
        for (entity, _) in debug_query.iter() {
            commands
                .entity(entity)
                .remove::<MeshMaterial3d<crate::app::meshlet_debug::MeshletDebugMaterial>>()
                .insert(MeshMaterial3d(std_handle.clone()));
        }
        debug!("Meshlet debug visualization disabled");
    }
}
