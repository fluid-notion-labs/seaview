//! Spatial accelerator plugin — bundles all BVH / spatial-index systems.
//!
//! This plugin owns:
//!
//! - **[`SpatialIndex`]** resource (per-frame BVH acceleration structures)
//! - **[`build_spatial_meshes`]** system (time-budgeted BVH construction)
//! - **[`BvhGizmoConfig`]** resource + **[`draw_bvh_gizmos`]** system
//!
//! Gate the entire subsystem behind the `spatial` feature flag:
//!
//! ```toml
//! [features]
//! spatial = []
//! ```
//!
//! In `main.rs`:
//!
//! ```ignore
//! #[cfg(feature = "spatial")]
//! app.add_plugins(SpatialAcceleratorPlugin);
//! ```
//!
//! When the plugin is **not** loaded, the `SpatialIndex` and `BvhGizmoConfig`
//! resources simply don't exist.  Systems that consume them should accept
//! `Option<Res<SpatialIndex>>` / `Option<ResMut<BvhGizmoConfig>>` and
//! gracefully skip work when the resource is absent.

use bevy::prelude::*;

use crate::app::systems::bvh_gizmos::{draw_bvh_gizmos, BvhGizmoConfig};
use crate::lib::sequence::loader::spatial_index;
pub use crate::lib::sequence::loader::spatial_index::SpatialIndex;

/// Plugin that registers all spatial acceleration resources and systems.
///
/// When this plugin is added, the loader will populate [`SpatialIndex`] with
/// per-frame BVH structures (time-budgeted), and the BVH gizmo visualisation
/// becomes available in the environment panel.
pub struct SpatialAcceleratorPlugin;

impl Plugin for SpatialAcceleratorPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SpatialIndex>()
            .init_resource::<BvhGizmoConfig>()
            // build_spatial_meshes reads FrameLoadedEvents via MessageReader.
            // Messages written this frame are readable next frame (double-
            // buffered), so no explicit ordering against the loader chain is
            // needed — one tick of latency on BVH building is irrelevant.
            .add_systems(Update, spatial_index::build_spatial_meshes)
            .add_systems(PostUpdate, draw_bvh_gizmos);
    }
}
