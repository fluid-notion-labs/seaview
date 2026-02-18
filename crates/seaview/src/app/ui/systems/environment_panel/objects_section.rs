//! Environment Objects section for the Environment Panel
//!
//! Renders the UI for importing, listing, and controlling environment objects
//! (static GLB/glTF scene props).  Each object gets a collapsible row with:
//!
//! - Name (editable)
//! - Visibility toggle
//! - Delete button
//! - Transform controls (position, rotation, scale)
//! - Texture toggle
//! - Per-mesh visibility (once the scene's meshes have been discovered)
//!
//! The "Add Object" button opens a native file dialog via `rfd`.  The async
//! result is polled each frame and converted into an
//! [`ImportEnvironmentObjectEvent`] message.

use std::path::PathBuf;
use std::sync::mpsc;

use bevy_egui::egui;

use crate::app::environment_objects::EnvironmentObjectStoreRes;
use crate::lib::environment_objects::{
    AxisMapping, AxisSign, AxisSwizzle, EnvironmentObjectId, SourceAxis,
};

// ---------------------------------------------------------------------------
// Async file-dialog receiver (lives across frames)
// ---------------------------------------------------------------------------

/// Holds the receiving end of a one-shot channel from an `rfd` file dialog.
///
/// Created when the user clicks "Add Object…" and consumed once a path (or
/// cancellation) arrives.
pub struct FileDialogReceiver {
    pub rx: mpsc::Receiver<Option<PathBuf>>,
}

// ---------------------------------------------------------------------------
// Public render function
// ---------------------------------------------------------------------------

