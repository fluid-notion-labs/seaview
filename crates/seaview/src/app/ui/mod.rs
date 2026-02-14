//! User interface module for Seaview
//!
//! This module contains all UI-related functionality using bevy_egui.
//! The UI is organized into separate submodules for different features.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts, EguiPlugin};

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

        // Add startup system to configure egui fonts
        app.add_systems(Startup, configure_egui_fonts);

        // Log that the UI plugin is loaded
        debug!("Seaview UI Plugin loaded - Phase 0.2 egui integration active");

        // Log initial UI state
        let ui_state = app.world().resource::<UiState>();
        debug!("Initial UI state - show_environment_panel: {}, show_network_panel: {}, show_playback_controls: {}",
              ui_state.show_environment_panel,
              ui_state.show_network_panel,
              ui_state.show_playback_controls);
    }
}

/// Configures egui font sizes for better readability in the sidebar
fn configure_egui_fonts(mut contexts: EguiContexts) {
    let Ok(ctx) = contexts.ctx_mut() else {
        return;
    };

    // Get the current style and modify font sizes
    let mut style = (*ctx.style()).clone();

    // Configure text styles with appropriate font sizes
    style.text_styles.insert(
        egui::TextStyle::Heading,
        egui::FontId::new(16.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Body,
        egui::FontId::new(13.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Button,
        egui::FontId::new(13.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Small,
        egui::FontId::new(11.0, egui::FontFamily::Proportional),
    );
    style.text_styles.insert(
        egui::TextStyle::Monospace,
        egui::FontId::new(12.0, egui::FontFamily::Monospace),
    );

    // Apply the modified style
    ctx.set_style(style);

    debug!("Configured egui font sizes: Heading=16px, Body=13px, Button=13px, Small=11px, Monospace=12px");
}
