//! Window management systems (fullscreen toggle, etc.)

use bevy::prelude::*;
use bevy::window::{PrimaryWindow, WindowMode};

use crate::app::input::ActionStateRes;
use crate::lib::actions::Action;

/// Toggle borderless fullscreen via the action system (default: F key).
pub fn fullscreen_toggle(
    actions: Res<ActionStateRes>,
    mut window_query: Query<&mut Window, With<PrimaryWindow>>,
) {
    if actions.0.just_activated(Action::ToggleFullscreen) {
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
