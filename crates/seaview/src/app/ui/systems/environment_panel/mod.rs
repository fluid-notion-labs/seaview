//! Environment Panel — the primary left sidebar for Seaview
//!
//! This module implements the left side panel containing collapsible accordion
//! sections for environment lighting, spot lights, material, rendering, and
//! performance controls.
//!
//! Each section is implemented in its own submodule and rendered as a plain
//! function taking `&mut egui::Ui` plus the resources it needs.

mod environment_light_section;
mod lighting_section;
mod material_section;
mod perf_section;
mod rendering_section;
mod spatial_section;

pub use environment_light_section::render_environment_light_controls;
pub use lighting_section::{render_lighting_controls, GenerateGroupState};
pub use material_section::render_material_controls;
pub use perf_section::render_perf_section;
pub use rendering_section::render_rendering_section;
pub use spatial_section::render_spatial_section;

use bevy::diagnostic::DiagnosticsStore;
use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::app::systems::bvh_gizmos::BvhGizmoConfig;
use crate::app::systems::diagnostics::RenderingStats;
use crate::app::ui::state::{MaterialConfig, RenderConfig, UiState};
use crate::lib::lighting::{EnvironmentLightConfig, LightingRig};
use crate::lib::mesh_info::MeshDimensions;
use crate::lib::sequence::loader::spatial_index::SpatialIndex;
use crate::lib::sequence::FrameStore;

/// System that renders the environment panel (left sidebar).
///
/// Contains collapsible sections for Environment Light, Spot Lights, Material,
/// Rendering, and Performance.
pub fn environment_panel_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    mut env_light_config: ResMut<EnvironmentLightConfig>,
    mut lighting_rig: ResMut<LightingRig>,
    mut gen_state: Local<GenerateGroupState>,
    mut material_config: ResMut<MaterialConfig>,
    mut render_config: ResMut<RenderConfig>,
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
                .auto_shrink([false, false])
                .show(ui, |ui| {
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

/// Stub — session deletion is no longer exposed in the UI.
/// Kept as a no-op to satisfy the system registration in UiSystemsPlugin.
pub fn delete_confirmation_dialog_system(
    mut contexts: EguiContexts,
    mut _ui_state: ResMut<UiState>,
) {
    let _ctx = contexts.ctx_mut();
}
