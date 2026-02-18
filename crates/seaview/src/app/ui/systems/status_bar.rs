//! Status bar system
//!
//! Renders a thin bottom bar below the playback controls showing:
//! - Loading progress (when active)
//! - Current status text (watching, idle, etc.)
//! - Save indicator with fade animation and time-since-saved
//! - Toast history button that toggles a scrollable message log window

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

use crate::app::ui::state::UiState;
use crate::lib::fade::format_relative_time;
use crate::lib::toast::ToastLevel;

/// Height of the status bar in pixels.
const STATUS_BAR_HEIGHT: f32 = 24.0;

/// System that renders the status bar at the very bottom of the screen.
///
/// Shows loading progress, a status summary, save indicator with fade
/// animation, and a button to open the toast history log.
pub fn status_bar_system(
    mut contexts: EguiContexts,
    mut ui_state: ResMut<UiState>,
    time: Res<Time>,
) {
    let ctx = contexts.ctx_mut().unwrap();
    let now = time.elapsed_secs_f64();

    egui::TopBottomPanel::bottom("status_bar")
        .exact_height(STATUS_BAR_HEIGHT)
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;

                // ── Loading indicator ──────────────────────────────
                render_loading_status(ui, &ui_state);

                // ── Save indicator ─────────────────────────────────
                render_save_indicator(ui, &ui_state, now);

                // Spacer pushes the history button to the right
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    // ── History button ─────────────────────────────
                    let history_count = ui_state.toasts.history_count();
                    let button_label = if history_count > 0 {
                        format!("\u{1f4cb} {history_count}")
                    } else {
                        "\u{1f4cb}".to_string()
                    };

                    let history_btn = ui.add(
                        egui::Button::new(
                            egui::RichText::new(button_label)
                                .size(11.0)
                                .color(egui::Color32::from_gray(180)),
                        )
                        .frame(false),
                    );
                    if history_btn.clicked() {
                        ui_state.toasts.show_history = !ui_state.toasts.show_history;
                    }
                    if history_btn.hovered() {
                        history_btn.on_hover_text("Toggle message history");
                    }
                });
            });
        });

    // ── History window (rendered outside the panel) ───────────────
    if ui_state.toasts.show_history {
        render_history_window(ctx, &mut ui_state);
    }
}

/// Renders the save indicator: a floppy-disk icon that flashes on save then
/// fades over 10 seconds, plus a human-readable "time since saved" label.
fn render_save_indicator(ui: &mut egui::Ui, ui_state: &UiState, now: f64) {
    let Some(saved_at) = ui_state.last_saved_at else {
        // No save has happened yet this session — nothing to show.
        return;
    };

    let elapsed = now - saved_at;
    let relative = format_relative_time(elapsed);

    // Determine the icon colour: bright green during the fade, dim grey after.
    let (icon_color, text_color) = if let Some(ref fade) = ui_state.save_fade {
        let opacity = fade.opacity(now);
        if opacity > 0.0 {
            // Interpolate from bright green (active) → dim grey (resting)
            let bright = egui::Color32::from_rgb(80, 200, 120);
            let dim = egui::Color32::from_gray(120);
            let icon = lerp_color(bright, dim, 1.0 - opacity);

            let text_bright = egui::Color32::from_rgb(180, 230, 190);
            let text_dim = egui::Color32::from_gray(120);
            let text = lerp_color(text_bright, text_dim, 1.0 - opacity);
            (icon, text)
        } else {
            // Fade expired — resting state
            (egui::Color32::from_gray(120), egui::Color32::from_gray(120))
        }
    } else {
        // No fade effect (shouldn't happen if last_saved_at is set, but be safe)
        (egui::Color32::from_gray(120), egui::Color32::from_gray(120))
    };

    // ── Render icon + relative time ──────────────────────────────
    ui.add_space(4.0);

    // Thin vertical separator to visually divide from loading status
    ui.separator();

    ui.label(
        egui::RichText::new("\u{1f4be}")
            .size(12.0)
            .color(icon_color),
    );

    ui.label(egui::RichText::new(relative).size(11.0).color(text_color));
}

