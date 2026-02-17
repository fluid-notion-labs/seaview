//! UI systems module
//!
//! This module contains all the UI systems and their registration logic.

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;

pub mod environment_panel;
mod event_handlers;
mod material_system;
mod menu_bar;
mod playback_controls;
mod status_bar;
mod toast_events;
mod toast_overlay;

pub use environment_panel::*;
pub use event_handlers::*;
pub use material_system::*;
pub use menu_bar::*;
pub use playback_controls::*;
pub use status_bar::*;
pub use toast_events::*;
pub use toast_overlay::*;

/// Plugin that registers all UI systems
pub struct UiSystemsPlugin;

impl Plugin for UiSystemsPlugin {
    fn build(&self, app: &mut App) {
        // Insert the resource that tracks previous loading phase for transition detection
        app.init_resource::<PreviousLoadingPhaseState>();

        // Register all UI systems to run in the EguiPrimaryContextPass schedule
        app.add_systems(
            EguiPrimaryContextPass,
            (
                menu_bar_system,
                environment_panel_system,
                environment_objects_panel_system,
                new_session_dialog_system,
                delete_confirmation_dialog_system,
                message_display_system,
                playback_controls_system,
                toast_overlay_system,
                status_bar_system,
            ),
        );

        // Register update systems that run outside of egui pass
        app.add_systems(
            Update,
            (
                handle_switch_session_events,
                handle_delete_session_events,
                handle_create_session_events,
                apply_material_config,
                sync_wireframe_config,
                toast_events_system,
            ),
        );
    }
}
