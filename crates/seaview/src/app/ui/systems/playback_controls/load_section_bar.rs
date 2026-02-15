//! Load-section bar widget
//!
//! Renders one or more load-section ranges on a thin horizontal bar beneath
//! the timeline scrubber.  Each section is drawn as a highlighted region with
//! draggable edge handles.
//!
//! **Multi-section interactions (seaview-wme):**
//! - **Ctrl+Click** on an empty region to add a new section centred there.
//! - **Right-click** on an existing section to remove it.
//! - Sections are colour-coded with alternating hues so the user can tell
//!   them apart at a glance.
//! - A small section-count label is drawn at the right edge.

use bevy_egui::egui;

use crate::app::ui::state::LoadSectionEdge;
use crate::lib::sequence::Timeline;

/// Persistent drag state tracked across frames.
///
/// Stored in the parent system's `Local` so that a drag that starts on one
/// edge stays locked to that edge even if the pointer drifts away.
pub struct LoadSectionDragState {
    /// Which section index is being dragged (index into `timeline.sections`).
    pub section_idx: usize,
    /// Which edge of that section (or `Body` for whole-section move).
    pub edge: LoadSectionEdge,
    /// For `Body` drags: the frame offset from section start to the grab point.
    /// E.g. if the section starts at frame 100 and the user clicks at frame 130,
    /// `grab_offset` is 30 so the section tracks the mouse naturally.
    pub grab_offset: usize,
}

// ---------------------------------------------------------------------------
// Colour palette — one fill / handle pair per section (cycles)
// ---------------------------------------------------------------------------

/// (fill, handle) colour pairs for up to 6 sections; wraps around after that.
const SECTION_COLORS: [(egui::Color32, egui::Color32); 6] = [
    (
        egui::Color32::from_rgb(60, 120, 80),   // green fill
        egui::Color32::from_rgb(100, 180, 120), // green handle
    ),
    (
        egui::Color32::from_rgb(60, 90, 140),   // blue fill
        egui::Color32::from_rgb(100, 140, 200), // blue handle
    ),
    (
        egui::Color32::from_rgb(130, 90, 50),  // orange fill
        egui::Color32::from_rgb(190, 140, 80), // orange handle
    ),
    (
        egui::Color32::from_rgb(110, 60, 120),  // purple fill
        egui::Color32::from_rgb(170, 110, 180), // purple handle
    ),
    (
        egui::Color32::from_rgb(120, 110, 50), // olive fill
        egui::Color32::from_rgb(180, 170, 90), // olive handle
    ),
    (
        egui::Color32::from_rgb(50, 120, 120), // teal fill
        egui::Color32::from_rgb(90, 180, 180), // teal handle
    ),
];

fn section_fill(index: usize) -> egui::Color32 {
    SECTION_COLORS[index % SECTION_COLORS.len()].0
}

