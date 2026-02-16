//! BVH debug visualization via Bevy gizmos.
//!
//! This module provides the [`draw_bvh_gizmos`] system that renders wireframe
//! AABBs for the BVH nodes of the currently displayed frame's spatial mesh.
//!
//! The visualization is controlled by [`BvhGizmoConfig`], which is toggled
//! from the Spatial section of the environment panel.

use bevy::math::bounding::Aabb3d;
use bevy::prelude::*;

use crate::lib::sequence::loader::SpatialIndex;
use crate::lib::sequence::FrameStore;

// ---------------------------------------------------------------------------
// Configuration resource
// ---------------------------------------------------------------------------

/// Controls BVH wireframe debug visualization.
#[derive(Resource)]
pub struct BvhGizmoConfig {
    /// Whether to draw BVH wireframe boxes.
    pub enabled: bool,

    /// Maximum BVH depth level to display (0 = root children only).
    pub max_depth: u32,

    /// Whether to show leaf nodes.
    pub show_leaves: bool,

    /// Whether to show internal (non-leaf) nodes.
    pub show_internal: bool,

    /// Global opacity multiplier for the gizmo lines (0.0–1.0).
    pub opacity: f32,
}

impl Default for BvhGizmoConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            max_depth: 4,
            show_leaves: true,
            show_internal: true,
            opacity: 0.6,
        }
    }
}

// ---------------------------------------------------------------------------
// Color palette
// ---------------------------------------------------------------------------

/// Pick a color for a BVH node based on its depth and leaf/internal status.
///
/// Internal nodes cycle through a warm palette (red → orange → yellow → white)
/// as depth increases.  Leaf nodes are always green.
fn color_for_node(depth: u32, is_leaf: bool, opacity: f32) -> Color {
    if is_leaf {
        Color::srgba(0.2, 0.9, 0.3, opacity * 0.8)
    } else {
        // Warm gradient: deeper nodes get cooler / brighter
        let palette: &[Color] = &[
            Color::srgba(0.9, 0.15, 0.15, opacity),       // depth 0 — red
            Color::srgba(0.95, 0.5, 0.1, opacity),        // depth 1 — orange
            Color::srgba(0.95, 0.85, 0.1, opacity),       // depth 2 — yellow
            Color::srgba(0.6, 0.95, 0.2, opacity * 0.9),  // depth 3 — lime
            Color::srgba(0.2, 0.8, 0.95, opacity * 0.85), // depth 4 — cyan
            Color::srgba(0.5, 0.4, 0.95, opacity * 0.8),  // depth 5 — purple
            Color::srgba(0.9, 0.4, 0.9, opacity * 0.75),  // depth 6 — magenta
        ];
        let idx = (depth as usize).min(palette.len() - 1);
        palette[idx]
    }
}

// ---------------------------------------------------------------------------
// System
// ---------------------------------------------------------------------------

/// Draws wireframe AABB boxes for BVH nodes of the currently displayed frame.
///
/// Reads the [`BvhGizmoConfig`] to decide whether to draw, what depth levels
/// to include, and which node types (leaf / internal) to show.
///
/// The coordinate transform from [`LoaderConfig`] is applied so the boxes
/// align with the rendered mesh.
pub fn draw_bvh_gizmos(
    config: Res<BvhGizmoConfig>,
    frame_store: Res<FrameStore>,
    spatial_index: Res<SpatialIndex>,
    mut gizmos: Gizmos,
) {
    if !config.enabled {
        return;
    }

    let Some(current_frame) = frame_store.displayed_frame else {
        return;
    };

    let Some(spatial_meshes) = spatial_index.get(current_frame) else {
        return;
    };

    for sm in spatial_meshes {
        let nodes = sm.bvh_node_aabbs(config.max_depth);

        for node in &nodes {
            // Filter by node type
            if node.is_leaf && !config.show_leaves {
                continue;
            }
            if !node.is_leaf && !config.show_internal {
                continue;
            }

            let min = Vec3::from(node.min);
            let max = Vec3::from(node.max);
            let center = (min + max) * 0.5;
            let half_extents = (max - min) * 0.5;

            // Skip degenerate boxes (zero extent on all axes)
            if half_extents.length_squared() < 1e-10 {
                continue;
            }

            let color = color_for_node(node.depth, node.is_leaf, config.opacity);

            let aabb = Aabb3d::new(center, half_extents);
            gizmos.aabb_3d(aabb, Transform::IDENTITY, color);
        }
    }
}

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// Plugin that registers the BVH debug gizmo system and its config resource.
pub struct BvhGizmosPlugin;

impl Plugin for BvhGizmosPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<BvhGizmoConfig>()
            .add_systems(PostUpdate, draw_bvh_gizmos);
    }
}
