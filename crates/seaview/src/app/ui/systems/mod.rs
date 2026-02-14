//! UI systems module
//!
//! This module contains all the UI systems and their registration logic.

use bevy::prelude::*;
use bevy_egui::EguiPrimaryContextPass;

mod event_handlers;
mod material_system;
mod menu_bar;
mod playback_controls;
pub mod environment_panel;

pub use event_handlers::*;
pub use material_system::*;
pub use menu_bar::*;
pub use playback_controls::*;
pub use environment_panel::*;

/// Plugin that registers all UI systems
pub struct UiSystemsPlugin;

impl Plugin for UiSystemsPlugin {
    fn build(&self, app: &mut App) {
        // Register all UI systems to run in the EguiPrimaryContextPass schedule
        app.add_systems(
            EguiPrimaryContextPass,
            (
                menu_bar_system,
                environment_panel_system,
                new_session_dialog_system,
                delete_confirmation_dialog_system,
                message_display_system,
                playback_controls_system,
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
            ),
        );
    }
}
