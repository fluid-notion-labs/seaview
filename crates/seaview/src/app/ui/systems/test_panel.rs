//! Test panel system for Phase 0.2 sanity test
//!
//! This module contains a simple test panel with a button to verify
//! that the egui integration is working correctly.

use bevy::prelude::*;
use bevy_egui::{egui, EguiContexts};

/// Test panel system that displays a simple UI window with a test button
pub fn test_panel_system(mut contexts: EguiContexts) {
    let ctx = contexts.ctx_mut().unwrap();

    // Create a window with a test button
    egui::Window::new("Seaview Test Panel")
        .default_pos([10.0, 10.0])
        .default_size([300.0, 200.0])
        .resizable(true)
        .show(ctx, |ui| {
            ui.heading("Phase 0.2 - Egui Integration Test");

            ui.separator();

            ui.label("This is a test panel to verify egui is working correctly.");

            ui.add_space(10.0);

            // Test button with logging
            if ui.button("Click Me!").clicked() {
                debug!("Test button clicked! Egui integration is working.");
            }

            ui.add_space(10.0);

            // Add some more test widgets
            ui.horizontal(|ui| {
                ui.label("Status:");
                ui.colored_label(egui::Color32::from_rgb(0, 255, 0), "✓ Egui Loaded");
            });

            ui.separator();

            // Instructions
            ui.label("If you can:");
            ui.label("• See this window");
            ui.label("• Click the button above");
            ui.label("• See the log message");
            ui.label("");
            ui.label("Then Phase 0.2 is complete! 🎉");
        });
}
