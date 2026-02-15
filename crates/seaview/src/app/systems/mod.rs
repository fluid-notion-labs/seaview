pub mod camera;
pub mod diagnostics;
// pub mod network;
pub mod window;

use bevy::prelude::*;

/// Plugin that registers app-level systems (window management, etc.)
pub struct AppSystemsPlugin;

impl Plugin for AppSystemsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, window::fullscreen_toggle);
    }
}
