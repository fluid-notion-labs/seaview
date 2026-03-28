//! Mesh Series section for the Environment Panel
//!
//! Renders a collapsible accordion section listing all loaded mesh series.
//! Each series row shows:
//!
//! - Active indicator (click to set as timeline focus)
//! - Visibility checkbox (toggle rendering without changing active)
//! - Name (editable)
//! - Progress bar (loaded / total frames)
//! - Collapsible per-series transform controls

use std::path::PathBuf;
use std::sync::mpsc;

use bevy_egui::egui;

use crate::app::ui::widgets::{section_header_tracked, transform_editor};
use crate::lib::coordinates::SourceOrientation;
use crate::lib::environment_objects::{AxisSign, AxisSwizzle, CoordinatePreset, Transform3D};
use crate::lib::sequence::discovery::{discover_sequences, SequencePatterns};
use crate::lib::sequence::SeriesManagerRes;
use crate::lib::series::{SeriesId, SeriesSummary};

// ---------------------------------------------------------------------------
// Async directory-dialog receiver (lives across frames)
// ---------------------------------------------------------------------------

pub struct DirDialogReceiver {
    pub rx: mpsc::Receiver<Option<PathBuf>>,
}

// ---------------------------------------------------------------------------
// Deferred actions
// ---------------------------------------------------------------------------

#[derive(Default)]
pub struct SeriesPanelActions {
    /// Switch the active (timeline focus) series to this ID.
    pub switch_to: Option<SeriesId>,
    /// Rename a series: (id, new_name).
    pub rename: Option<(SeriesId, String)>,
    /// Per-series transforms that changed: (id, new_transform).
    pub transform_changes: Vec<(SeriesId, Transform3D)>,
    /// Per-series swizzle changes: (id, new_swizzle, new_coordinate_preset).
    pub swizzle_changes: Vec<(SeriesId, AxisSwizzle, CoordinatePreset)>,
    /// Clone a series (source ID).
    pub clone: Option<SeriesId>,
    /// Remove a series by ID.
    pub remove: Option<SeriesId>,
    /// Directories to discover and add series from.
    pub add_directories: Vec<PathBuf>,
    /// Visibility toggle: (id, new_visible).
    pub set_visible: Option<(SeriesId, bool)>,
}

// ---------------------------------------------------------------------------
// Public render function
// ---------------------------------------------------------------------------

