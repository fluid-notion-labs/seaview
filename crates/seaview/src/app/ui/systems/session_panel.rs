//! Session panel system for Seaview
//!
//! This module implements the left side panel that displays and manages sessions,
//! network status, and lighting rig configuration using an accordion-style UI.

use bevy::diagnostic::DiagnosticsStore;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};
use uuid::Uuid;

use crate::app::systems::diagnostics::RenderingStats;
use crate::app::ui::state::{AlphaModeConfig, DeleteSessionEvent, MaterialConfig, RenderConfig, SwitchSessionEvent, UiState};
use crate::app::ui::systems::perf_panel::render_perf_section;
use crate::lib::lighting::{NightLightingConfig, PlacementAlgorithm};
use crate::lib::mesh_info::{MeshDimensions, RecomputeMeshBounds};
use crate::lib::sequence::SequenceAssets;
use crate::lib::session::SessionManager;

/// System that renders the session management panel
pub fn session_panel_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    mut switch_events: MessageWriter<SwitchSessionEvent>,
    _delete_events: MessageWriter<DeleteSessionEvent>,
    session_manager: Res<SessionManager>,
    mut lighting_config: ResMut<NightLightingConfig>,
    mut material_config: ResMut<MaterialConfig>,
    mut render_config: ResMut<RenderConfig>,
    mesh_dims: Res<MeshDimensions>,
    sequence_assets: Res<SequenceAssets>,
    mut recompute_events: MessageWriter<RecomputeMeshBounds>,
    diagnostics: Res<DiagnosticsStore>,
    rendering_stats: Res<RenderingStats>,
) {
    if !ui_state.show_session_panel {
        debug!("Session panel is hidden");
        return;
    }

    debug!("Rendering session panel");

    let ctx = contexts.ctx_mut().unwrap();

    // Get real sessions from SessionManager
    let sessions = session_manager.get_all_sessions();

    let panel_width = ui_state.panel_sizes.session_panel_width;

    egui::SidePanel::left("session_panel")
        .resizable(true)
        .default_width(panel_width)
        .width_range(200.0..=500.0)
        .show(ctx, |ui| {
            debug!("Session panel UI callback running");
            // Update panel width if resized
            ui_state.panel_sizes.session_panel_width = ui.available_width();

            // Main scroll area for all accordion sections
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // Sessions Section (Collapsible)
                    egui::CollapsingHeader::new("Sessions")
                        .default_open(ui_state.collapsible.sessions_open)
                        .show(ui, |ui| {
                            ui_state.collapsible.sessions_open = true;

                            if sessions.is_empty() {
                                ui.label("No sessions yet");
                                ui.label("Create a new session from the Session menu");
                            } else {
                                for session in sessions {
                                    let is_active = ui_state
                                        .active_session
                                        .map(|id| id == session.id)
                                        .unwrap_or(false);

                                    let delete_requested = ui
                                        .push_id(session.id, |ui| {
                                            render_session_item(
                                                ui,
                                                session,
                                                is_active,
                                                &mut switch_events,
                                            )
                                        })
                                        .inner;

                                    if let Some(session_id) = delete_requested {
                                        ui_state.temp_state.show_delete_confirmation =
                                            Some(session_id);
                                    }

                                    ui.add_space(5.0);
                                }
                            }
                        });

                    ui.add_space(5.0);

                    // Network Status Section (Collapsible)
                    if ui_state.show_network_panel {
                        egui::CollapsingHeader::new("Network Status")
                            .default_open(ui_state.collapsible.network_status_open)
                            .show(ui, |ui| {
                                ui_state.collapsible.network_status_open = true;

                                // Get sessions again to avoid borrow conflict
                                let sessions_for_network = session_manager.get_all_sessions();
                                let network_sessions: Vec<_> = sessions_for_network
                                    .iter()
                                    .filter_map(|s| match &s.source {
                                        crate::lib::session::types::SessionSource::Network {
                                            port,
                                            ..
                                        } => Some((*port, s.frame_count())),
                                        _ => None,
                                    })
                                    .collect();

                                if network_sessions.is_empty() {
                                    ui.horizontal(|ui| {
                                        ui.label("Status:");
                                        ui.colored_label(
                                            egui::Color32::from_rgb(255, 255, 0),
                                            "● No Active",
                                        );
                                    });
                                } else {
                                    for (port, frame_count) in network_sessions {
                                        ui.horizontal(|ui| {
                                            ui.label("Port:");
                                            ui.label(format!("{}", port));
                                        });
                                        ui.horizontal(|ui| {
                                            ui.label("Frames:");
                                            ui.label(format!("{}", frame_count));
                                        });
                                    }
                                }

                                ui.add_space(10.0);

                                ui.horizontal(|ui| {
                                    if ui.button("Start").clicked() {
                                        debug!("Starting network receiver");
                                    }
                                    if ui.button("Stop").clicked() {
                                        debug!("Stopping network receiver");
                                    }
                                });
                            });

                        ui.add_space(5.0);
                    }

                    // Mesh Info Section (Collapsible)
                    egui::CollapsingHeader::new("Mesh Info")
                        .default_open(true)
                        .show(ui, |ui| {
                            if let Some(dims) = mesh_dims.dimensions {
                                ui.horizontal(|ui| {
                                    ui.label("Dimensions:");
                                });
                                egui::Grid::new("mesh_dims_grid")
                                    .num_columns(2)
                                    .spacing([8.0, 4.0])
                                    .show(ui, |ui| {
                                        ui.label("X:");
                                        ui.label(format!("{:.2} m", dims.x));
                                        ui.end_row();
                                        ui.label("Y:");
                                        ui.label(format!("{:.2} m", dims.y));
                                        ui.end_row();
                                        ui.label("Z:");
                                        ui.label(format!("{:.2} m", dims.z));
                                        ui.end_row();
                                    });

                                if let (Some(mn), Some(mx)) = (mesh_dims.min, mesh_dims.max) {
                                    ui.add_space(4.0);
                                    ui.horizontal(|ui| {
                                        ui.label("Min:");
                                        ui.label(format!("({:.1}, {:.1}, {:.1})", mn.x, mn.y, mn.z));
                                    });
                                    ui.horizontal(|ui| {
                                        ui.label("Max:");
                                        ui.label(format!("({:.1}, {:.1}, {:.1})", mx.x, mx.y, mx.z));
                                    });
                                }
                            } else {
                                ui.label("No mesh loaded");
                            }

                            ui.add_space(4.0);
                            if ui.button("⟳ Recompute").clicked() {
                                recompute_events.write(RecomputeMeshBounds);
                            }
                        });

                    ui.add_space(5.0);

                    // Lighting Rig Section (Collapsible)
                    egui::CollapsingHeader::new("Lighting Rig")
                        .default_open(ui_state.collapsible.lighting_rig_open)
                        .show(ui, |ui| {
                            ui_state.collapsible.lighting_rig_open = true;

                            render_lighting_rig_controls(ui, &mut lighting_config, &mesh_dims);
                        });

                    ui.add_space(5.0);

                    // Material Section (Collapsible)
                    egui::CollapsingHeader::new("Material")
                        .default_open(ui_state.collapsible.material_open)
                        .show(ui, |ui| {
                            ui_state.collapsible.material_open = true;

                            render_material_controls(ui, &mut material_config);
                        });

                    ui.add_space(5.0);

                    // Rendering Section (Collapsible)
                    render_rendering_section(ui, &mut ui_state, &mut render_config, &sequence_assets);

                    ui.add_space(5.0);

                    // Performance Section (Collapsible)
                    render_perf_section(ui, &mut ui_state, &diagnostics, &rendering_stats);
                });
        });
}

