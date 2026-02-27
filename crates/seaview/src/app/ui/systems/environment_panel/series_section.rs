//! Mesh Series section for the Environment Panel
//!
//! Renders a collapsible accordion section listing all loaded mesh series.
//! Each series row shows:
//!
//! - Active indicator + click-to-switch
//! - Name (editable via double-click)
//! - Progress bar (loaded / total frames)
//! - Collapsible per-series transform controls (position, rotation, scale)
//!
//! Read path: series summaries come from `UiState.series` (mirrored each
//! frame by `sync_ui_from_sequence_manager`).
//!
//! Write path: switching, renaming, and transform edits go through
//! `SeriesManagerRes` directly.

use bevy_egui::egui;

use crate::app::ui::widgets::{section_header_tracked, transform_editor};
use crate::lib::environment_objects::Transform3D;
use crate::lib::sequence::SeriesManagerRes;
use crate::lib::series::{SeriesId, SeriesSummary};

// ---------------------------------------------------------------------------
// Deferred actions
// ---------------------------------------------------------------------------

/// Actions collected during rendering that the caller must apply after the
/// UI closure finishes (to avoid double-borrow issues).
#[derive(Default)]
pub struct SeriesPanelActions {
    /// Switch the active series to this ID.
    pub switch_to: Option<SeriesId>,
    /// Rename a series: (id, new_name).
    pub rename: Option<(SeriesId, String)>,
    /// Per-series transforms that changed: (id, new_transform).
    pub transform_changes: Vec<(SeriesId, Transform3D)>,
    /// Clone a series (source ID).
    pub clone: Option<SeriesId>,
    /// Remove a series by ID.
    pub remove: Option<SeriesId>,
}

// ---------------------------------------------------------------------------
// Public render function
// ---------------------------------------------------------------------------

/// Renders the "Mesh Series" accordion section.
///
/// Returns a [`SeriesPanelActions`] struct that the caller must apply to
/// `SeriesManagerRes` outside the egui closure.
pub fn render_series_section(
    ui: &mut egui::Ui,
    series_open: &mut bool,
    summaries: &[SeriesSummary],
    series_mgr: &mut SeriesManagerRes,
) -> SeriesPanelActions {
    let mut actions = SeriesPanelActions::default();

    section_header_tracked(ui, "Mesh Series", "mesh_series", series_open, |ui| {
        if summaries.is_empty() {
            ui.label(
                egui::RichText::new("No series loaded")
                    .small()
                    .color(egui::Color32::from_gray(120)),
            );
            return;
        }

        // Summary line
        let total_series = summaries.len();
        let active_label = summaries
            .iter()
            .find(|s| s.is_active)
            .map(|s| s.label.as_str())
            .unwrap_or("—");
        ui.label(
            egui::RichText::new(format!("{total_series} series — active: {active_label}"))
                .small()
                .color(egui::Color32::from_gray(150)),
        );

        ui.add_space(4.0);

        // ── Per-series rows ───────────────────────────────────────
        for summary in summaries {
            let id = summary.id;
            let is_active = summary.is_active;

            // Get the mutable state for transform editing.  We read the
            // current transform from the manager (authoritative) and write
            // back only if the user changed it.
            let current_transform = series_mgr
                .get(id)
                .map(|s| s.transform.clone())
                .unwrap_or_default();

            let header_color = if is_active {
                egui::Color32::from_rgb(130, 200, 255)
            } else {
                egui::Color32::from_gray(200)
            };

            let active_prefix = if is_active { "▶ " } else { "   " };
            let header_text = format!("{active_prefix}{}", summary.label);

            let collapsing_id = ui.id().with(("series_row", id.raw()));

            egui::CollapsingHeader::new(egui::RichText::new(&header_text).color(header_color))
                .id_salt(collapsing_id)
                .default_open(is_active)
                .show(ui, |ui| {
                    // ── Click to activate ─────────────────────────
                    if !is_active {
                        if ui
                            .button("⏵ Set Active")
                            .on_hover_text("Switch to this series")
                            .clicked()
                        {
                            actions.switch_to = Some(id);
                        }
                        ui.add_space(2.0);
                    }

                    // ── Rename ────────────────────────────────────
                    let rename_buf_id = ui.id().with(("series_rename", id.raw()));
                    let mut name_buf: String = ui
                        .data_mut(|d| d.get_temp::<String>(rename_buf_id))
                        .unwrap_or_else(|| summary.label.clone());

                    ui.horizontal(|ui| {
                        ui.label("Name:");
                        let response = ui.text_edit_singleline(&mut name_buf);

                        if response.lost_focus()
                            && ui.input(|i| i.key_pressed(egui::Key::Enter))
                            && !name_buf.trim().is_empty()
                            && name_buf.trim() != summary.label
                        {
                            actions.rename = Some((id, name_buf.trim().to_string()));
                            // Reset buffer to the new name
                            let trimmed = name_buf.trim().to_string();
                            ui.data_mut(|d| d.insert_temp(rename_buf_id, trimmed));
                        } else {
                            ui.data_mut(|d| d.insert_temp(rename_buf_id, name_buf));
                        }
                    });

                    ui.add_space(2.0);

                    // ── Progress bar ──────────────────────────────
                    let loaded = summary.loaded_frames;
                    let total = summary.total_frames;
                    let fraction = if total > 0 {
                        loaded as f32 / total as f32
                    } else {
                        0.0
                    };

                    ui.horizontal(|ui| {
                        ui.label("Frames:");
                        let bar = egui::ProgressBar::new(fraction)
                            .text(format!("{loaded} / {total}"))
                            .desired_width(ui.available_width() - 4.0);
                        ui.add(bar);
                    });

                    if summary.total_frames > 0 {
                        ui.label(
                            egui::RichText::new(format!(
                                "Playhead: {} / {}",
                                summary.playhead, summary.total_frames
                            ))
                            .small()
                            .color(egui::Color32::from_gray(140)),
                        );
                    }

                    ui.add_space(4.0);

                    // ── Per-series transform ──────────────────────
                    let mut t = current_transform.clone();
                    let transform_changed = transform_editor::transform_editor_collapsible(
                        ui,
                        ("series_transform", id.raw()),
                        &mut t,
                    );

                    if transform_changed {
                        actions.transform_changes.push((id, t));
                    }

                    ui.add_space(4.0);

                    // ── Clone / Remove buttons ────────────────────
                    ui.horizontal(|ui| {
                        if ui
                            .button("⧉ Clone")
                            .on_hover_text("Duplicate this series with independent transform")
                            .clicked()
                        {
                            actions.clone = Some(id);
                        }

                        if !is_active {
                            if ui
                                .button("🗑 Remove")
                                .on_hover_text("Unload this series")
                                .clicked()
                            {
                                actions.remove = Some(id);
                            }
                        } else {
                            ui.add_enabled_ui(false, |ui| {
                                ui.button("🗑 Remove")
                                    .on_hover_text("Cannot remove the active series");
                            });
                        }
                    });
                });

            ui.add_space(2.0);
        }
    });

    actions
}
