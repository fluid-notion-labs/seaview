//! User interface module for Seaview
//!
//! This module contains all UI-related functionality using bevy_egui.
//! The UI is organized into separate submodules for different features.

use bevy::prelude::*;
use bevy_egui::EguiPlugin;

pub mod state;
pub mod systems;
pub mod widgets;

pub use state::UiState;
pub use systems::*;

/// Main UI plugin that sets up all UI functionality
pub struct SeaviewUiPlugin;

impl Plugin for SeaviewUiPlugin {
    fn build(&self, app: &mut App) {
        // Add bevy_egui plugin if not already added
        if !app.is_plugin_added::<EguiPlugin>() {
            app.add_plugins(EguiPlugin::default());
        }

        // Add UI state management
        app.add_plugins(state::UiStatePlugin);

        // Register UI systems
        app.add_plugins(systems::UiSystemsPlugin);

        // Log that the UI plugin is loaded
        debug!("Seaview UI Plugin loaded - Phase 0.2 egui integration active");

        // Log initial UI state
        let ui_state = app.world().resource::<UiState>();
        debug!("Initial UI state - show_session_panel: {}, show_network_panel: {}, show_playback_controls: {}",
              ui_state.show_session_panel,
              ui_state.show_network_panel,
              ui_state.show_playback_controls);
    }
}