/// Render a single session item in the list
fn render_session_item(
    ui: &mut egui::Ui,
    session: &crate::lib::session::Session,
    is_active: bool,
    switch_events: &mut MessageWriter<SwitchSessionEvent>,
) -> Option<Uuid> {
    let mut delete_requested = None;
    let frame_color = if is_active {
        egui::Color32::from_rgb(100, 150, 255)
    } else {
        egui::Color32::from_gray(80)
    };

    egui::Frame::new()
        .fill(if is_active {
            egui::Color32::from_gray(50)
        } else {
            egui::Color32::from_gray(30)
        })
        .stroke(egui::Stroke::new(1.0, frame_color))
        .inner_margin(8.0)
        .show(ui, |ui| {
            ui.vertical(|ui| {
                // Session name
                ui.horizontal(|ui| {
                    if is_active {
                        ui.label(
                            egui::RichText::new(&session.name)
                                .strong()
                                .color(egui::Color32::from_rgb(150, 200, 255)),
                        );
                    } else {
                        ui.label(&session.name);
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        // Delete button
                        if ui
                            .small_button("🗑")
                            .on_hover_text("Delete session")
                            .clicked()
                        {
                            delete_requested = Some(session.id);
                        }
                    });
                });

                // Session info
                ui.label(
                    egui::RichText::new(format!("{} frames", session.frame_count()))
                        .small()
                        .color(egui::Color32::from_gray(180)),
                );
                ui.label(
                    egui::RichText::new(session.source.display_string())
                        .small()
                        .color(egui::Color32::from_gray(180)),
                );
                ui.label(
                    egui::RichText::new(session.created_at.format("%Y-%m-%d %H:%M").to_string())
                        .small()
                        .color(egui::Color32::from_gray(150)),
                );

                // Activate button
                if !is_active {
                    ui.add_space(5.0);
                    if ui.button("Activate").clicked() {
                        switch_events.write(SwitchSessionEvent {
                            session_id: session.id,
                        });
                    }
                }
            });
        });

    delete_requested
}

