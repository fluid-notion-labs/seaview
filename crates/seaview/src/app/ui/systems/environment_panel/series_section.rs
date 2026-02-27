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

use std::path::PathBuf;
use std::sync::mpsc;

use bevy_egui::egui;

use crate::app::ui::widgets::{section_header_tracked, transform_editor};
use crate::lib::coordinates::SourceOrientation;
use crate::lib::environment_objects::Transform3D;
use crate::lib::sequence::discovery::{discover_sequences, SequencePatterns};
use crate::lib::sequence::SeriesManagerRes;
use crate::lib::series::{SeriesId, SeriesSummary};

// ---------------------------------------------------------------------------
// Async directory-dialog receiver (lives across frames)
// ---------------------------------------------------------------------------

/// Holds the receiving end of a one-shot channel from an `rfd` directory dialog.
///
/// Created when the user clicks "Add Series…" and consumed once a path (or
/// cancellation) arrives.
pub struct DirDialogReceiver {
    pub rx: mpsc::Receiver<Option<PathBuf>>,
}

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
    /// Directories to discover and add series from.
    pub add_directories: Vec<PathBuf>,
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
    dir_dialog_rx: &mut Option<DirDialogReceiver>,
    source_orientation: SourceOrientation,
) -> SeriesPanelActions {
    let mut actions = SeriesPanelActions::default();

    // Poll any in-flight directory dialog.
    poll_dir_dialog(dir_dialog_rx, &mut actions.add_directories);

    section_header_tracked(ui, "Mesh Series", "mesh_series", series_open, |ui| {
        // ── Add Series button ─────────────────────────────────
        ui.horizontal(|ui| {
            let dialog_in_flight = dir_dialog_rx.is_some();

            let button_text = if dialog_in_flight {
                "Waiting for folder…"
            } else {
                "\u{2795} Add Series…"
            };

            if ui
                .add_enabled(!dialog_in_flight, egui::Button::new(button_text))
                .on_hover_text("Add mesh series from a directory")
                .clicked()
            {
                open_dir_dialog(dir_dialog_rx, source_orientation);
            }
        });

        ui.add_space(4.0);

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

// ---------------------------------------------------------------------------
// Directory dialog helpers
// ---------------------------------------------------------------------------

/// Opens a native directory picker on a background thread.
fn open_dir_dialog(rx_slot: &mut Option<DirDialogReceiver>, _orientation: SourceOrientation) {
    let (tx, rx) = mpsc::channel();
    *rx_slot = Some(DirDialogReceiver { rx });

    std::thread::spawn(move || {
        let result = rfd::FileDialog::new()
            .set_title("Select Mesh Series Directory")
            .pick_folder();
        let _ = tx.send(result);
    });
}

/// Polls the in-flight directory dialog.  If a path arrived, appends it to
/// `pending_dirs` and clears the receiver.
fn poll_dir_dialog(rx_slot: &mut Option<DirDialogReceiver>, pending_dirs: &mut Vec<PathBuf>) {
    let Some(dialog) = rx_slot.as_ref() else {
        return;
    };

    match dialog.rx.try_recv() {
        Ok(Some(path)) => {
            pending_dirs.push(path);
            *rx_slot = None;
        }
        Ok(None) => {
            // User cancelled.
            *rx_slot = None;
        }
        Err(mpsc::TryRecvError::Empty) => {
            // Still waiting.
        }
        Err(mpsc::TryRecvError::Disconnected) => {
            *rx_slot = None;
        }
    }
}

/// Discover sequences in a directory and add them to the series manager.
///
/// Returns the number of new series added.
pub fn add_series_from_directory(
    dir: &std::path::Path,
    series_mgr: &mut SeriesManagerRes,
    source_orientation: SourceOrientation,
    max_frames: usize,
) -> usize {
    let patterns = SequencePatterns::default();
    let sequences = match discover_sequences(dir, true, &patterns, source_orientation) {
        Ok(seqs) => seqs,
        Err(e) => {
            bevy::prelude::warn!("Failed to discover sequences in {}: {}", dir.display(), e);
            return 0;
        }
    };

    if sequences.is_empty() {
        bevy::prelude::info!("No sequences found in {}", dir.display());
        return 0;
    }

    let mut added = 0;
    for seq in sequences {
        // Skip if a series with this base_dir already exists
        if series_mgr.find_by_dir(&seq.base_dir).is_some() {
            bevy::prelude::info!(
                "Skipping '{}' — already loaded ({})",
                seq.name,
                seq.base_dir.display()
            );
            continue;
        }

        let id = series_mgr.add(seq.clone(), max_frames);
        bevy::prelude::info!(
            "Added series '{}' as {} ({} frames) from {}",
            seq.name,
            id,
            seq.frame_count(),
            dir.display(),
        );
        added += 1;
    }

    added
}
