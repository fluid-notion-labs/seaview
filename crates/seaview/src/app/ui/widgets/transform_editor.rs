//! Reusable transform editor widget.
//!
//! Provides a compact position / rotation / scale editor that works with
//! [`Transform3D`] — the plain-data transform type used by both environment
//! objects and mesh series.
//!
//! # Usage
//!
//! ```ignore
//! use crate::app::ui::widgets::transform_editor;
//!
//! // Full editor (position + rotation + scale)
//! let changed = transform_editor::transform_editor(ui, &mut obj.transform);
//!
//! // Individual components
//! let changed = transform_editor::position_editor(ui, &mut obj.transform.position);
//! let changed = transform_editor::rotation_editor(ui, &mut obj.transform.rotation_degrees);
//! let changed = transform_editor::scale_editor(ui, &mut obj.transform.scale);
//!
//! // Raw vec3 drag — reusable anywhere
//! let changed = transform_editor::drag_vec3(ui, &mut values, "my_id", 0.1);
//! ```

use bevy_egui::egui;

use crate::lib::environment_objects::Transform3D;

// ---------------------------------------------------------------------------
// Full transform editor
// ---------------------------------------------------------------------------

/// Draw a full transform editor (position + rotation + scale) for a
/// [`Transform3D`].
///
/// Returns `true` if any value was changed by the user.
pub fn transform_editor(ui: &mut egui::Ui, transform: &mut Transform3D) -> bool {
    let mut changed = false;

    changed |= position_editor(ui, &mut transform.position);

    ui.add_space(2.0);
    changed |= rotation_editor(ui, &mut transform.rotation_degrees);

    ui.add_space(2.0);
    changed |= scale_editor(ui, &mut transform.scale);

    changed
}

/// Draw a compact single-line transform summary (position only) with a
/// toggle to expand the full editor.
///
/// Useful in list views where vertical space is tight (e.g. series panel).
/// Returns `true` if any value was changed.
pub fn transform_editor_collapsible(
    ui: &mut egui::Ui,
    id_salt: impl std::hash::Hash,
    transform: &mut Transform3D,
) -> bool {
    let mut changed = false;

    egui::CollapsingHeader::new("Transform")
        .id_salt(id_salt)
        .default_open(false)
        .show(ui, |ui| {
            changed |= transform_editor(ui, transform);
        });

    changed
}

// ---------------------------------------------------------------------------
// Individual component editors
// ---------------------------------------------------------------------------

/// Position editor — three drag values labelled X / Y / Z.
///
/// Returns `true` if any value changed.
pub fn position_editor(ui: &mut egui::Ui, position: &mut [f32; 3]) -> bool {
    ui.label("Position");
    drag_vec3(ui, position, "pos", 0.1)
}

/// Rotation editor — three drag values in degrees labelled X / Y / Z.
///
/// Returns `true` if any value changed.
pub fn rotation_editor(ui: &mut egui::Ui, rotation_degrees: &mut [f32; 3]) -> bool {
    ui.label("Rotation (degrees)");
    drag_vec3(ui, rotation_degrees, "rot", 1.0)
}

/// Scale editor — three drag values labelled X / Y / Z.
///
/// Returns `true` if any value changed.
pub fn scale_editor(ui: &mut egui::Ui, scale: &mut [f32; 3]) -> bool {
    ui.label("Scale");
    drag_vec3(ui, scale, "scale", 0.01)
}

// ---------------------------------------------------------------------------
// Primitives
// ---------------------------------------------------------------------------

/// Axis colours for X (red), Y (green), Z (blue).
const AXIS_COLORS: [egui::Color32; 3] = [
    egui::Color32::from_rgb(200, 80, 80),
    egui::Color32::from_rgb(80, 180, 80),
    egui::Color32::from_rgb(80, 120, 220),
];

/// Three-component drag-value row labelled X / Y / Z.
///
/// This is the low-level building block — use [`position_editor`],
/// [`rotation_editor`], or [`scale_editor`] for labelled variants.
///
/// Returns `true` if any of the three values was changed by the user.
pub fn drag_vec3(
    ui: &mut egui::Ui,
    values: &mut [f32; 3],
    _id_prefix: &str,
    speed: f32,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        for (i, (label, value)) in ["X: ", "Y: ", "Z: "]
            .iter()
            .zip(values.iter_mut())
            .enumerate()
        {
            let response = ui.add(
                egui::DragValue::new(value)
                    .speed(speed)
                    .prefix(*label)
                    .update_while_editing(false),
            );
            // Subtle axis-coloured underline on hover/active
            if response.hovered() || response.dragged() {
                let rect = response.rect;
                ui.painter().line_segment(
                    [rect.left_bottom(), rect.right_bottom()],
                    egui::Stroke::new(2.0, AXIS_COLORS[i]),
                );
            }
            changed |= response.changed();
        }
    });
    changed
}

// ---------------------------------------------------------------------------
// Reset button helper
// ---------------------------------------------------------------------------

/// Draw a small "↺" reset button.  Returns `true` if clicked.
///
/// Typical usage:
/// ```ignore
/// ui.horizontal(|ui| {
///     ui.label("Position");
///     if transform_editor::reset_button(ui, "Reset position to origin") {
///         transform.position = [0.0, 0.0, 0.0];
///     }
/// });
/// ```
pub fn reset_button(ui: &mut egui::Ui, tooltip: &str) -> bool {
    ui.small_button("↺").on_hover_text(tooltip).clicked()
}
