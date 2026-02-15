//! Load-section bar widget
//!
//! Renders one or more load-section ranges on a thin horizontal bar beneath
//! the timeline scrubber.  Each section is drawn as a highlighted region with
//! draggable edge handles.
//!
//! Extracted from the monolithic `playback_controls_system` so the drawing
//! logic is isolated and ready for multi-section support (seaview-wme).

use bevy_egui::egui;

use crate::app::ui::state::LoadSectionEdge;
use crate::lib::sequence::Timeline;

/// Persistent drag state tracked across frames.
///
/// Stored in the parent system's `UiState.load_section_dragging` field so that
/// a drag that starts on one edge stays locked to that edge even if the pointer
/// drifts away.
pub struct LoadSectionDragState {
    /// Which section index is being dragged (index into `timeline.sections`).
    pub section_idx: usize,
    /// Which edge of that section.
    pub edge: LoadSectionEdge,
}

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
        let desired_size = egui::Vec2::new(ui.available_width() - 20.0, bar_height);
        let (rect, response) =
            ui.allocate_exact_size(desired_size, egui::Sense::click_and_drag());

        let total = display_total_frames.max(1) as f32;

        // -----------------------------------------------------------------
        // Interaction: dragging section edges
        // -----------------------------------------------------------------
        if response.dragged() {
            if let Some(pos) = response.interact_pointer_pos() {
                let rel_x = (pos.x - rect.min.x) / rect.width();
                let frame = (rel_x * total).round().max(0.0) as usize;
                let budget = max_loaded_frames.min(display_total_frames);

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
                                    timeline.sections[idx]
                                        .set_end(frame, display_total_frames, budget);
                                }
                            }
                        }
                    }
                    None => {
                        // Determine which section edge is closest to the pointer
                        if let Some((idx, edge)) =
                            closest_edge(&timeline, rect, pos.x, handle_width)
                        {
                            match edge {
                                LoadSectionEdge::Start => {
                                    timeline.sections[idx].set_start(frame, budget);
                                }
                                LoadSectionEdge::End => {
                                    timeline.sections[idx]
                                        .set_end(frame, display_total_frames, budget);
                                }
                            }
                            *drag_state = Some(LoadSectionDragState {
                                section_idx: idx,
                                edge,
                            });
                        }
                    }
                }
            }
        } else {
            // Drag ended
            *drag_state = None;
        }

        // Change cursor when hovering over any section handle
        if response.hovered() {
            if let Some(pos) = ui.ctx().pointer_hover_pos() {
                if closest_edge(&timeline, rect, pos.x, handle_width).is_some() {
                    ui.ctx()
                        .set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
                }
            }
        }

        // -----------------------------------------------------------------
        // Painting
        // -----------------------------------------------------------------
        if ui.is_rect_visible(rect) {
            let painter = ui.painter();

            // Background (unloaded region)
            painter.rect_filled(rect, 2.0, egui::Color32::from_gray(35));

            // Draw each section
            for section in &timeline.sections {
                let section_start = section.start as f32 / total;
                let section_end = section.end as f32 / total;

                if section_end <= section_start {
                    continue;
                }

                let start_x = rect.min.x + rect.width() * section_start;
                let end_x = rect.min.x + rect.width() * section_end;

                // Loaded region — green
                let loaded_rect = egui::Rect::from_min_max(
                    egui::pos2(start_x, rect.min.y),
                    egui::pos2(end_x, rect.max.y),
                );
                painter.rect_filled(
                    loaded_rect,
                    2.0,
                    egui::Color32::from_rgb(60, 120, 80),
                );

                // Left handle
                let left_handle = egui::Rect::from_min_size(
                    egui::pos2(start_x - handle_width / 2.0, rect.min.y),
                    egui::Vec2::new(handle_width, bar_height),
                );
                painter.rect_filled(
                    left_handle,
                    2.0,
                    egui::Color32::from_rgb(100, 180, 120),
                );

                // Right handle
                let right_handle = egui::Rect::from_min_size(
                    egui::pos2(end_x - handle_width / 2.0, rect.min.y),
                    egui::Vec2::new(handle_width, bar_height),
                );
                painter.rect_filled(
                    right_handle,
                    2.0,
                    egui::Color32::from_rgb(100, 180, 120),
                );
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
        let tooltip = section_tooltip(&timeline, max_loaded_frames, display_total_frames);
        response.on_hover_text(tooltip);
    });
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Find the section edge closest to `pointer_x` within `threshold` pixels.
///
/// Returns `(section_index, edge)` or `None` if no edge is close enough.
fn closest_edge(
    timeline: &Timeline,
    rect: egui::Rect,
    pointer_x: f32,
    threshold: f32,
) -> Option<(usize, LoadSectionEdge)> {
    let total = timeline.total_frames.max(1) as f32;
    let mut best: Option<(usize, LoadSectionEdge, f32)> = None;

    for (i, section) in timeline.sections.iter().enumerate() {
        let start_x = rect.min.x + rect.width() * (section.start as f32 / total);
        let end_x = rect.min.x + rect.width() * (section.end as f32 / total);

        let d_start = (pointer_x - start_x).abs();
        let d_end = (pointer_x - end_x).abs();

        if d_start < threshold * 1.5 {
            if best.as_ref().map_or(true, |b| d_start < b.2) {
                best = Some((i, LoadSectionEdge::Start, d_start));
            }
        }
        if d_end < threshold * 1.5 {
            if best.as_ref().map_or(true, |b| d_end < b.2) {
                best = Some((i, LoadSectionEdge::End, d_end));
            }
        }
    }

    best.map(|(idx, edge, _)| (idx, edge))
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
            "Loaded: frames {} – {} ({} frames)\nMax: {} frames\nDrag edges to adjust",
            s.start,
            s.end.saturating_sub(1),
            s.frame_count(),
            budget,
        )
    } else {
        let mut lines = Vec::with_capacity(timeline.sections.len() + 2);
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
        lines.push("Drag edges to adjust".to_string());
        lines.join("\n")
    }
}