/// Render lighting rig controls
fn render_lighting_rig_controls(ui: &mut egui::Ui, config: &mut NightLightingConfig, mesh_dims: &MeshDimensions) {
    // Global lighting toggle
    ui.horizontal(|ui| {
        ui.label("Global Lights:");
        ui.checkbox(&mut config.global_lighting_enabled, "");
    });

    ui.add_space(5.0);
    ui.separator();

    // Spot lighting enable/disable toggle
    ui.horizontal(|ui| {
        ui.label("Spot Lights:");
        ui.checkbox(&mut config.enabled, "");
    });

    ui.add_space(5.0);

    // Only show spot light controls if enabled
    if !config.enabled {
        ui.label(egui::RichText::new("Enable to configure").color(egui::Color32::GRAY));
        return;
    }

    ui.separator();

    // Number of lights slider
    ui.horizontal(|ui| {
        ui.label("Lights:");
        ui.add(egui::Slider::new(&mut config.num_lights, 1..=100).suffix(" lights"));
    });

    ui.add_space(5.0);

    // Placement algorithm chooser
    ui.horizontal(|ui| {
        ui.label("Layout:");
    });
    egui::ComboBox::from_id_salt("placement_algorithm")
        .selected_text(config.placement_algorithm.name())
        .show_ui(ui, |ui| {
            for algorithm in PlacementAlgorithm::all() {
                ui.selectable_value(
                    &mut config.placement_algorithm,
                    *algorithm,
                    algorithm.name(),
                );
            }
        });

    ui.add_space(5.0);

    // Coverage percentage slider
    ui.horizontal(|ui| {
        ui.label("Coverage:");
    });
    ui.add(egui::Slider::new(&mut config.coverage_pct, 10.0..=300.0).suffix(" %"));

    ui.add_space(5.0);

    // 2D Light Map — top-down view of light positions over the mesh footprint
    render_light_map(ui, config, mesh_dims);

    ui.add_space(5.0);

    // Height slider (logarithmic scale)
    ui.horizontal(|ui| {
        ui.label("Height:");
    });
    let mut log_height = config.height.log10();
    if ui
        .add(
            egui::Slider::new(&mut log_height, 0.7..=2.7) // 5m to 500m
                .custom_formatter(|value, _| format!("{:.1} m", 10_f64.powf(value)))
                .suffix(""),
        )
        .changed()
    {
        config.height = 10_f32.powf(log_height);
    }

    ui.add_space(5.0);

    // Cone angle slider
    ui.horizontal(|ui| {
        ui.label("Cone Angle:");
    });
    ui.add(egui::Slider::new(&mut config.cone_angle, 5.0..=180.0).suffix("°"));

    ui.add_space(5.0);
    ui.separator();

    // Light properties section
    ui.label(egui::RichText::new("Light Properties").strong());

    ui.add_space(5.0);

    // Intensity slider (logarithmic)
    ui.horizontal(|ui| {
        ui.label("Intensity:");
    });
    let mut log_intensity = config.intensity.log10();
    if ui
        .add(
            egui::Slider::new(&mut log_intensity, 1.0..=7.0) // 10 to 10,000,000
                .custom_formatter(|value, _| format!("{:.0}", 10_f64.powf(value)))
                .suffix(""),
        )
        .changed()
    {
        config.intensity = 10_f32.powf(log_intensity);
    }

    ui.add_space(5.0);

    // Range slider
    ui.horizontal(|ui| {
        ui.label("Range:");
    });
    ui.add(egui::Slider::new(&mut config.range, 10.0..=5000.0).suffix(" m"));

    ui.add_space(5.0);

    // Color picker
    ui.horizontal(|ui| {
        ui.label("Color:");
    });

    // Convert Bevy Color to egui color
    let mut color_array = [
        config.color.to_srgba().red,
        config.color.to_srgba().green,
        config.color.to_srgba().blue,
    ];

    if ui.color_edit_button_rgb(&mut color_array).changed() {
        config.color = Color::srgb(color_array[0], color_array[1], color_array[2]);
    }

    ui.add_space(5.0);
    ui.separator();

    // Marker visualization section
    ui.label(egui::RichText::new("Light Markers").strong());

    ui.add_space(5.0);

    // Show markers toggle
    ui.horizontal(|ui| {
        ui.label("Show Markers:");
        ui.checkbox(&mut config.show_markers, "");
    });

    ui.add_space(5.0);

    // Marker size slider (only show if markers are enabled)
    if config.show_markers {
        ui.horizontal(|ui| {
            ui.label("Marker Size:");
        });
        ui.add(egui::Slider::new(&mut config.marker_size, 0.1..=5.0).suffix(" m"));
    }
}

