//! Menu bar system for Seaview
//!
//! This module implements the top menu bar with File, View, and Help menus.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::app::ui::state::UiState;

/// System that renders the top menu bar
pub fn menu_bar_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    mut exit: MessageWriter<AppExit>,
) {
    let ctx = contexts.ctx_mut().unwrap();

    egui::TopBottomPanel::top("menu_bar")
        .exact_height(ui_state.panel_sizes.menu_bar_height)
        .show(ctx, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                // File menu
                ui.menu_button("File", |ui| {
                    if ui.button("Open File...").clicked() {
                        // TODO: Implement file picker
                        ui_state.show_info("File picker not yet implemented");
                        ui.close();
                    }

                    if ui.button("Open Directory...").clicked() {
                        // TODO: Implement directory picker
                        ui_state.show_info("Directory picker not yet implemented");
                        ui.close();
                    }

                    ui.separator();

                    if ui.button("Exit").clicked() {
                        exit.write(AppExit::Success);
                        ui.close();
                    }
                });

                // View menu
                ui.menu_button("View", |ui| {
                    if ui
                        .checkbox(&mut ui_state.show_environment_panel, "Environment")
                        .clicked()
                    {
                        ui.close();
                    }

                    if ui
                        .checkbox(&mut ui_state.show_playback_controls, "Playback Controls")
                        .clicked()
                    {
                        ui.close();
                    }

                    ui.separator();

                    if ui.button("Reset Layout").clicked() {
                        ui_state.panel_sizes = Default::default();
                        ui_state.show_environment_panel = true;
                        ui_state.show_playback_controls = true;
                        ui.close();
                    }
                });

                // Help menu
                ui.menu_button("Help", |ui| {
                    if ui.button("Controls").clicked() {
                        ui_state.show_info(
                            "Camera Controls:\n\
                            • WASD: Move\n\
                            • Q/E: Down/Up\n\
                            • Mouse: Look (click to grab)\n\
                            • Escape: Release cursor\n\
                            • Shift: Fast mode\n\
                            • Alt: Slow mode",
                        );
                        ui.close();
                    }

                    ui.separator();

                    if ui.button("About Seaview").clicked() {
                        ui_state.show_info(
                            "Seaview Mesh Viewer\n\
                            Version 0.1.0\n\n\
                            A real-time mesh visualization tool for fluid simulations.",
                        );
                        ui.close();
                    }
                });
            });
        });
}

/// System that shows the new session dialog
pub fn new_session_dialog_system(
    mut _contexts: EguiContexts,
    mut _ui_state: ResMut<UiState>,
) {
    // Stub — session creation is no longer exposed in the UI.
    // Kept as a no-op to satisfy the system registration in UiSystemsPlugin.
}

/// System that displays error and info messages
pub fn message_display_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    _time: Res<Time>,
) {
    let ctx = contexts.ctx_mut().unwrap();

    // Display error message if present
    let mut clear_error = false;
    if let Some(error_msg) = &ui_state.temp_state.error_message {
        egui::Window::new("Error")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 100.0])
            .show(ctx, |ui| {
                ui.colored_label(egui::Color32::from_rgb(255, 100, 100), error_msg);
                if ui.button("OK").clicked() {
                    clear_error = true;
                }
            });
    }
    if clear_error {
        ui_state.temp_state.error_message = None;
    }

    // Display info message if present
    let mut clear_info = false;
    if let Some(info_msg) = &ui_state.temp_state.info_message {
        egui::Window::new("Information")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_TOP, [0.0, 100.0])
            .show(ctx, |ui| {
                ui.label(info_msg);
                if ui.button("OK").clicked() {
                    clear_info = true;
                }
            });
    }
    if clear_info {
        ui_state.temp_state.info_message = None;
    }
}