fn section_handle(index: usize) -> egui::Color32 {
    SECTION_COLORS[index % SECTION_COLORS.len()].1
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Draw the load-section bar for all sections in the timeline.
///
/// `drag_state` is `&mut Option<LoadSectionDragState>` so the caller (the
/// playback-controls system) can persist it across frames.  Pass the same
/// mutable reference every frame; we `take()` on drag-end.
pub fn draw_load_section_bar(
    ui: &mut egui::Ui,
    timeline: &mut Timeline,
    display_total_frames: usize,
    max_loaded_frames: usize,
    drag_state: &mut Option<LoadSectionDragState>,
) {
    if display_total_frames == 0 {
        return;
    }

    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.add_space(10.0);

        let bar_height = 14.0;
        let handle_width = 8.0;
        // Reserve a little space on the right for the section-count label.
        let label_width = 40.0;
        let desired_size = egui::Vec2::new(ui.available_width() - 20.0 - label_width, bar_height);
        let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click_and_drag());

        let total = display_total_frames.max(1) as f32;
        let budget = max_loaded_frames.min(display_total_frames);

        // =================================================================
        // Interaction: Ctrl+Click to ADD a section
        // =================================================================
        if response.clicked() && ui.input(|i| i.modifiers.ctrl) {
            if let Some(pos) = response.interact_pointer_pos() {
                let rel_x = (pos.x - rect.min.x) / rect.width();
                let frame = (rel_x * total).round().max(0.0) as usize;
                // Only add if the click is NOT inside an existing section
                if !timeline.is_frame_loaded(frame) {
                    let _ = timeline.add_section(frame, budget);
                }
            }
        }

        // =================================================================
        // Interaction: Right-click to REMOVE a section
        // =================================================================
        if response.secondary_clicked() {
            if let Some(pos) = response.interact_pointer_pos() {
                let rel_x = (pos.x - rect.min.x) / rect.width();
                let frame = (rel_x * total).round().max(0.0) as usize;
                if let Some((idx, _)) = timeline.section_containing_indexed(frame) {
                    // Don't allow removing the last section (remove_section
                    // restores a default, but it's a confusing UX to remove
                    // the only section).
                    if timeline.sections.len() > 1 {
                        timeline.remove_section(idx, budget);
                    }
                }
            }
        }

        // =================================================================
        // Interaction: dragging section edges
        // =================================================================
        if response.dragged() {
            if let Some(pos) = response.interact_pointer_pos() {
                let rel_x = (pos.x - rect.min.x) / rect.width();
                let frame = (rel_x * total).round().max(0.0) as usize;

                match drag_state {
                    Some(ref ds) => {
                        // Continue an existing drag
                        let idx = ds.section_idx;
                        if idx < timeline.sections.len() {
                            match ds.edge {
                                LoadSectionEdge::Start => {
                                    timeline.sections[idx].set_start(frame, budget);
                                }
                                LoadSectionEdge::End => {
                                    timeline.sections[idx].set_end(
                                        frame,
                                        display_total_frames,
                                        budget,
                                    );
                                }
                                LoadSectionEdge::Body => {
                                    let new_start = frame.saturating_sub(ds.grab_offset);
                                    timeline.sections[idx].move_to(new_start, display_total_frames);
                                }
                            }
                        }
                    }
                    None => {
                        // Determine which section edge (or body) is closest to the pointer
                        if let Some((idx, edge)) =
                            hit_test_section(timeline, rect, pos.x, handle_width)
                        {
                            let grab_offset = match edge {
                                LoadSectionEdge::Body => {
                                    frame.saturating_sub(timeline.sections[idx].start)
                                }
                                _ => 0,
                            };
                            match edge {
                                LoadSectionEdge::Start => {
                                    timeline.sections[idx].set_start(frame, budget);
                                }
                                LoadSectionEdge::End => {
                                    timeline.sections[idx].set_end(
                                        frame,
                                        display_total_frames,
                                        budget,
                                    );
                                }
                                LoadSectionEdge::Body => {
                                    let new_start = frame.saturating_sub(grab_offset);
                                    timeline.sections[idx].move_to(new_start, display_total_frames);
                                }
                            }
                            *drag_state = Some(LoadSectionDragState {
                                section_idx: idx,
                                edge,
                                grab_offset,
                            });
                        }
                    }
                }
            }
        } else {
            // Drag ended
            *drag_state = None;
        }

        // Change cursor when hovering over section handles or body
        if response.hovered() {
            if let Some(pos) = ui.ctx().pointer_hover_pos() {
                match hit_test_section(timeline, rect, pos.x, handle_width) {
                    Some((_, LoadSectionEdge::Start | LoadSectionEdge::End)) => {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                    }
                    Some((_, LoadSectionEdge::Body)) => {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                    }
                    None => {}
                }
            }
        }

        // =================================================================
        // Painting
        // =================================================================
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();

            // Background (unloaded region)
            painter.rect_filled(rect, 2.0, egui::Color32::from_gray(35));

            // Draw each section with its own colour
            for (i, section) in timeline.sections.iter().enumerate() {
                let section_start = section.start as f32 / total;
                let section_end = section.end as f32 / total;

                if section_end <= section_start {
                    continue;
                }

                let start_x = rect.min.x + rect.width() * section_start;
                let end_x = rect.min.x + rect.width() * section_end;

                // Loaded region
                let loaded_rect = egui::Rect::from_min_max(
                    egui::pos2(start_x, rect.min.y),
                    egui::pos2(end_x, rect.max.y),
                );
                painter.rect_filled(loaded_rect, 2.0, section_fill(i));

                // Left handle
                let left_handle = egui::Rect::from_min_size(
                    egui::pos2(start_x - handle_width / 2.0, rect.min.y),
                    egui::Vec2::new(handle_width, bar_height),
                );
                painter.rect_filled(left_handle, 2.0, section_handle(i));

                // Right handle
                let right_handle = egui::Rect::from_min_size(
                    egui::pos2(end_x - handle_width / 2.0, rect.min.y),
                    egui::Vec2::new(handle_width, bar_height),
                );
                painter.rect_filled(right_handle, 2.0, section_handle(i));
            }

            // Border
            painter.rect_stroke(
                rect,
                2.0,
                egui::Stroke::new(1.0, egui::Color32::from_gray(60)),
                egui::StrokeKind::Outside,
            );
        }

        // Tooltip — summarise all sections
        let tooltip = section_tooltip(timeline, max_loaded_frames, display_total_frames);
        response.on_hover_text(tooltip);

        // Section-count label to the right of the bar
        ui.add_space(4.0);
        let n = timeline.sections.len();
        let label_text = if n == 1 {
            "1 section".to_string()
        } else {
            format!("{} sections", n)
        };
        ui.label(
            egui::RichText::new(label_text)
                .small()
                .color(egui::Color32::from_gray(140)),
        );
    });
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Hit-test the load-section bar: edges first, then body.
///
/// Returns `(section_index, edge_or_body)` or `None` if pointer is on empty
/// background.  Edge handles have priority over the body so that thin sections
/// are still resizable.
fn hit_test_section(
    timeline: &Timeline,
    rect: egui::Rect,
    pointer_x: f32,
    handle_threshold: f32,
) -> Option<(usize, LoadSectionEdge)> {
    let total = timeline.total_frames.max(1) as f32;

    // Pass 1: check edges (highest priority)
    let mut best_edge: Option<(usize, LoadSectionEdge, f32)> = None;

    for (i, section) in timeline.sections.iter().enumerate() {
        let start_x = rect.min.x + rect.width() * (section.start as f32 / total);
        let end_x = rect.min.x + rect.width() * (section.end as f32 / total);

        let d_start = (pointer_x - start_x).abs();
        let d_end = (pointer_x - end_x).abs();

        if d_start < handle_threshold * 1.5 {
            if best_edge.as_ref().map_or(true, |b| d_start < b.2) {
                best_edge = Some((i, LoadSectionEdge::Start, d_start));
            }
        }
        if d_end < handle_threshold * 1.5 {
            if best_edge.as_ref().map_or(true, |b| d_end < b.2) {
                best_edge = Some((i, LoadSectionEdge::End, d_end));
            }
        }
    }

    if let Some((idx, edge, _)) = best_edge {
        return Some((idx, edge));
    }

    // Pass 2: check section bodies (pointer is inside but not near an edge)
    for (i, section) in timeline.sections.iter().enumerate() {
        let start_x = rect.min.x + rect.width() * (section.start as f32 / total);
        let end_x = rect.min.x + rect.width() * (section.end as f32 / total);

        if pointer_x >= start_x && pointer_x <= end_x {
            return Some((i, LoadSectionEdge::Body));
        }
    }

    None
}