/// Render an interactive 2D top-down light map showing the mesh footprint and light positions.
///
/// The view is anchored to the mesh footprint so the mesh outline stays stable
/// and the light dots visually move when coverage changes. The widget aspect
/// ratio matches the mesh X:Z ratio.
fn render_light_map(ui: &mut egui::Ui, config: &mut NightLightingConfig, mesh_dims: &MeshDimensions) {
    // --- Mesh footprint (always 100% bounds) ---
    let (mesh_min, mesh_max) = if let (Some(mn), Some(mx)) = (mesh_dims.min, mesh_dims.max) {
        (egui::Vec2::new(mn.x, mn.z), egui::Vec2::new(mx.x, mx.z))
    } else {
        (egui::Vec2::new(-100.0, -100.0), egui::Vec2::new(100.0, 100.0))
    };

    // --- Coverage bounds (where the lights actually are) ---
    let scale = config.coverage_pct / 100.0;
    let mesh_centre = (mesh_min + mesh_max) * 0.5;
    let mesh_half = (mesh_max - mesh_min) * 0.5;
    let cov_min = mesh_centre - mesh_half * scale;
    let cov_max = mesh_centre + mesh_half * scale;

    // --- Stable view bounds: union of mesh + coverage, plus 10% padding ---
    let view_min = egui::Vec2::new(mesh_min.x.min(cov_min.x), mesh_min.y.min(cov_min.y));
    let view_max = egui::Vec2::new(mesh_max.x.max(cov_max.x), mesh_max.y.max(cov_max.y));
    let view_centre = (view_min + view_max) * 0.5;
    let view_half = (view_max - view_min) * 0.5 * 1.1; // 10% padding
    let view_min = view_centre - view_half;
    let view_max = view_centre + view_half;
    let view_extent = view_max - view_min;

    // --- Widget size: match mesh X:Z aspect ratio, fit panel width ---
    let available_w = ui.available_width().min(280.0);
    let aspect = if view_extent.x.abs() > f32::EPSILON {
        view_extent.y / view_extent.x
    } else {
        1.0
    };
    // Clamp aspect so the widget doesn't get absurdly tall or short
    let aspect = aspect.clamp(0.3, 3.0);
    let map_size = egui::Vec2::new(available_w, available_w * aspect);

    let (response, painter) = ui.allocate_painter(map_size, egui::Sense::click_and_drag());
    let rect = response.rect;

    // Background
    painter.rect_filled(rect, 2.0, egui::Color32::from_gray(30));

    // --- coordinate mapping: world XZ → widget pixel ---
    let margin = 8.0;
    let draw_rect = rect.shrink(margin);

    let world_to_screen = |wx: f32, wz: f32| -> egui::Pos2 {
        let nx = if view_extent.x.abs() > f32::EPSILON {
            (wx - view_min.x) / view_extent.x
        } else {
            0.5
        };
        let nz = if view_extent.y.abs() > f32::EPSILON {
            (wz - view_min.y) / view_extent.y
        } else {
            0.5
        };
        egui::Pos2::new(
            draw_rect.left() + nx * draw_rect.width(),
            draw_rect.top() + nz * draw_rect.height(),
        )
    };

    // Draw mesh footprint rectangle (stable anchor)
    let mesh_tl = world_to_screen(mesh_min.x, mesh_min.y);
    let mesh_br = world_to_screen(mesh_max.x, mesh_max.y);
    painter.rect_stroke(
        egui::Rect::from_two_pos(mesh_tl, mesh_br),
        1.0,
        egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(100, 160, 255, 120)),
        egui::StrokeKind::Outside,
    );

    // Draw coverage bounds outline when different from mesh
    if (scale - 1.0).abs() > 0.01 {
        let cov_tl = world_to_screen(cov_min.x, cov_min.y);
        let cov_br = world_to_screen(cov_max.x, cov_max.y);
        painter.rect_stroke(
            egui::Rect::from_two_pos(cov_tl, cov_br),
            1.0,
            egui::Stroke::new(1.0, egui::Color32::from_rgba_unmultiplied(255, 200, 80, 80)),
            egui::StrokeKind::Outside,
        );
    }

    // Compute light positions via the placement algorithm
    let bounds_min_bevy = bevy::prelude::Vec2::new(cov_min.x, cov_min.y);
    let bounds_max_bevy = bevy::prelude::Vec2::new(cov_max.x, cov_max.y);
    let positions = config
        .placement_algorithm
        .calculate_positions(config.num_lights, bounds_min_bevy, bounds_max_bevy);

    // Draw each light as a filled circle
    let light_color = egui::Color32::from_rgb(
        (config.color.to_srgba().red * 255.0) as u8,
        (config.color.to_srgba().green * 255.0) as u8,
        (config.color.to_srgba().blue * 255.0) as u8,
    );
    let dot_radius = 4.0;

    for pos in &positions {
        let screen_pos = world_to_screen(pos.x, pos.y);
        painter.circle_filled(screen_pos, dot_radius, light_color);
        painter.circle_stroke(screen_pos, dot_radius, egui::Stroke::new(1.0, egui::Color32::WHITE));
    }

    // Label with mesh dimensions
    let mesh_ext = mesh_max - mesh_min;
    painter.text(
        rect.left_bottom() + egui::Vec2::new(2.0, -2.0),
        egui::Align2::LEFT_BOTTOM,
        format!("{:.0}×{:.0} m", mesh_ext.x, mesh_ext.y),
        egui::FontId::proportional(10.0),
        egui::Color32::from_gray(140),
    );
}

