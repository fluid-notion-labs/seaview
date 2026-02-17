//! Environment Panel — the primary left sidebar for Seaview
//!
//! This module implements the left side panel containing collapsible accordion
//! sections for environment lighting, spot lights, material, rendering, and
//! performance controls.
//!
//! Each section is implemented in its own submodule and rendered as a plain
//! function taking `&mut egui::Ui` plus the resources it needs.
//!
//! The **Environment Objects** section is rendered by a separate Bevy system
//! (`environment_objects_panel_system`) to stay within Bevy's 16-parameter
//! limit.  It writes into a shared `egui::SidePanel` using the same panel ID,
//! which egui merges into a single visual panel.

mod environment_light_section;
mod lighting_section;
mod material_section;
mod objects_section;
mod perf_section;
mod rendering_section;
mod spatial_section;

pub use environment_light_section::render_environment_light_controls;
pub use lighting_section::{render_lighting_controls, GenerateGroupState};
pub use material_section::render_material_controls;
pub use objects_section::{render_objects_section, FileDialogReceiver};
pub use perf_section::render_perf_section;
pub use rendering_section::render_rendering_section;
pub use spatial_section::render_spatial_section;

use std::path::PathBuf;

use bevy::diagnostic::DiagnosticsStore;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::app::environment_objects::{EnvironmentObjectStoreRes, ImportEnvironmentObjectEvent};
use crate::app::systems::bvh_gizmos::BvhGizmoConfig;
use crate::app::systems::diagnostics::RenderingStats;
use crate::app::ui::state::{MaterialConfig, RenderConfig, UiState};
use crate::lib::lighting::{
    EnvironmentLightConfig, EnvironmentMapCatalog, EnvironmentMapConfig, LightingRig,
};
use crate::lib::mesh_info::MeshDimensions;
use crate::lib::sequence::loader::spatial_index::SpatialIndex;
use crate::lib::sequence::FrameStore;

// ---------------------------------------------------------------------------
// Panel system (everything except environment objects)
// ---------------------------------------------------------------------------

/// System that renders the environment panel (left sidebar).
///
/// Contains collapsible sections for Environment Light, Spot Lights, Material,
/// Rendering, and Performance.
///
/// **Environment Objects** are rendered by [`environment_objects_panel_system`]
/// which runs after this system and appends into the same egui panel.
pub fn environment_panel_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    mut env_light_config: ResMut<EnvironmentLightConfig>,
    mut lighting_rig: ResMut<LightingRig>,
    mut gen_state: Local<GenerateGroupState>,
    mut material_config: ResMut<MaterialConfig>,
    mut render_config: ResMut<RenderConfig>,
    mut env_map_config: ResMut<EnvironmentMapConfig>,
    env_map_catalog: Res<EnvironmentMapCatalog>,
    mesh_dims: Res<MeshDimensions>,
    frame_store: Res<FrameStore>,
    spatial_index: Option<Res<SpatialIndex>>,
    mut bvh_config: Option<ResMut<BvhGizmoConfig>>,
    diagnostics: Res<DiagnosticsStore>,
    rendering_stats: Res<RenderingStats>,
) {
    if !ui_state.show_environment_panel {
        return;
    }

    let ctx = contexts.ctx_mut().unwrap();

    let panel_width = ui_state.panel_sizes.environment_panel_width;

    egui::SidePanel::left("environment_panel")
        .resizable(true)
        .default_width(panel_width)
        .width_range(200.0..=500.0)
        .show(ctx, |ui| {
            // Update panel width if resized
            ui_state.panel_sizes.environment_panel_width = ui.available_width();

            // Main scroll area for all accordion sections
            egui::ScrollArea::vertical()
                .id_salt("env_panel_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    // ── Environment Map (IBL + Skybox) ─────────────────
                    egui::CollapsingHeader::new("Environment Map")
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.checkbox(&mut env_map_config.enabled, "Enabled");
                            });

                            if !env_map_config.enabled {
                                ui.label(
                                    egui::RichText::new("IBL reflections and skybox are off")
                                        .small()
                                        .color(egui::Color32::from_gray(120)),
                                );
                            } else {
                                // Map selector
                                let selected_name = env_map_config
                                    .selected
                                    .and_then(|i| env_map_catalog.entries.get(i))
                                    .map(|e| e.name.as_str())
                                    .unwrap_or("None");

                                ui.horizontal(|ui| {
                                    ui.label("Map");
                                    egui::ComboBox::from_id_salt("env_map_select")
                                        .width(ui.available_width() - 4.0)
                                        .selected_text(selected_name)
                                        .show_ui(ui, |ui| {
                                            // "None" option
                                            let is_none = env_map_config.selected.is_none();
                                            if ui.selectable_label(is_none, "None").clicked() {
                                                env_map_config.selected = None;
                                            }
                                            ui.separator();
                                            for (idx, entry) in
                                                env_map_catalog.entries.iter().enumerate()
                                            {
                                                let selected = env_map_config.selected == Some(idx);
                                                if ui
                                                    .selectable_label(selected, &entry.name)
                                                    .clicked()
                                                    && !selected
                                                {
                                                    env_map_config.selected = Some(idx);
                                                }
                                            }
                                        });
                                });

                                ui.add_space(3.0);

                                // Intensity slider
                                ui.add(
                                    egui::Slider::new(&mut env_map_config.intensity, 0.0..=5000.0)
                                        .text("IBL intensity")
                                        .logarithmic(true),
                                );

                                // Skybox brightness slider
                                ui.add(
                                    egui::Slider::new(
                                        &mut env_map_config.skybox_brightness,
                                        0.0..=5000.0,
                                    )
                                    .text("Sky brightness")
                                    .logarithmic(true),
                                );
                            }
                        });

                    ui.add_space(5.0);

                    // ── Environment Light ──────────────────────────────
                    egui::CollapsingHeader::new("Environment Light")
                        .default_open(ui_state.collapsible.environment_light_open)
                        .show(ui, |ui| {
                            ui_state.collapsible.environment_light_open = true;
                            render_environment_light_controls(ui, &mut env_light_config);
                        });

                    ui.add_space(5.0);

                    // ── Lighting Rig (v2) ─────────────────────────────
                    egui::CollapsingHeader::new("Lights")
                        .default_open(ui_state.collapsible.lighting_rig_open)
                        .show(ui, |ui| {
                            ui_state.collapsible.lighting_rig_open = true;
                            render_lighting_controls(
                                ui,
                                &mut lighting_rig,
                                &mesh_dims,
                                &mut gen_state,
                            );
                        });

                    ui.add_space(5.0);

                    // ── Material ───────────────────────────────────────
                    egui::CollapsingHeader::new("Material")
                        .default_open(ui_state.collapsible.material_open)
                        .show(ui, |ui| {
                            ui_state.collapsible.material_open = true;
                            render_material_controls(ui, &mut material_config);
                        });

                    ui.add_space(5.0);

                    // ── Rendering ──────────────────────────────────────
                    render_rendering_section(ui, &mut ui_state, &mut render_config, &frame_store);

                    ui.add_space(5.0);

                    // ── Spatial (only when SpatialAcceleratorPlugin is loaded) ──
                    if let (Some(si), Some(mut bvh)) = (spatial_index.as_ref(), bvh_config.as_mut())
                    {
                        render_spatial_section(ui, &mut ui_state, &frame_store, &si, &mut bvh);
                    }

                    ui.add_space(5.0);

                    // ── Performance ────────────────────────────────────
                    render_perf_section(ui, &mut ui_state, &diagnostics, &rendering_stats);
                });
        });
}

