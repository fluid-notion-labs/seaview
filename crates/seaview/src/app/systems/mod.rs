pub mod bvh_gizmos;
pub mod camera;
pub mod diagnostics;
// pub mod network;
pub mod window;

use bevy::pbr::ScreenSpaceReflections;
use bevy::prelude::*;

use crate::app::systems::camera::FpsCamera;
use crate::app::ui::state::SsrConfig;

/// Plugin that registers app-level systems (window management, etc.)
pub struct AppSystemsPlugin;

impl Plugin for AppSystemsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SsrConfig>()
            .add_systems(Update, (window::fullscreen_toggle, sync_ssr_config));
    }
}

/// Sync [`SsrConfig`] resource → camera's [`ScreenSpaceReflections`] component.
///
/// When SSR is enabled, inserts or updates the component with the current
/// settings.  When disabled, removes the component entirely so Bevy skips
/// the SSR render pass (zero GPU cost).
fn sync_ssr_config(
    ssr_config: Res<SsrConfig>,
    mut commands: Commands,
    camera_query: Query<(Entity, Option<&ScreenSpaceReflections>), With<FpsCamera>>,
) {
    if !ssr_config.is_changed() {
        return;
    }

    for (entity, existing_ssr) in camera_query.iter() {
        if ssr_config.enabled {
            let component = ssr_config.to_component();
            if existing_ssr.is_some() {
                // Update in place — the component already exists.
                commands.entity(entity).insert(component);
            } else {
                // First enable — add the component.
                commands.entity(entity).insert(component);
            }
        } else if existing_ssr.is_some() {
            // Disabled — remove the component so Bevy skips the SSR pass.
            commands.entity(entity).remove::<ScreenSpaceReflections>();
        }
    }
}