/// Render material property controls
fn render_material_controls(ui: &mut egui::Ui, config: &mut MaterialConfig) {
    // Base color picker
    ui.horizontal(|ui| {
        ui.label("Color:");
    });
    let mut color_array = [
        config.base_color.to_srgba().red,
        config.base_color.to_srgba().green,
        config.base_color.to_srgba().blue,
    ];
    if ui.color_edit_button_rgb(&mut color_array).changed() {
        config.base_color = Color::srgb(color_array[0], color_array[1], color_array[2]);
    }

    ui.add_space(5.0);

    // Roughness slider
    ui.horizontal(|ui| {
        ui.label("Roughness:");
    });
    ui.add(
        egui::Slider::new(&mut config.perceptual_roughness, 0.0..=1.0)
            .custom_formatter(|v, _| format!("{:.2}", v)),
    );

    ui.add_space(5.0);

    // Metallic slider
    ui.horizontal(|ui| {
        ui.label("Metallic:");
    });
    ui.add(
        egui::Slider::new(&mut config.metallic, 0.0..=1.0)
            .custom_formatter(|v, _| format!("{:.2}", v)),
    );

    ui.add_space(5.0);

    // Reflectance slider
    ui.horizontal(|ui| {
        ui.label("Reflectance:");
    });
    ui.add(
        egui::Slider::new(&mut config.reflectance, 0.0..=1.0)
            .custom_formatter(|v, _| format!("{:.2}", v)),
    );

    ui.add_space(5.0);
    ui.separator();

    // Emissive section
    ui.label(egui::RichText::new("Emissive").strong());
    ui.add_space(5.0);

    ui.horizontal(|ui| {
        ui.label("Color:");
    });
    let mut emissive_array = [
        config.emissive.to_srgba().red,
        config.emissive.to_srgba().green,
        config.emissive.to_srgba().blue,
    ];
    if ui.color_edit_button_rgb(&mut emissive_array).changed() {
        config.emissive = Color::srgb(emissive_array[0], emissive_array[1], emissive_array[2]);
    }

    ui.add_space(5.0);

    ui.horizontal(|ui| {
        ui.label("Intensity:");
    });
    ui.add(egui::Slider::new(&mut config.emissive_intensity, 0.0..=50.0));

    ui.add_space(5.0);
    ui.separator();

    // Rendering options
    ui.label(egui::RichText::new("Rendering").strong());
    ui.add_space(5.0);

    // Double-sided toggle
    ui.horizontal(|ui| {
        ui.label("Double Sided:");
        ui.checkbox(&mut config.double_sided, "");
    });

    ui.add_space(5.0);

    // Alpha mode
    ui.horizontal(|ui| {
        ui.label("Alpha Mode:");
    });
    egui::ComboBox::from_id_salt("alpha_mode")
        .selected_text(config.alpha_mode.name())
        .show_ui(ui, |ui| {
            for mode in AlphaModeConfig::all() {
                ui.selectable_value(&mut config.alpha_mode, *mode, mode.name());
            }
        });

    // Alpha cutoff (only for Mask mode)
    if config.alpha_mode == AlphaModeConfig::Mask {
        ui.add_space(5.0);
        ui.horizontal(|ui| {
            ui.label("Alpha Cutoff:");
        });
        ui.add(
            egui::Slider::new(&mut config.alpha_cutoff, 0.0..=1.0)
                .custom_formatter(|v, _| format!("{:.2}", v)),
        );
    }
}