// ---------------------------------------------------------------------------
// Environment objects panel system (separate to stay under param limit)
// ---------------------------------------------------------------------------

/// System that renders the Environment Objects section inside the environment
/// panel.
///
/// Runs alongside [`environment_panel_system`].  We re-enter the same
/// `SidePanel` ID; egui appends our content to the existing panel area.
///
/// Pending file-dialog paths are converted to [`ImportEnvironmentObjectEvent`]
/// messages so the app-layer plugin can handle the actual spawning.
pub fn environment_objects_panel_system(
    mut contexts: EguiContexts,
    ui_state: Res<UiState>,
    mut env_obj_store: ResMut<EnvironmentObjectStoreRes>,
    mut import_events: MessageWriter<ImportEnvironmentObjectEvent>,
    mut file_dialog_rx: Local<Option<FileDialogReceiver>>,
    mut objects_open: Local<Option<bool>>,
) {
    if !ui_state.show_environment_panel {
        return;
    }

    // Initialise the local open state from the collapsible state on first run.
    if objects_open.is_none() {
        *objects_open = Some(ui_state.collapsible.environment_objects_open);
    }
    let open = objects_open.as_mut().unwrap();

    let ctx = contexts.ctx_mut().unwrap();

    // Re-enter the same side panel.  egui merges content into the panel that
    // was already created by `environment_panel_system`.
    egui::SidePanel::left("environment_panel")
        .resizable(true)
        .show(ctx, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("env_panel_scroll")
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    ui.add_space(5.0);

                    // ── Environment Objects ────────────────────────────
                    let mut pending_imports: Vec<PathBuf> = Vec::new();

                    render_objects_section(
                        ui,
                        &mut env_obj_store,
                        open,
                        &mut pending_imports,
                        &mut file_dialog_rx,
                    );

                    // Fire import messages for any paths the file dialog returned.
                    for path in pending_imports {
                        import_events.write(ImportEnvironmentObjectEvent { path });
                    }
                });
        });
}

/// Stub — session deletion is no longer exposed in the UI.
/// Kept as a no-op to satisfy the system registration in UiSystemsPlugin.
pub fn delete_confirmation_dialog_system(
    mut contexts: EguiContexts,
    mut _ui_state: ResMut<UiState>,
) {
    let _ctx = contexts.ctx_mut();
}
