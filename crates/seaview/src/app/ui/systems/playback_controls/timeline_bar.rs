//! Timeline scrubber bar widget
//!
//! Extracted from the monolithic `playback_controls_system` to keep each
//! UI concern in its own file.

use bevy_egui::egui;

use crate::lib::sequence::Timeline;

use super::TRACK_LEFT_MARGIN;
use super::TRACK_RIGHT_RESERVE;

/// Draw the timeline scrubber bar with a draggable handle.
///
/// Returns the frame the user scrubbed to (if any interaction occurred).
pub fn draw_timeline_bar(
    ui: &mut egui::Ui,
    timeline: &mut Timeline,
    display_current_frame: usize,
    display_total_frames: usize,
) {
    let bar_height = 20.0;
    let handle_width = 12.0;
    let handle_height = 24.0;

    ui.add_space(TRACK_LEFT_MARGIN);
    let bar_width = (ui.available_width() - TRACK_RIGHT_RESERVE).max(0.0);
    let desired_size = egui::Vec2::new(bar_width, handle_height);

    let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click_and_drag());

    let max_frame = display_total_frames.saturating_sub(1).max(1);
    let progress = if display_total_frames > 0 {
        display_current_frame as f32 / max_frame as f32
    } else {
        0.0
    };

    // Handle dragging — scrub to frame
    if response.dragged() || response.clicked() {
        if let Some(pos) = response.interact_pointer_pos() {
            let rel_x = ((pos.x - rect.min.x) / rect.width()).clamp(0.0, 1.0);
            let new_frame = (rel_x * max_frame as f32).round() as usize;
            timeline.jump_to_frame(new_frame);
        }
    }

    // Change cursor when hovering
    if response.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();

        // Bar area (centered vertically)
        let bar_y = rect.center().y - bar_height / 2.0;
        let bar_rect = egui::Rect::from_min_size(
            egui::pos2(rect.min.x, bar_y),
            egui::Vec2::new(rect.width(), bar_height),
        );

        // Background
        painter.rect_filled(bar_rect, 3.0, egui::Color32::from_gray(45));

        // Progress fill (blue)
        if progress > 0.0 {
            let progress_rect = egui::Rect::from_min_size(
                bar_rect.min,
                egui::Vec2::new(bar_rect.width() * progress, bar_height),
            );
            painter.rect_filled(progress_rect, 3.0, egui::Color32::from_rgb(70, 130, 220));
        }

        // Frame markers (every 10%)
        for i in 1..10 {
            let marker_x = bar_rect.min.x + (bar_rect.width() * (i as f32 / 10.0));
            painter.line_segment(
                [
                    egui::pos2(marker_x, bar_rect.min.y),
                    egui::pos2(marker_x, bar_rect.max.y),
                ],
                egui::Stroke::new(1.0, egui::Color32::from_gray(70)),
            );
        }

        // Scrubber handle (vertical bar with rounded ends)
        let handle_x = rect.min.x + (rect.width() * progress);
        let handle_rect = egui::Rect::from_center_size(
            egui::pos2(handle_x, rect.center().y),
            egui::Vec2::new(handle_width, handle_height),
        );

        // Handle shadow
        let shadow_rect = handle_rect.translate(egui::vec2(1.0, 1.0));
        painter.rect_filled(shadow_rect, 3.0, egui::Color32::from_black_alpha(60));

        // Handle body
        painter.rect_filled(
            handle_rect,
            3.0,
            egui::Color32::from_rgb(255, 200, 80), // Orange/yellow
        );

        // Handle border
        painter.rect_stroke(
            handle_rect,
            3.0,
            egui::Stroke::new(1.5, egui::Color32::from_rgb(200, 150, 50)),
            egui::StrokeKind::Outside,
        );

        // Handle grip lines (3 small horizontal lines)
        let grip_color = egui::Color32::from_rgb(180, 130, 40);
        for i in -1..=1 {
            let y = handle_rect.center().y + (i as f32 * 4.0);
            painter.line_segment(
                [
                    egui::pos2(handle_rect.min.x + 3.0, y),
                    egui::pos2(handle_rect.max.x - 3.0, y),
                ],
                egui::Stroke::new(1.0, grip_color),
            );
        }
    }

    // Tooltip
    response.on_hover_text(format!(
        "Frame {} / {}\nClick or drag to scrub",
        display_current_frame,
        display_total_frames.saturating_sub(1)
    ));
}
