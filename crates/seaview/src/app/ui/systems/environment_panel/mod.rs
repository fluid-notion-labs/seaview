//! Environment Panel — the primary left sidebar for Seaview
//!
//! This module implements the left side panel containing collapsible accordion
//! sections for environment lighting, spot lights, material, rendering,
//! performance controls, and environment objects.
//!
//! Each section is implemented in its own submodule and rendered as a plain
//! function taking `&mut egui::Ui` plus the resources it needs.

mod environment_light_section;
mod lighting_section;
mod material_section;
mod objects_section;
mod perf_section;
mod rendering_section;
mod series_section;
mod spatial_section;

pub use environment_light_section::render_environment_light_controls;
pub use lighting_section::{render_lighting_controls, GenerateGroupState};
pub use material_section::render_material_controls;
pub use objects_section::{render_objects_section, FileDialogReceiver};
pub use perf_section::render_perf_section;
pub use rendering_section::render_rendering_section;
pub use series_section::{render_series_section, DirDialogReceiver};
pub use spatial_section::render_spatial_section;

use std::path::PathBuf;

use bevy::diagnostic::DiagnosticsStore;
use bevy::ecs::system::SystemParam;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::app::environment_objects::{EnvironmentObjectStoreRes, ImportEnvironmentObjectEvent};
use crate::app::systems::bvh_gizmos::BvhGizmoConfig;
use crate::app::systems::diagnostics::RenderingStats;
use crate::app::ui::state::{MaterialConfig, RenderConfig, SsrConfig, UiState};
use crate::app::ui::widgets::{section_header, section_header_tracked};
use crate::lib::coordinates::SourceOrientation;
use crate::lib::lighting::{
    EnvironmentLightConfig, EnvironmentMapCatalog, EnvironmentMapConfig, LightingRig,
};
use crate::lib::mesh_info::MeshDimensions;
use crate::lib::sequence::loader::spatial_index::SpatialIndex;
use crate::lib::sequence::loader::LoaderConfig;
use crate::lib::sequence::FrameStore;
use crate::lib::sequence::loader::PendingLoadRequests;
use crate::lib::sequence::LoadSequenceRequest;
use crate::lib::sequence::SeriesManagerRes;

// ---------------------------------------------------------------------------
// Bundled system params (keeps us under Bevy's 16-parameter limit)
// ---------------------------------------------------------------------------

/// Lighting, material, rendering, and performance resources.
#[derive(SystemParam)]
pub struct EnvironmentPanelParams<'w, 's> {
    env_light_config: ResMut<'w, EnvironmentLightConfig>,
    lighting_rig: ResMut<'w, LightingRig>,
    gen_state: Local<'s, GenerateGroupState>,
    material_config: ResMut<'w, MaterialConfig>,
    render_config: ResMut<'w, RenderConfig>,
    ssr_config: ResMut<'w, SsrConfig>,
    env_map_config: ResMut<'w, EnvironmentMapConfig>,
    env_map_catalog: Res<'w, EnvironmentMapCatalog>,
    mesh_dims: Res<'w, MeshDimensions>,
    frame_store: Res<'w, FrameStore>,
    spatial_index: Option<Res<'w, SpatialIndex>>,
    bvh_config: Option<ResMut<'w, BvhGizmoConfig>>,
    diagnostics: Res<'w, DiagnosticsStore>,
    rendering_stats: Res<'w, RenderingStats>,
    source_orientation: ResMut<'w, SourceOrientation>,
    loader_config: ResMut<'w, LoaderConfig>,
}

/// Environment-objects resources (file dialog, store, import events).
#[derive(SystemParam)]
pub struct EnvObjectParams<'w, 's> {
    env_obj_store: ResMut<'w, EnvironmentObjectStoreRes>,
    import_events: MessageWriter<'w, ImportEnvironmentObjectEvent>,
    file_dialog_rx: Local<'s, Option<FileDialogReceiver>>,
    objects_open: Local<'s, Option<bool>>,
}

// ---------------------------------------------------------------------------
// Single unified panel system
// ---------------------------------------------------------------------------

