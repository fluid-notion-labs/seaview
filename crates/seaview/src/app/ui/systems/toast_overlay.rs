//! Toast overlay system
//!
//! Renders floating toast notifications in the top-right corner of the viewport.
//! Toasts auto-dismiss after their duration and fade out near the end.
//! Clicking a toast dismisses it immediately.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::app::ui::state::UiState;
use crate::lib::toast::ToastLevel;

/// Vertical offset from the top of the screen for the first toast.
const TOP_OFFSET: f32 = 40.0;

/// Right margin from the edge of the screen.
const RIGHT_MARGIN: f32 = 16.0;

/// Width of each toast notification.
const TOAST_WIDTH: f32 = 320.0;

/// System that renders active toasts as floating overlays and drains expired ones.
///
/// Toasts appear anchored to the top-right, stacked vertically. Each toast
/// fades out during the last 20% of its lifetime and can be clicked to dismiss.
pub fn toast_overlay_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    time: Res<Time>,
) {
    let now = time.elapsed_secs_f64();

    // Drain expired toasts first
    ui_state.toasts.drain_expired(now);

    let active_count = ui_state.toasts.active_count();
    if active_count == 0 {
        return;
    }

    let ctx = contexts.ctx_mut().unwrap();

    // Collect toast data before rendering so we don't hold a mutable borrow
    // across the egui call. We need: message, level, opacity, index.
    let toast_data: Vec<(String, ToastLevel, f32, usize)> = ui_state
        .toasts
        .active()
        .iter()
        .enumerate()
        .map(|(i, t)| (t.message.clone(), t.level, t.opacity(now), i))
        .collect();

    let mut dismiss_index: Option<usize> = None;

    // Use an egui::Area per toast so they float above everything else.
    // Stack them vertically from the top-right corner.
    let content_rect = ctx.content_rect();
    let base_x = content_rect.max.x - TOAST_WIDTH - RIGHT_MARGIN;

    let mut y_offset = TOP_OFFSET;

    for (message, level, opacity, index) in &toast_data {
        if *opacity <= 0.0 {
            continue;
        }

        let (bg_color, accent_color, text_color) = toast_colors(*level, *opacity);

        let area_id = egui::Id::new("toast_overlay").with(*index);

        let area_response = egui::Area::new(area_id)
            .fixed_pos(egui::pos2(base_x, y_offset))
            .order(egui::Order::Foreground)
            .show(ctx, |ui| {
                let frame = egui::Frame::new()
                    .fill(bg_color)
                    .stroke(egui::Stroke::new(1.0, accent_color))
                    .corner_radius(6.0)
                    .inner_margin(egui::Margin::same(10));

                frame.show(ui, |ui| {
                    ui.set_width(TOAST_WIDTH - 20.0); // account for margin

                    ui.horizontal(|ui| {
                        // Level icon
                        ui.label(
                            egui::RichText::new(level.icon())
                                .color(accent_color)
                                .size(14.0),
                        );

                        ui.vertical(|ui| {
                            ui.label(
                                egui::RichText::new(message.as_str())
                                    .color(text_color)
                                    .size(13.0),
                            );
                        });

                        // Dismiss button (×) on the right
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                            let dismiss_btn = ui.add(
                                egui::Button::new(
                                    egui::RichText::new("×").color(text_color).size(14.0),
                                )
                                .frame(false),
                            );
                            if dismiss_btn.clicked() {
                                // Can't mutate toasts here; record for later
                                // (handled via dismiss_index)
                            }
                            dismiss_btn
                        })
                        .inner
                    })
                    .inner
                });
            });

        // Check if the dismiss button or the whole toast area was clicked
        let response = &area_response.response;
        if response.clicked() {
            dismiss_index = Some(*index);
        }

        // Advance y for the next toast. Estimate height from the area response.
        y_offset += area_response.response.rect.height() + 6.0;
    }

    // Apply dismissal outside the render loop
    if let Some(idx) = dismiss_index {
        ui_state.toasts.dismiss(idx);
    }
}

/// Returns `(background, accent, text)` colors for a toast level, with opacity applied.
fn toast_colors(level: ToastLevel, opacity: f32) -> (egui::Color32, egui::Color32, egui::Color32) {
    let alpha = (opacity * 230.0) as u8;
    let text_alpha = (opacity * 255.0) as u8;

    match level {
        ToastLevel::Info => (
            egui::Color32::from_rgba_unmultiplied(30, 40, 60, alpha),
            egui::Color32::from_rgba_unmultiplied(80, 160, 255, text_alpha),
            egui::Color32::from_rgba_unmultiplied(220, 225, 235, text_alpha),
        ),
        ToastLevel::Success => (
            egui::Color32::from_rgba_unmultiplied(25, 50, 35, alpha),
            egui::Color32::from_rgba_unmultiplied(80, 200, 120, text_alpha),
            egui::Color32::from_rgba_unmultiplied(220, 235, 225, text_alpha),
        ),
        ToastLevel::Warning => (
            egui::Color32::from_rgba_unmultiplied(55, 45, 20, alpha),
            egui::Color32::from_rgba_unmultiplied(240, 180, 50, text_alpha),
            egui::Color32::from_rgba_unmultiplied(240, 235, 215, text_alpha),
        ),
        ToastLevel::Error => (
            egui::Color32::from_rgba_unmultiplied(55, 25, 25, alpha),
            egui::Color32::from_rgba_unmultiplied(240, 80, 80, text_alpha),
            egui::Color32::from_rgba_unmultiplied(240, 220, 220, text_alpha),
        ),
    }
}
