//! Window management systems (fullscreen toggle, etc.)

use bevy::prelude::*;
use bevy::window::{PrimaryWindow, WindowMode};

/// Toggle borderless fullscreen with the **F** key.
pub fn fullscreen_toggle(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut window_query: Query<&mut Window, With<PrimaryWindow>>,
) {
    if keyboard.just_pressed(KeyCode::KeyF) {
        let Ok(mut window) = window_query.single_mut() else {
            return;
        };

        window.mode = match window.mode {
            WindowMode::Windowed => WindowMode::BorderlessFullscreen(MonitorSelection::Current),
            _ => WindowMode::Windowed,
        };

        debug!(
            "Window mode: {}",
            match window.mode {
                WindowMode::Windowed => "windowed",
                _ => "fullscreen",
            }
        );
    }
}