/// System that renders the entire environment panel (left sidebar) in a single
/// `SidePanel` + `ScrollArea`, including environment objects.
///
/// Previous versions split environment objects into a second system that
/// re-entered the same panel ID.  That caused overlapping interaction rects
/// and stolen mouse input.  Now everything lives in one system, with
/// [`SystemParam`] bundles to stay within Bevy's tuple-size limits.
pub fn environment_panel_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    mut panel: EnvironmentPanelParams,
    mut env_objs: EnvObjectParams,
    mut series_mgr: ResMut<SeriesManagerRes>,
    mut dir_dialog_rx: Local<Option<DirDialogReceiver>>,
    mut pending_loads: ResMut<PendingLoadRequests>,
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
                    section_header(ui, "Environment Map", "env_map", false, |ui| {
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut panel.env_map_config.enabled, "Enabled");
                        });

                        if !panel.env_map_config.enabled {
                            ui.label(
                                egui::RichText::new("IBL reflections and skybox are off")
                                    .small()
                                    .color(egui::Color32::from_gray(120)),
                            );
                        } else {
                            // Map selector
                            let selected_name = panel
                                .env_map_config
                                .selected
                                .and_then(|i| panel.env_map_catalog.entries.get(i))
                                .map(|e| e.name.as_str())
                                .unwrap_or("None");

                            ui.horizontal(|ui| {
                                ui.label("Map");
                                egui::ComboBox::from_id_salt("env_map_select")
                                    .width(ui.available_width() - 4.0)
                                    .selected_text(selected_name)
                                    .show_ui(ui, |ui| {
                                        // "None" option
                                        let is_none = panel.env_map_config.selected.is_none();
                                        if ui.selectable_label(is_none, "None").clicked() {
                                            panel.env_map_config.selected = None;
                                        }
                                        ui.separator();
                                        for (idx, entry) in
                                            panel.env_map_catalog.entries.iter().enumerate()
                                        {
                                            let selected =
                                                panel.env_map_config.selected == Some(idx);
                                            if ui.selectable_label(selected, &entry.name).clicked()
                                                && !selected
                                            {
                                                panel.env_map_config.selected = Some(idx);
                                            }
                                        }
                                    });
                            });

                            ui.add_space(3.0);

                            // Intensity slider
                            ui.add(
                                egui::Slider::new(
                                    &mut panel.env_map_config.intensity,
                                    0.0..=50_000.0,
                                )
                                .text("IBL intensity")
                                .logarithmic(true),
                            );

                            // Skybox brightness slider
                            ui.add(
                                egui::Slider::new(
                                    &mut panel.env_map_config.skybox_brightness,
                                    0.0..=50_000.0,
                                )
                                .text("Sky brightness")
                                .logarithmic(true),
                            );
                        }
                    });

                    ui.add_space(5.0);

                    // ── Environment Light ──────────────────────────────
                    section_header_tracked(
                        ui,
                        "Environment Light",
                        "env_light",
                        &mut ui_state.collapsible.environment_light_open,
                        |ui| {
                            render_environment_light_controls(ui, &mut panel.env_light_config);
                        },
                    );

                    ui.add_space(5.0);

                    // ── Lighting Rig (v2) ─────────────────────────────
                    section_header_tracked(
                        ui,
                        "Lights",
                        "lights",
                        &mut ui_state.collapsible.lighting_rig_open,
                        |ui| {
                            render_lighting_controls(
                                ui,
                                &mut panel.lighting_rig,
                                &panel.mesh_dims,
                                &mut panel.gen_state,
                            );
                        },
                    );

                    ui.add_space(5.0);

                    // ── Material ───────────────────────────────────────
                    section_header_tracked(
                        ui,
                        "Material",
                        "material",
                        &mut ui_state.collapsible.material_open,
                        |ui| {
                            // Snapshot before UI — only mark changed if a
                            // widget actually modified a value.  Without
                            // this, merely expanding the section triggers
                            // DerefMut on the ResMut and re-applies the
                            // material every frame, which can cause meshlet
                            // meshes to go dark.
                            let before = panel.material_config.clone();
                            render_material_controls(
                                ui,
                                panel.material_config.bypass_change_detection(),
                            );
                            if *panel.material_config != before {
                                // Touch the ResMut so change detection fires.
                                panel.material_config.set_changed();
                            }
                        },
                    );

                    ui.add_space(5.0);

                    // ── Rendering ──────────────────────────────────────
                    render_rendering_section(
                        ui,
                        &mut ui_state,
                        &mut panel.render_config,
                        &panel.frame_store,
                        &mut panel.ssr_config,
                    );

                    ui.add_space(5.0);

                    // ── Source Coordinates ──────────────────────────────
                    section_header(ui, "Source Coordinates", "source_coords", false, |ui| {
                        ui.label(
                            egui::RichText::new(
                                "Coordinate system for mesh sequences. \
                                     Changes apply on next load.",
                            )
                            .small()
                            .color(egui::Color32::from_gray(120)),
                        );
                        ui.add_space(2.0);

                        let current = *panel.source_orientation;
                        let current_label = current.description();

                        egui::ComboBox::from_id_salt("source_orientation")
                            .width(ui.available_width() - 4.0)
                            .selected_text(current_label)
                            .show_ui(ui, |ui| {
                                for (value, label) in [
                                    (SourceOrientation::YUp, "Y-up (Bevy default)"),
                                    (SourceOrientation::ZUp, "Z-up (CAD / GIS)"),
                                    (SourceOrientation::FluidX3D, "FluidX3D"),
                                ] {
                                    let is_selected = current == value;
                                    if ui.selectable_label(is_selected, label).clicked()
                                        && !is_selected
                                    {
                                        *panel.source_orientation = value;
                                        panel.loader_config.coord_transform = value.to_transform();
                                    }
                                }
                            });
                    });

                    ui.add_space(5.0);

                    // ── Spatial (only when SpatialAcceleratorPlugin is loaded) ──
                    if let (Some(si), Some(mut bvh)) =
                        (panel.spatial_index.as_ref(), panel.bvh_config.as_mut())
                    {
                        render_spatial_section(
                            ui,
                            &mut ui_state,
                            &panel.frame_store,
                            &si,
                            &mut bvh,
                        );
                    }

                    ui.add_space(5.0);

                    // ── Performance ────────────────────────────────────
                    render_perf_section(
                        ui,
                        &mut ui_state,
                        &panel.diagnostics,
                        &panel.rendering_stats,
                    );

                    ui.add_space(5.0);

                    // ── Mesh Series ────────────────────────────────────
                    {
                        let summaries = ui_state.series.clone();
                        let orientation = *panel.source_orientation;
                        let series_actions = render_series_section(
                            ui,
                            &mut ui_state.collapsible.series_open,
                            &summaries,
                            &mut series_mgr,
                            &mut dir_dialog_rx,
                            orientation,
                        );

                        // Apply deferred actions
                        if let Some(id) = series_actions.switch_to {
                            series_mgr.set_active(id);
                        }
                        if let Some((id, new_name)) = series_actions.rename {
                            series_mgr.rename(id, new_name);
                        }
                        for (id, transform) in series_actions.transform_changes {
                            if let Some(state) = series_mgr.get_mut(id) {
                                state.transform = transform;
                            }
                        }
                        for (id, swizzle, preset) in series_actions.swizzle_changes {
                            if let Some(state) = series_mgr.get_mut(id) {
                                state.swizzle = swizzle;
                                state.coordinate_preset = preset;
                            }
                        }
                        if let Some(id) = series_actions.clone {
                            series_mgr.clone_series(id);
                        }
                        if let Some(id) = series_actions.remove {
                            series_mgr.remove(id);
                        }
                        if let Some((id, visible)) = series_actions.set_visible {
                            series_mgr.set_visible(id, visible);
                        }
                        for dir in series_actions.add_directories {
                            let added = series_section::add_series_from_directory(
                                &dir,
                                &mut series_mgr,
                                orientation,
                                usize::MAX,
                            );
                            for (series_id, frame_paths) in added {
                                pending_loads.queue.push(LoadSequenceRequest { series_id, frame_paths });
                            }
                        }
                    }

                    ui.add_space(5.0);

                    // ── Environment Objects ────────────────────────────
                    {
                        // Initialise the local open state from the collapsible
                        // state on first run.
                        if env_objs.objects_open.is_none() {
                            *env_objs.objects_open =
                                Some(ui_state.collapsible.environment_objects_open);
                        }
                        let open = env_objs.objects_open.as_mut().unwrap();

                        let mut pending_imports: Vec<PathBuf> = Vec::new();

                        render_objects_section(
                            ui,
                            &mut env_objs.env_obj_store,
                            open,
                            &mut pending_imports,
                            &mut env_objs.file_dialog_rx,
                        );

                        // Fire import messages for any paths the file dialog
                        // returned.
                        for path in pending_imports {
                            env_objs
                                .import_events
                                .write(ImportEnvironmentObjectEvent { path });
                        }
                    }
                });
        });
}

// ---------------------------------------------------------------------------
// Kept for backward compatibility — registered as a no-op system
// ---------------------------------------------------------------------------

/// Stub — previously rendered env objects in a separate panel call.
/// Now a no-op since everything is in [`environment_panel_system`].
pub fn environment_objects_panel_system(
    _contexts: EguiContexts,
    _ui_state: Res<UiState>,
    _env_obj_store: ResMut<EnvironmentObjectStoreRes>,
    _import_events: MessageWriter<ImportEnvironmentObjectEvent>,
    _file_dialog_rx: Local<Option<FileDialogReceiver>>,
    _objects_open: Local<Option<bool>>,
) {
    // No-op: environment objects are now rendered inside
    // environment_panel_system via the unified scroll area.
}

/// Stub — session deletion is no longer exposed in the UI.
/// Kept as a no-op to satisfy the system registration in UiSystemsPlugin.
pub fn delete_confirmation_dialog_system(
    mut contexts: EguiContexts,
    mut _ui_state: ResMut<UiState>,
) {
    let _ctx = contexts.ctx_mut();
}