/// Build a tooltip string summarising all sections.
fn section_tooltip(
    timeline: &Timeline,
    max_loaded_frames: usize,
    display_total_frames: usize,
) -> String {
    let budget = max_loaded_frames.min(display_total_frames);

    if timeline.sections.len() == 1 {
        let s = &timeline.sections[0];
        format!(
            "Loaded: frames {} – {} ({} frames)\n\
             Max: {} frames\n\
             Drag edges to adjust\n\
             Ctrl+Click empty area to add a section",
            s.start,
            s.end.saturating_sub(1),
            s.frame_count(),
            budget,
        )
    } else {
        let mut lines = Vec::with_capacity(timeline.sections.len() + 4);
        for (i, s) in timeline.sections.iter().enumerate() {
            lines.push(format!(
                "Section {}: {} – {} ({} frames)",
                i + 1,
                s.start,
                s.end.saturating_sub(1),
                s.frame_count(),
            ));
        }
        lines.push(format!(
            "Total loaded: {} / {} budget",
            timeline.total_loaded(),
            budget,
        ));
        lines.push(String::new());
        lines.push("Drag edges to adjust".to_string());
        lines.push("Ctrl+Click empty area to add a section".to_string());
        lines.push("Right-click a section to remove it".to_string());
        lines.join("\n")
    }
}