pub fn render_series_section(
    ui: &mut egui::Ui,
    series_open: &mut bool,
    summaries: &[SeriesSummary],
    series_mgr: &mut SeriesManagerRes,
    dir_dialog_rx: &mut Option<DirDialogReceiver>,
    source_orientation: SourceOrientation,
) -> SeriesPanelActions {
    let mut actions = SeriesPanelActions::default();

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
        let visible_count = summaries.iter().filter(|s| s.is_visible).count();
        let active_label = summaries
            .iter()
            .find(|s| s.is_active)
            .map(|s| s.label.as_str())
            .unwrap_or("—");
        ui.label(
            egui::RichText::new(format!(
                "{total_series} series — {visible_count} visible — active: {active_label}"
            ))
            .small()
            .color(egui::Color32::from_gray(150)),
        );

        ui.add_space(4.0);

        // ── Per-series rows ───────────────────────────────────────
        for summary in summaries {
            let id = summary.id;
            let is_active = summary.is_active;
            let is_visible = summary.is_visible;

            let current_transform = series_mgr
                .get(id)
                .map(|s| s.transform.clone())
                .unwrap_or_default();

            let header_color = if is_active {
                egui::Color32::from_rgb(130, 200, 255)
            } else if is_visible {
                egui::Color32::from_gray(220)
            } else {
                egui::Color32::from_gray(140)
            };

            let active_prefix = if is_active { "▶ " } else { "   " };
            let header_text = format!("{active_prefix}{}", summary.label);

            let collapsing_id = ui.id().with(("series_row", id.raw()));

            egui::CollapsingHeader::new(egui::RichText::new(&header_text).color(header_color))
                .id_salt(collapsing_id)
                .default_open(is_active)
                .show(ui, |ui| {
                    // ── Visibility + Active row ───────────────────
                    ui.horizontal(|ui| {
                        // Visibility checkbox — toggles rendering without
                        // changing the active (timeline focus) series.
                        let mut vis = is_visible;
                        if ui
                            .checkbox(&mut vis, "Visible")
                            .on_hover_text("Show/hide this series in the viewport")
                            .changed()
                        {
                            actions.set_visible = Some((id, vis));
                        }

                        // Set active — only shown when not already active
                        if !is_active {
                            if ui
                                .button("⏵ Set Active")
                                .on_hover_text("Use this series for timeline / playback controls")
                                .clicked()
                            {
                                actions.switch_to = Some(id);
                            }
                        }
                    });

                    ui.add_space(2.0);

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

                    // ── Axis swizzle ──────────────────────────────
                    let (mut current_swizzle, mut current_preset) = series_mgr
                        .get(id)
                        .map(|s| (s.swizzle.clone(), s.coordinate_preset))
                        .unwrap_or_default();

                    let mut swizzle_changed = false;

                    egui::CollapsingHeader::new("Coordinates")
                        .id_salt(("series_coords", id.raw()))
                        .default_open(false)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.label("Preset:");
                                egui::ComboBox::from_id_salt(("series_coord_preset", id.raw()))
                                    .width(140.0)
                                    .selected_text(current_preset.label())
                                    .show_ui(ui, |ui| {
                                        for preset in CoordinatePreset::ALL {
                                            let is_selected = current_preset == *preset;
                                            if ui
                                                .selectable_label(is_selected, preset.label())
                                                .clicked()
                                                && !is_selected
                                            {
                                                current_preset = *preset;
                                                if let Some(swizzle) = preset.to_swizzle() {
                                                    current_swizzle = swizzle;
                                                }
                                                swizzle_changed = true;
                                            }
                                        }
                                    });
                            });

                            ui.horizontal(|ui| {
                                ui.label("Flip");
                                let mut flip_x =
                                    current_swizzle.x.sign == AxisSign::Negative;
                                let mut flip_y =
                                    current_swizzle.y.sign == AxisSign::Negative;
                                let mut flip_z =
                                    current_swizzle.z.sign == AxisSign::Negative;
                                if ui.checkbox(&mut flip_x, "X").changed() {
                                    current_swizzle.x.sign = if flip_x {
                                        AxisSign::Negative
                                    } else {
                                        AxisSign::Positive
                                    };
                                    current_preset = CoordinatePreset::Custom;
                                    swizzle_changed = true;
                                }
                                if ui.checkbox(&mut flip_y, "Y").changed() {
                                    current_swizzle.y.sign = if flip_y {
                                        AxisSign::Negative
                                    } else {
                                        AxisSign::Positive
                                    };
                                    current_preset = CoordinatePreset::Custom;
                                    swizzle_changed = true;
                                }
                                if ui.checkbox(&mut flip_z, "Z").changed() {
                                    current_swizzle.z.sign = if flip_z {
                                        AxisSign::Negative
                                    } else {
                                        AxisSign::Positive
                                    };
                                    current_preset = CoordinatePreset::Custom;
                                    swizzle_changed = true;
                                }
                            });
                        });

                    if swizzle_changed {
                        actions
                            .swizzle_changes
                            .push((id, current_swizzle, current_preset));
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
            *rx_slot = None;
        }
        Err(mpsc::TryRecvError::Empty) => {}
        Err(mpsc::TryRecvError::Disconnected) => {
            *rx_slot = None;
        }
    }
}

/// Returns a vec of `(SeriesId, frame_paths)` for every newly-added series so
/// the caller can fire [`LoadSequenceRequest`] for each one.
pub fn add_series_from_directory(
    dir: &std::path::Path,
    series_mgr: &mut SeriesManagerRes,
    source_orientation: SourceOrientation,
    max_frames: usize,
) -> Vec<(crate::lib::series::SeriesId, Vec<PathBuf>)> {
    let patterns = SequencePatterns::default();
    let sequences = match discover_sequences(dir, true, &patterns, source_orientation) {
        Ok(seqs) => seqs,
        Err(e) => {
            bevy::prelude::warn!("Failed to discover sequences in {}: {}", dir.display(), e);
            return Vec::new();
        }
    };

    if sequences.is_empty() {
        bevy::prelude::info!("No sequences found in {}", dir.display());
        return Vec::new();
    }

    let mut added = Vec::new();
    for seq in sequences {
        if series_mgr.find_by_dir(&seq.base_dir).is_some() {
            bevy::prelude::info!(
                "Skipping '{}' — already loaded ({})",
                seq.name,
                seq.base_dir.display()
            );
            continue;
        }

        let frame_paths: Vec<PathBuf> = seq.frames.iter().map(|f| f.path.clone()).collect();
        let id = series_mgr.add(seq.clone(), max_frames);
        bevy::prelude::info!(
            "Added series '{}' as {} ({} frames) from {}",
            seq.name,
            id,
            seq.frame_count(),
            dir.display(),
        );
        added.push((id, frame_paths));
    }

    added
}
