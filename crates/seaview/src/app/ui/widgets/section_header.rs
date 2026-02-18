//! Reusable collapsible section header widget.
//!
//! Replaces `egui::CollapsingHeader` for top-level panel sections with a
//! cleaner look: rounded border, no arrow, lighter gray fill.
//!
//! # Usage
//!
//! ```ignore
//! section_header(ui, "Environment Light", "env_light", true, |ui| {
//!     ui.label("Section body here");
//! });
//! ```

use bevy_egui::egui;

/// Background fill for the header bar.
const HEADER_FILL: egui::Color32 = egui::Color32::from_gray(42);

/// Background fill when hovered.
const HEADER_HOVER_FILL: egui::Color32 = egui::Color32::from_gray(50);

/// Border/stroke colour.
const HEADER_STROKE_COLOR: egui::Color32 = egui::Color32::from_gray(60);

/// Text colour for the header label.
const HEADER_TEXT_COLOR: egui::Color32 = egui::Color32::from_gray(190);

/// Corner rounding in logical pixels.
const HEADER_ROUNDING: f32 = 4.0;

/// Horizontal padding inside the header bar.
const PAD_X: f32 = 8.0;

/// Render a collapsible section with a styled header.
///
/// Returns `true` if the body is currently open.
pub fn section_header(
    ui: &mut egui::Ui,
    title: impl Into<String>,
    id_salt: impl std::hash::Hash,
    default_open: bool,
    body: impl FnOnce(&mut egui::Ui),
) -> bool {
    let title: String = title.into();
    let id = ui.make_persistent_id(id_salt);

    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        id,
        default_open,
    );

    // ── Header bar ────────────────────────────────────────────────
    draw_header(ui, &title, &mut state);

    // ── Body ──────────────────────────────────────────────────────
    let is_open = state.is_open();
    state.show_body_unindented(ui, |ui| {
        ui.add_space(2.0);
        body(ui);
        ui.add_space(2.0);
    });

    is_open
}

/// Same as [`section_header`] but takes a mutable `open` flag that is kept
/// in sync with the collapsing state. Useful when the caller tracks
/// open/closed state externally (e.g. in `CollapsibleState`).
pub fn section_header_tracked(
    ui: &mut egui::Ui,
    title: impl Into<String>,
    id_salt: impl std::hash::Hash,
    open: &mut bool,
    body: impl FnOnce(&mut egui::Ui),
) {
    let title: String = title.into();
    let id = ui.make_persistent_id(&id_salt);

    let mut state =
        egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, *open);

    // Sync: if caller changed the flag externally, honour it.
    if state.is_open() != *open {
        state.set_open(*open);
    }

    // ── Header bar ────────────────────────────────────────────────
    draw_header(ui, &title, &mut state);

    // Update caller's flag.
    *open = state.is_open();

    // ── Body ──────────────────────────────────────────────────────
    state.show_body_unindented(ui, |ui| {
        ui.add_space(2.0);
        body(ui);
        ui.add_space(2.0);
    });
}

// ---------------------------------------------------------------------------
// Internal
// ---------------------------------------------------------------------------

/// Draws the rounded header bar and handles click-to-toggle.
fn draw_header(
    ui: &mut egui::Ui,
    title: &str,
    state: &mut egui::collapsing_header::CollapsingState,
) {
    let is_open = state.is_open();

    ui.scope(|ui| {
        let (rect, response) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 24.0), egui::Sense::click());

        if ui.is_rect_visible(rect) {
            let fill = if response.hovered() {
                HEADER_HOVER_FILL
            } else {
                HEADER_FILL
            };

            // Rounded background + border
            ui.painter().rect(
                rect,
                HEADER_ROUNDING,
                fill,
                egui::Stroke::new(1.0, HEADER_STROKE_COLOR),
                egui::StrokeKind::Outside,
            );

            // Open/closed indicator (subtle chevron, not the default arrow)
            let indicator = if is_open { "▾" } else { "▸" };
            let indicator_pos = egui::pos2(rect.left() + PAD_X, rect.center().y - 6.0);
            ui.painter().text(
                indicator_pos,
                egui::Align2::LEFT_TOP,
                indicator,
                egui::FontId::proportional(12.0),
                egui::Color32::from_gray(100),
            );

            // Title text
            let text_pos = egui::pos2(rect.left() + PAD_X + 14.0, rect.center().y - 6.5);
            ui.painter().text(
                text_pos,
                egui::Align2::LEFT_TOP,
                title,
                egui::FontId::proportional(13.0),
                if response.hovered() {
                    egui::Color32::from_gray(220)
                } else {
                    HEADER_TEXT_COLOR
                },
            );
        }

        if response.clicked() {
            state.toggle(ui);
        }
    });
}
