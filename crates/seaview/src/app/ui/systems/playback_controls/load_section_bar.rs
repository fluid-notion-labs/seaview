//! Load-section bar widget
//!
//! Renders one or more load-section ranges on a thin horizontal bar beneath
//! the timeline scrubber.  Each section is drawn as a highlighted region with
//! draggable edge handles.
//!
//! **Multi-section interactions (seaview-wme):**
//! - **Ctrl+Drag** on an empty region to paint a new section: click sets
//!   the start, drag right to extend the end.
//! - **Right-click** on an existing section to remove it.
//! - Sections are colour-coded with alternating hues so the user can tell
//!   them apart at a glance.
//! - A small section-count label is drawn at the right edge.

use bevy_egui::egui;

use crate::app::ui::state::LoadSectionEdge;
use crate::lib::sequence::Timeline;

use super::TRACK_LEFT_MARGIN;
use super::TRACK_RIGHT_RESERVE;

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
        ui.add_space(TRACK_LEFT_MARGIN);

        let bar_height = 14.0;
        let handle_width = 8.0;
        let bar_width = (ui.available_width() - TRACK_RIGHT_RESERVE).max(0.0);
        let desired_size = egui::Vec2::new(bar_width, bar_height);
        let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click_and_drag());

        let total = display_total_frames.max(1) as f32;
        let budget = max_loaded_frames.min(display_total_frames);

        // =================================================================
        // Interaction: Ctrl+click to TOGGLE section active/inactive
        // =================================================================
        if response.clicked() && ui.input(|i| i.modifiers.ctrl) {
            if let Some(pos) = response.interact_pointer_pos() {
                let rel_x = (pos.x - rect.min.x) / rect.width();
                let frame = (rel_x * total).round().max(0.0) as usize;
                if let Some((idx, section)) = timeline.section_containing_indexed_any(frame) {
                    if section.active {
                        timeline.deactivate_section(idx);
                    } else {
                        timeline.activate_section(idx, budget);
                    }
                }
            }
        }

        // =================================================================
        // Interaction: Click on inactive section to ACTIVATE it
        // =================================================================
        if response.clicked() && !ui.input(|i| i.modifiers.ctrl) {
            if let Some(pos) = response.interact_pointer_pos() {
                let rel_x = (pos.x - rect.min.x) / rect.width();
                let frame = (rel_x * total).round().max(0.0) as usize;
                if let Some((idx, section)) = timeline.section_containing_indexed_any(frame) {
                    if !section.active {
                        timeline.activate_section(idx, budget);
                    }
                }
            }
        }

        // =================================================================
        // Interaction: Ctrl+Right-click to REMOVE a section
        // =================================================================
        if response.secondary_clicked() && ui.input(|i| i.modifiers.ctrl) {
            if let Some(pos) = response.interact_pointer_pos() {
                let rel_x = (pos.x - rect.min.x) / rect.width();
                let frame = (rel_x * total).round().max(0.0) as usize;
                if let Some((idx, _)) = timeline.section_containing_indexed_any(frame) {
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
                            apply_drag(
                                timeline,
                                idx,
                                ds.edge,
                                frame,
                                ds.grab_offset,
                                display_total_frames,
                            );
                        }
                    }
                    None => {
                        let is_ctrl = ui.input(|i| i.modifiers.ctrl);

                        if is_ctrl && !timeline.is_frame_in_any_section(frame) {
                            // -------------------------------------------------
                            // Ctrl+drag on empty space: paint a new section.
                            // Create a minimal section starting at the pointer;
                            // the user drags right to extend the End edge.
                            // -------------------------------------------------
                            if let Some(new_idx) = timeline.add_section_at(frame, budget) {
                                // Immediately apply the first drag tick so the
                                // end edge tracks the pointer.
                                apply_drag(
                                    timeline,
                                    new_idx,
                                    LoadSectionEdge::End,
                                    frame,
                                    0,
                                    display_total_frames,
                                );
                                *drag_state = Some(LoadSectionDragState {
                                    section_idx: new_idx,
                                    edge: LoadSectionEdge::End,
                                    grab_offset: 0,
                                });
                            }
                        } else if let Some((idx, edge)) =
                            hit_test_section(timeline, rect, pos.x, handle_width)
                        {
                            // Normal drag on an existing section edge / body.
                            let grab_offset = match edge {
                                LoadSectionEdge::Body => {
                                    frame.saturating_sub(timeline.sections[idx].start)
                                }
                                _ => 0,
                            };
                            apply_drag(
                                timeline,
                                idx,
                                edge,
                                frame,
                                grab_offset,
                                display_total_frames,
                            );
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
                let is_ctrl = ui.input(|i| i.modifiers.ctrl);
                match hit_test_section(timeline, rect, pos.x, handle_width) {
                    Some((_, LoadSectionEdge::Start | LoadSectionEdge::End)) => {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                    }
                    Some((_, LoadSectionEdge::Body)) => {
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
                    }
                    None if is_ctrl => {
                        // Ctrl over empty space → show crosshair to hint
                        // that the user can paint a new section here.
                        ui.ctx().set_cursor_icon(egui::CursorIcon::Crosshair);
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

            // Draw each section — active sections get full colour,
            // inactive sections are dimmed/greyed out.
            for (i, section) in timeline.sections.iter().enumerate() {
                let section_start = section.start as f32 / total;
                let section_end = section.end as f32 / total;

                if section_end <= section_start {
                    continue;
                }

                let start_x = rect.min.x + rect.width() * section_start;
                let end_x = rect.min.x + rect.width() * section_end;

                let (fill, handle_color) = if section.active {
                    (section_fill(i), section_handle(i))
                } else {
                    // Inactive: greyed out
                    (egui::Color32::from_gray(55), egui::Color32::from_gray(75))
                };

                // Section region
                let loaded_rect = egui::Rect::from_min_max(
                    egui::pos2(start_x, rect.min.y),
                    egui::pos2(end_x, rect.max.y),
                );
                painter.rect_filled(loaded_rect, 2.0, fill);

                // Left handle
                let left_handle = egui::Rect::from_min_size(
                    egui::pos2(start_x - handle_width / 2.0, rect.min.y),
                    egui::Vec2::new(handle_width, bar_height),
                );
                painter.rect_filled(left_handle, 2.0, handle_color);

                // Right handle
                let right_handle = egui::Rect::from_min_size(
                    egui::pos2(end_x - handle_width / 2.0, rect.min.y),
                    egui::Vec2::new(handle_width, bar_height),
                );
                painter.rect_filled(right_handle, 2.0, handle_color);
            }

            // Border
            painter.rect_stroke(
                rect,
                2.0,
                egui::Stroke::new(1.0_f32, egui::Color32::from_gray(60)),
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

/// Apply a drag operation on section `idx`, clamping against neighbor sections
/// so that sections never overlap each other.
///
/// No budget constraints — sections can be any size.  Budget is enforced
/// through the visibility (active/inactive) model, not by limiting drag.
fn apply_drag(
    timeline: &mut Timeline,
    idx: usize,
    edge: LoadSectionEdge,
    frame: usize,
    grab_offset: usize,
    display_total_frames: usize,
) {
    let n = timeline.sections.len();

    // Compute neighbor boundaries — the closest edge we must not cross.
    // Previous section's end is our minimum start; next section's start is
    // our maximum end.
    let prev_end = if idx > 0 {
        timeline.sections[idx - 1].end
    } else {
        0
    };
    let next_start = if idx + 1 < n {
        timeline.sections[idx + 1].start
    } else {
        display_total_frames
    };

    match edge {
        LoadSectionEdge::Start => {
            let clamped_frame = frame.max(prev_end);
            timeline.sections[idx].set_start(clamped_frame);
        }
        LoadSectionEdge::End => {
            let clamped_frame = frame.min(next_start);
            timeline.sections[idx].set_end(clamped_frame, display_total_frames);
        }
        LoadSectionEdge::Body => {
            let size = timeline.sections[idx].frame_count();
            let new_start = frame.saturating_sub(grab_offset);
            let min_start = prev_end;
            let max_start = next_start.saturating_sub(size);
            let clamped_start = new_start.clamp(min_start, max_start);
            timeline.sections[idx].move_to(clamped_start, display_total_frames);
        }
    }
}

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
    let threshold = handle_threshold * 1.5;

    // Collect all candidate edges within the handle threshold.
    // We need all of them so we can disambiguate overlapping edges at
    // shared boundaries between adjacent sections.
    struct Candidate {
        section_idx: usize,
        edge: LoadSectionEdge,
        distance: f32,
        /// Signed offset: negative = pointer is left of edge, positive = right
        signed: f32,
    }

    let mut candidates: Vec<Candidate> = Vec::new();

    for (i, section) in timeline.sections.iter().enumerate() {
        let start_x = rect.min.x + rect.width() * (section.start as f32 / total);
        let end_x = rect.min.x + rect.width() * (section.end as f32 / total);

        let d_start = pointer_x - start_x;
        let d_end = pointer_x - end_x;

        if d_start.abs() < threshold {
            candidates.push(Candidate {
                section_idx: i,
                edge: LoadSectionEdge::Start,
                distance: d_start.abs(),
                signed: d_start,
            });
        }
        if d_end.abs() < threshold {
            candidates.push(Candidate {
                section_idx: i,
                edge: LoadSectionEdge::End,
                distance: d_end.abs(),
                signed: d_end,
            });
        }
    }

    if !candidates.is_empty() {
        // Sort by distance first (closest wins).
        candidates.sort_by(|a, b| a.distance.partial_cmp(&b.distance).unwrap());

        let best_dist = candidates[0].distance;
        // Epsilon for "same position" — edges closer than 1px are tied.
        let tie_eps = 1.0_f32;

        // Collect all candidates that are tied with the best.
        let tied: Vec<&Candidate> = candidates
            .iter()
            .take_while(|c| c.distance - best_dist < tie_eps)
            .collect();

        if tied.len() == 1 {
            return Some((tied[0].section_idx, tied[0].edge));
        }

        // Multiple edges at the same position (e.g. section A's End and
        // section B's Start at a shared boundary).  Use pointer bias:
        //   - Pointer left of boundary  → prefer the End edge (shrink left section)
        //   - Pointer right of boundary → prefer the Start edge (shrink right section)
        //   - Exactly on it             → prefer Start (more intuitive: "grab next")
        for c in &tied {
            match c.edge {
                LoadSectionEdge::End if c.signed < -0.5 => {
                    return Some((c.section_idx, c.edge));
                }
                LoadSectionEdge::Start if c.signed >= -0.5 => {
                    return Some((c.section_idx, c.edge));
                }
                _ => {}
            }
        }

        // Fallback: return the first candidate.
        return Some((tied[0].section_idx, tied[0].edge));
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

    let mut lines = Vec::with_capacity(timeline.sections.len() + 6);
    for (i, s) in timeline.sections.iter().enumerate() {
        let status = if s.active { "●" } else { "○" };
        lines.push(format!(
            "{} Section {}: {} – {} ({} frames)",
            status,
            i + 1,
            s.start,
            s.end.saturating_sub(1),
            s.frame_count(),
        ));
    }
    lines.push(format!(
        "Active: {} / {} total frames (budget: {})",
        timeline.total_loaded(),
        timeline
            .sections
            .iter()
            .map(|s| s.frame_count())
            .sum::<usize>(),
        budget,
    ));
    lines.push(String::new());
    lines.push("Drag edges to adjust".to_string());
    lines.push("Click inactive section to activate".to_string());
    lines.push("Ctrl+Drag empty area to add a section".to_string());
    lines.push("Right-click a section to remove it".to_string());
    lines.join("\n")
}
