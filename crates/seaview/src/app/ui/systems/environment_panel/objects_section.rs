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
    AxisSign, CustomMaterial, EnvironmentObjectId, MaterialOverride, MaterialPreset,
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

                        // ── Normals + Double Sided ─────────────────
                        ui.horizontal(|ui| {
                            if ui
                                .checkbox(&mut obj.flip_normals, "Flip Normals")
                                .on_hover_text("Invert face normals (fixes inside-out meshes)")
                                .changed()
                            {
                                mutated = true;
                            }
                            if ui
                                .checkbox(&mut obj.double_sided, "Double Sided")
                                .on_hover_text(
                                    "Render both sides of faces (disables backface culling)",
                                )
                                .changed()
                            {
                                mutated = true;
                            }
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

                        // ── Flip Axes ──────────────────────────────
                        ui.horizontal(|ui| {
                            ui.label("Flip");
                            let mut flip_x = obj.swizzle.x.sign == AxisSign::Negative;
                            let mut flip_y = obj.swizzle.y.sign == AxisSign::Negative;
                            let mut flip_z = obj.swizzle.z.sign == AxisSign::Negative;
                            if ui.checkbox(&mut flip_x, "X").changed() {
                                obj.swizzle.x.sign = if flip_x {
                                    AxisSign::Negative
                                } else {
                                    AxisSign::Positive
                                };
                                mutated = true;
                            }
                            if ui.checkbox(&mut flip_y, "Y").changed() {
                                obj.swizzle.y.sign = if flip_y {
                                    AxisSign::Negative
                                } else {
                                    AxisSign::Positive
                                };
                                mutated = true;
                            }
                            if ui.checkbox(&mut flip_z, "Z").changed() {
                                obj.swizzle.z.sign = if flip_z {
                                    AxisSign::Negative
                                } else {
                                    AxisSign::Positive
                                };
                                mutated = true;
                            }
                        });

                        // ── Material Override ──────────────────────
                        egui::CollapsingHeader::new("Material")
                            .id_salt(header_id.with("material"))
                            .default_open(false)
                            .show(ui, |ui| {
                                mutated |= render_material_controls(
                                    ui,
                                    &mut obj.material_override,
                                    header_id,
                                );
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
// Material override controls
// ---------------------------------------------------------------------------

/// Renders material override UI: mode selector + color picker + PBR sliders.
/// Returns `true` if any value changed.
fn render_material_controls(
    ui: &mut egui::Ui,
    material: &mut MaterialOverride,
    header_id: egui::Id,
) -> bool {
    let mut changed = false;

    // Current mode label for the dropdown
    let mode_label = match material {
        MaterialOverride::Default => "Default (GLB)",
        MaterialOverride::Preset(p) => p.label(),
        MaterialOverride::Custom(_) => "Custom",
    };

    // Mode selector dropdown
    egui::ComboBox::from_id_salt(header_id.with("mat_mode"))
        .width(ui.available_width() - 4.0)
        .selected_text(mode_label)
        .show_ui(ui, |ui| {
            // Default option
            if ui
                .selectable_label(material.is_default(), "Default (GLB)")
                .clicked()
                && !material.is_default()
            {
                *material = MaterialOverride::Default;
                changed = true;
            }

            ui.separator();

            // Presets
            for preset in MaterialPreset::ALL {
                let is_selected = matches!(material, MaterialOverride::Preset(p) if p == preset);
                if ui.selectable_label(is_selected, preset.label()).clicked() && !is_selected {
                    *material = MaterialOverride::Preset(*preset);
                    changed = true;
                }
            }

            ui.separator();

            // Custom option
            let is_custom = matches!(material, MaterialOverride::Custom(_));
            if ui.selectable_label(is_custom, "Custom").clicked() && !is_custom {
                // Seed custom from current resolved values or defaults
                let custom = if let Some((color, roughness, metallic)) = material.resolve() {
                    CustomMaterial {
                        base_color: color,
                        roughness,
                        metallic,
                    }
                } else {
                    CustomMaterial::default()
                };
                *material = MaterialOverride::Custom(custom);
                changed = true;
            }
        });

    // Show color picker + sliders for Custom mode
    if let MaterialOverride::Custom(ref mut custom) = material {
        ui.add_space(4.0);

        // Color picker — egui wants [f32; 3] or Color32
        let mut color3 = [
            custom.base_color[0],
            custom.base_color[1],
            custom.base_color[2],
        ];
        ui.horizontal(|ui| {
            ui.label("Color");
            if ui.color_edit_button_rgb(&mut color3).changed() {
                custom.base_color[0] = color3[0];
                custom.base_color[1] = color3[1];
                custom.base_color[2] = color3[2];
                changed = true;
            }
        });

        // Opacity slider
        changed |= ui
            .add(egui::Slider::new(&mut custom.base_color[3], 0.0..=1.0).text("Opacity"))
            .changed();

        // Roughness slider
        changed |= ui
            .add(egui::Slider::new(&mut custom.roughness, 0.0..=1.0).text("Roughness"))
            .changed();

        // Metallic slider
        changed |= ui
            .add(egui::Slider::new(&mut custom.metallic, 0.0..=1.0).text("Metallic"))
            .changed();
    }

    // Show a preview swatch for presets
    if let MaterialOverride::Preset(preset) = material {
        let (color, roughness, metallic) = preset.values();
        ui.add_space(2.0);
        ui.horizontal(|ui| {
            let swatch_color = egui::Color32::from_rgb(
                (color[0] * 255.0) as u8,
                (color[1] * 255.0) as u8,
                (color[2] * 255.0) as u8,
            );
            let (rect, _) = ui.allocate_exact_size(egui::vec2(14.0, 14.0), egui::Sense::hover());
            ui.painter().rect_filled(rect, 2.0, swatch_color);
            ui.label(
                egui::RichText::new(format!(
                    "R:{:.0}% M:{:.0}%",
                    roughness * 100.0,
                    metallic * 100.0
                ))
                .small()
                .color(egui::Color32::from_gray(140)),
            );
        });
    }

    changed
}

// ---------------------------------------------------------------------------
// Transform controls
// ---------------------------------------------------------------------------

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