/// Linearly interpolate between two colours by factor `t` (0.0 = `a`, 1.0 = `b`).
fn lerp_color(a: egui::Color32, b: egui::Color32, t: f32) -> egui::Color32 {
    let t = t.clamp(0.0, 1.0);
    let lerp = |from: u8, to: u8| -> u8 {
        let v = from as f32 * (1.0 - t) + to as f32 * t;
        v.round() as u8
    };
    egui::Color32::from_rgba_unmultiplied(
        lerp(a.r(), b.r()),
        lerp(a.g(), b.g()),
        lerp(a.b(), b.b()),
        lerp(a.a(), b.a()),
    )
}

/// Renders the loading progress or idle status on the left side of the bar.
fn render_loading_status(ui: &mut egui::Ui, ui_state: &UiState) {
    if ui_state.loading.is_loading && !ui_state.loading.is_complete() {
        let progress = ui_state.loading.progress();
        let loaded = ui_state.loading.loaded_frames;
        let total = ui_state.loading.total_frames;
        let pct = (progress * 100.0) as u32;

        let text = if let Some(ref name) = ui_state.loading.sequence_name {
            format!("Loading {name}: {loaded}/{total} ({pct}%)")
        } else {
            format!("Loading: {loaded}/{total} ({pct}%)")
        };

        // Inline progress bar with text overlay
        ui.add(
            egui::ProgressBar::new(progress)
                .desired_width(240.0)
                .text(
                    egui::RichText::new(text)
                        .size(11.0)
                        .color(egui::Color32::from_gray(220)),
                )
                .fill(egui::Color32::from_rgb(40, 100, 180)),
        );
    } else if ui_state.loading.is_complete() {
        let total = ui_state.loading.total_frames;
        let status = if total > 0 {
            format!("\u{2713} {total} frames loaded")
        } else {
            "Ready".to_string()
        };

        ui.label(
            egui::RichText::new(status)
                .size(11.0)
                .color(egui::Color32::from_gray(140)),
        );
    } else {
        ui.label(
            egui::RichText::new("Ready")
                .size(11.0)
                .color(egui::Color32::from_gray(120)),
        );
    }
}

/// Renders the toast history as a floating window anchored to the bottom-right.
fn render_history_window(ctx: &egui::Context, ui_state: &mut UiState) {
    let mut open = ui_state.toasts.show_history;

    egui::Window::new("Message History")
        .open(&mut open)
        .resizable(true)
        .default_width(400.0)
        .default_height(300.0)
        .anchor(egui::Align2::RIGHT_BOTTOM, [-8.0, -STATUS_BAR_HEIGHT - 8.0])
        .show(ctx, |ui| {
            // Toolbar
            ui.horizontal(|ui| {
                let count = ui_state.toasts.history_count();
                ui.label(
                    egui::RichText::new(format!("{count} messages"))
                        .small()
                        .color(egui::Color32::from_gray(140)),
                );

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.small_button("Clear").clicked() {
                        ui_state.toasts.clear_history();
                    }
                });
            });

            ui.separator();

            // Scrollable history list (newest at top)
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    let history = ui_state.toasts.history();
                    if history.is_empty() {
                        ui.label(
                            egui::RichText::new("No messages yet.")
                                .italics()
                                .color(egui::Color32::from_gray(100)),
                        );
                    } else {
                        // Iterate newest-first
                        for toast in history.iter().rev() {
                            render_history_entry(ui, &toast.message, toast.level);
                        }
                    }
                });
        });

    ui_state.toasts.show_history = open;
}

/// Renders a single history entry row.
fn render_history_entry(ui: &mut egui::Ui, message: &str, level: ToastLevel) {
    let accent = history_accent(level);

    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 6.0;

        ui.label(egui::RichText::new(level.icon()).color(accent).size(12.0));

        ui.label(
            egui::RichText::new(message)
                .size(12.0)
                .color(egui::Color32::from_gray(200)),
        );
    });

    ui.add_space(2.0);
}

/// Accent color for a toast level in the history list (full opacity).
fn history_accent(level: ToastLevel) -> egui::Color32 {
    match level {
        ToastLevel::Info => egui::Color32::from_rgb(80, 160, 255),
        ToastLevel::Success => egui::Color32::from_rgb(80, 200, 120),
        ToastLevel::Warning => egui::Color32::from_rgb(240, 180, 50),
        ToastLevel::Error => egui::Color32::from_rgb(240, 80, 80),
    }
}