/// Renders the Environment Objects accordion section.
///
/// `pending_imports` collects file paths that should be turned into
/// [`ImportEnvironmentObjectEvent`] messages by the caller (the panel system).
/// This avoids requiring a `MessageWriter` parameter inside a plain UI
/// function.
///
/// `file_dialog_rx` is `Option<&mut FileDialogReceiver>` — the caller owns the
/// `Local<Option<FileDialogReceiver>>` and passes it in for polling.
pub fn render_objects_section(
    ui: &mut egui::Ui,
    store: &mut EnvironmentObjectStoreRes,
    objects_open: &mut bool,
    pending_imports: &mut Vec<PathBuf>,
    file_dialog_rx: &mut Option<FileDialogReceiver>,
) {
    // Poll any in-flight file dialog.
    poll_file_dialog(file_dialog_rx, pending_imports);

    egui::CollapsingHeader::new("\u{1f4e6} Environment Objects")
        .default_open(*objects_open)
        .show(ui, |ui| {
            *objects_open = true;

            // ── Add Object button ──────────────────────────────────
            ui.horizontal(|ui| {
                let dialog_in_flight = file_dialog_rx.is_some();

                let button_text = if dialog_in_flight {
                    "Waiting for file…"
                } else {
                    "\u{2795} Add Object…"
                };

                if ui
                    .add_enabled(!dialog_in_flight, egui::Button::new(button_text))
                    .on_hover_text("Import a .glb / .gltf file as a static scene prop")
                    .clicked()
                {
                    open_file_dialog(file_dialog_rx);
                }

                if store.len() > 0 {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            egui::RichText::new(format!("{} object(s)", store.len()))
                                .small()
                                .color(egui::Color32::from_gray(140)),
                        );
                    });
                }
            });

            if store.is_empty() {
                ui.add_space(4.0);
                ui.label(
                    egui::RichText::new("No environment objects imported yet.")
                        .small()
                        .color(egui::Color32::from_gray(120)),
                );
                return;
            }

            ui.add_space(4.0);

            // ── Object list ────────────────────────────────────────
            // Collect IDs first to avoid borrow issues.
            let ids: Vec<EnvironmentObjectId> = store.ids();
            let mut to_remove: Option<EnvironmentObjectId> = None;
            let mut mutated = false;

            for id in &ids {
                let Some(obj) = store.get_mut(*id) else {
                    continue;
                };

                let header_id = egui::Id::new(id.0);
                let header_label = format!(
                    "{} {}",
                    if obj.visible {
                        "\u{1f441}"
                    } else {
                        "\u{1f648}"
                    },
                    obj.name,
                );

                egui::CollapsingHeader::new(header_label)
                    .id_salt(header_id)
                    .default_open(false)
                    .show(ui, |ui| {
                        // ── Name ───────────────────────────────────
                        ui.horizontal(|ui| {
                            ui.label("Name");
                            if ui.text_edit_singleline(&mut obj.name).changed() {
                                mutated = true;
                            }
                        });

                        // ── Visibility + Textures + Delete ─────────
                        ui.horizontal(|ui| {
                            if ui.checkbox(&mut obj.visible, "Visible").changed() {
                                mutated = true;
                            }
                            if ui
                                .checkbox(&mut obj.textures_enabled, "Textures")
                                .on_hover_text("Toggle PBR textures (off = flat material)")
                                .changed()
                            {
                                mutated = true;
                            }
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    if ui
                                        .button("\u{1f5d1}")
                                        .on_hover_text("Remove this object")
                                        .clicked()
                                    {
                                        to_remove = Some(*id);
                                    }
                                },
                            );
                        });

                        ui.add_space(2.0);

                        // ── Path (read-only) ───────────────────────
                        ui.horizontal(|ui| {
                            ui.label(
                                egui::RichText::new(format!("{}", obj.path.display()))
                                    .small()
                                    .color(egui::Color32::from_gray(130)),
                            );
                        });

                        ui.add_space(4.0);

                        // ── Transform ──────────────────────────────
                        egui::CollapsingHeader::new("Transform")
                            .id_salt(header_id.with("transform"))
                            .default_open(false)
                            .show(ui, |ui| {
                                mutated |= render_transform_controls(ui, &mut obj.transform);
                            });

                        // ── Axis Remap ─────────────────────────────
                        egui::CollapsingHeader::new("Axis Remap")
                            .id_salt(header_id.with("swizzle"))
                            .default_open(false)
                            .show(ui, |ui| {
                                mutated |= render_swizzle_controls(ui, &mut obj.swizzle);
                            });

                        // ── Per-mesh visibility ────────────────────
                        if !obj.meshes.is_empty() {
                            ui.add_space(2.0);
                            egui::CollapsingHeader::new(format!("Meshes ({})", obj.meshes.len()))
                                .id_salt(header_id.with("meshes"))
                                .default_open(false)
                                .show(ui, |ui| {
                                    // Show all / hide all buttons
                                    ui.horizontal(|ui| {
                                        if ui.small_button("Show all").clicked() {
                                            obj.set_all_meshes_visible(true);
                                            mutated = true;
                                        }
                                        if ui.small_button("Hide all").clicked() {
                                            obj.set_all_meshes_visible(false);
                                            mutated = true;
                                        }
                                    });
                                    ui.add_space(2.0);

                                    for mesh in &mut obj.meshes {
                                        if ui.checkbox(&mut mesh.visible, &mesh.name).changed() {
                                            mutated = true;
                                        }
                                    }
                                });
                        } else {
                            ui.label(
                                egui::RichText::new("(meshes not yet discovered)")
                                    .small()
                                    .italics()
                                    .color(egui::Color32::from_gray(110)),
                            );
                        }
                    });

                ui.add_space(2.0);
            }

            // Apply deferred mutations.
            if let Some(id) = to_remove {
                store.remove(id);
            } else if mutated {
                store.bump_generation();
            }
        });
}

// ---------------------------------------------------------------------------
// Transform controls
// ---------------------------------------------------------------------------

