//! Menu bar system for Seaview
//!
//! This module implements the top menu bar with File, Session, View, Network, and Help menus.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::app::ui::state::{CreateSessionEvent, SessionSourceType, UiState};

/// System that renders the top menu bar
pub fn menu_bar_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    mut create_session_events: MessageWriter<CreateSessionEvent>,
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

                // Session menu
                ui.menu_button("Session", |ui| {
                    if ui.button("New Session...").clicked() {
                        ui_state.temp_state.show_new_session_dialog = true;
                        ui.close();
                    }

                    ui.separator();

                    if ui.button("New Network Session").clicked() {
                        create_session_events.write(CreateSessionEvent {
                            name: format!(
                                "Network Session {}",
                                chrono::Local::now().format("%Y-%m-%d %H:%M")
                            ),
                            source_type: SessionSourceType::Network { port: 9877 },
                        });
                        ui.close();
                    }

                    ui.separator();

                    ui.add_enabled_ui(ui_state.active_session.is_some(), |ui| {
                        if ui.button("Save Session").clicked() {
                            // TODO: Implement session saving
                            ui_state.show_info("Session saving not yet implemented");
                            ui.close();
                        }

                        if ui.button("Export Session...").clicked() {
                            // TODO: Implement session export
                            ui_state.show_info("Session export not yet implemented");
                            ui.close();
                        }
                    });
                });

                // View menu
                ui.menu_button("View", |ui| {
                    if ui
                        .checkbox(&mut ui_state.show_session_panel, "Session Panel")
                        .clicked()
                    {
                        ui.close();
                    }

                    if ui
                        .checkbox(&mut ui_state.show_network_panel, "Network Panel")
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

                    if ui
                        .checkbox(&mut ui_state.show_perf_panel, "Performance")
                        .clicked()
                    {
                        ui.close();
                    }

                    ui.separator();

                    if ui.button("Reset Layout").clicked() {
                        ui_state.panel_sizes = Default::default();
                        ui_state.show_session_panel = true;
                        ui_state.show_network_panel = true;
                        ui_state.show_playback_controls = true;
                        ui.close();
                    }
                });

                // Network menu
                ui.menu_button("Network", |ui| {
                    if ui.button("Start Network Receiver...").clicked() {
                        // TODO: Show network configuration dialog
                        ui_state.show_info("Network configuration dialog not yet implemented");
                        ui.close();
                    }

                    if ui.button("Stop All Receivers").clicked() {
                        // TODO: Implement stopping all network receivers
                        ui_state.show_info("Stop receivers not yet implemented");
                        ui.close();
                    }

                    ui.separator();

                    if ui.button("Network Statistics").clicked() {
                        // TODO: Show network statistics window
                        ui_state.show_info("Network statistics not yet implemented");
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
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    mut create_session_events: MessageWriter<CreateSessionEvent>,
) {
    if !ui_state.temp_state.show_new_session_dialog {
        return;
    }

    let ctx = contexts.ctx_mut().unwrap();

    let mut show_dialog = ui_state.temp_state.show_new_session_dialog;
    let mut session_name = String::from("New Session");
    let mut selected_source = 0; // 0: Network, 1: File, 2: Data Lake

    egui::Window::new("New Session")
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.label("Session Name:");
                ui.text_edit_singleline(&mut session_name);
            });

            ui.add_space(10.0);

            ui.label("Source Type:");
            ui.radio_value(&mut selected_source, 0, "Network");
            ui.radio_value(&mut selected_source, 1, "File");
            ui.radio_value(&mut selected_source, 2, "Data Lake");

            ui.add_space(20.0);

            ui.horizontal(|ui| {
                if ui.button("Create").clicked() {
                    let source_type = match selected_source {
                        0 => SessionSourceType::Network { port: 9877 },
                        1 => SessionSourceType::File {
                            path: std::path::PathBuf::new(),
                        },
                        2 => SessionSourceType::DataLake {
                            connection_string: String::new(),
                        },
                        _ => unreachable!(),
                    };

                    create_session_events.write(CreateSessionEvent {
                        name: session_name.clone(),
                        source_type,
                    });

                    show_dialog = false;
                }

                if ui.button("Cancel").clicked() {
                    show_dialog = false;
                }
            });
        });

    ui_state.temp_state.show_new_session_dialog = show_dialog;
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