/// System that shows the delete confirmation dialog
pub fn delete_confirmation_dialog_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    mut delete_events: MessageWriter<DeleteSessionEvent>,
) {
    if let Some(session_id) = ui_state.temp_state.show_delete_confirmation {
        let ctx = contexts.ctx_mut().unwrap();

        let mut show_dialog = true;

        egui::Window::new("Confirm Delete")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("Are you sure you want to delete this session?");
                ui.label("This action cannot be undone.");

                ui.add_space(20.0);

                ui.horizontal(|ui| {
                    if ui.button("Delete").clicked() {
                        delete_events.write(DeleteSessionEvent { session_id });
                        show_dialog = false;
                    }

                    if ui.button("Cancel").clicked() {
                        show_dialog = false;
                    }
                });
            });

        if !show_dialog {
            ui_state.temp_state.show_delete_confirmation = None;
        }
    }
}

/// Renders the "Rendering" collapsible section with frame window controls.
fn render_rendering_section(
    ui: &mut egui::Ui,
    ui_state: &mut UiState,
    render_config: &mut RenderConfig,
    sequence_assets: &SequenceAssets,
) {
    egui::CollapsingHeader::new("🖥 Rendering")
        .default_open(ui_state.collapsible.rendering_open)
        .show(ui, |ui| {
            ui_state.collapsible.rendering_open = true;

            // Frame window size slider
            ui.heading("Frame Window");
            ui.add_space(2.0);
            ui.label("Spawned entities around current frame:");
            ui.add_space(2.0);

            let max_window = sequence_assets.total_frames.max(1);
            ui.add(
                egui::Slider::new(&mut render_config.frame_window_size, 1..=max_window.min(100))
                    .text("frames")
                    .logarithmic(true),
            );

            ui.add_space(4.0);

            // Show current window stats
            let spawned = sequence_assets.frame_entities.iter().filter(|e| e.is_some()).count();
            let total = sequence_assets.total_frames;

            egui::Grid::new("render_window_grid")
                .num_columns(2)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Spawned:");
                    ui.label(format!("{} / {}", spawned, total));
                    ui.end_row();

                    ui.label("Displayed:");
                    ui.label(
                        sequence_assets
                            .displayed_frame
                            .map(|f| format!("Frame {}", f))
                            .unwrap_or_else(|| "—".to_string()),
                    );
                    ui.end_row();

                    if total > 0 {
                        let current = sequence_assets.displayed_frame.unwrap_or(0);
                        let (lo, hi) = sequence_assets.window_range(
                            current,
                            render_config.frame_window_size,
                        );
                        ui.label("Window:");
                        ui.label(format!("[{}, {})", lo, hi));
                        ui.end_row();
                    }
                });
        });
}