/// Renders the 3×6 axis remap button grid.  Returns `true` if any mapping
/// changed.
///
/// ```text
/// X →  [-x] [+x] [-y] [+y] [-z] [+z]
/// Y →  [-x] [+x] [-y] [+y] [-z] [+z]
/// Z →  [-x] [+x] [-y] [+y] [-z] [+z]
/// ```
fn render_swizzle_controls(ui: &mut egui::Ui, swizzle: &mut AxisSwizzle) -> bool {
    let mut changed = false;

    const OPTIONS: [(AxisSign, SourceAxis); 6] = [
        (AxisSign::Negative, SourceAxis::X),
        (AxisSign::Positive, SourceAxis::X),
        (AxisSign::Negative, SourceAxis::Y),
        (AxisSign::Positive, SourceAxis::Y),
        (AxisSign::Negative, SourceAxis::Z),
        (AxisSign::Positive, SourceAxis::Z),
    ];

    let rows: [(&str, &mut AxisMapping); 3] = [
        ("X →", &mut swizzle.x),
        ("Y →", &mut swizzle.y),
        ("Z →", &mut swizzle.z),
    ];

    for (label, mapping) in rows {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(label).monospace().strong());
            for (sign, source) in &OPTIONS {
                let is_selected = mapping.sign == *sign && mapping.source == *source;
                let btn_label = format!("{}{}", sign, source);
                if ui
                    .selectable_label(is_selected, egui::RichText::new(btn_label).monospace())
                    .clicked()
                    && !is_selected
                {
                    mapping.sign = *sign;
                    mapping.source = *source;
                    changed = true;
                }
            }
        });
    }

    if !swizzle.is_identity() {
        ui.add_space(2.0);
        if ui.small_button("Reset to identity").clicked() {
            *swizzle = AxisSwizzle::default();
            changed = true;
        }
    }

    changed
}

// ---------------------------------------------------------------------------
// Transform controls
// ---------------------------------------------------------------------------

/// Renders position / rotation / scale sliders.  Returns `true` if any value
/// changed.
fn render_transform_controls(
    ui: &mut egui::Ui,
    transform: &mut crate::lib::environment_objects::Transform3D,
) -> bool {
    let mut changed = false;

    ui.label("Position");
    changed |= drag_vec3(ui, &mut transform.position, "pos", 0.1);

    ui.add_space(2.0);
    ui.label("Rotation (degrees)");
    changed |= drag_vec3(ui, &mut transform.rotation_degrees, "rot", 1.0);

    ui.add_space(2.0);
    ui.label("Scale");
    changed |= drag_vec3(ui, &mut transform.scale, "scale", 0.01);

    changed
}

/// Three-component drag-value row labelled X / Y / Z.
fn drag_vec3(ui: &mut egui::Ui, values: &mut [f32; 3], _id_prefix: &str, speed: f32) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        for (label, value) in ["X: ", "Y: ", "Z: "].iter().zip(values.iter_mut()) {
            changed |= ui
                .add(egui::DragValue::new(value).speed(speed).prefix(*label))
                .changed();
        }
    });
    changed
}

// ---------------------------------------------------------------------------
// File dialog helpers
// ---------------------------------------------------------------------------

/// Spawns a native file dialog on a background thread.
///
/// The result (path or `None` on cancel) is sent through a one-shot channel
/// that [`poll_file_dialog`] checks each frame.
fn open_file_dialog(rx_slot: &mut Option<FileDialogReceiver>) {
    let (tx, rx) = mpsc::channel();
    *rx_slot = Some(FileDialogReceiver { rx });

    std::thread::spawn(move || {
        let result = rfd::FileDialog::new()
            .set_title("Import Environment Object")
            .add_filter("glTF / GLB", &["glb", "gltf"])
            .add_filter("All files", &["*"])
            .pick_file();
        // Ignore send errors (receiver dropped = UI moved on).
        let _ = tx.send(result);
    });
}

/// Checks the in-flight file dialog channel.  If a path arrived, appends it
/// to `pending_imports` and clears the receiver.
fn poll_file_dialog(rx_slot: &mut Option<FileDialogReceiver>, pending_imports: &mut Vec<PathBuf>) {
    let Some(dialog) = rx_slot.as_ref() else {
        return;
    };

    match dialog.rx.try_recv() {
        Ok(Some(path)) => {
            pending_imports.push(path);
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
            // Sender dropped without sending (shouldn't happen, but clean up).
            *rx_slot = None;
        }
    }
}
