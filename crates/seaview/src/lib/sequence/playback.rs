//! Sequence playback control module
//!
//! Gutted as part of playback rebuild (commit 1).
//! Will be rebuilt with `handle_keyboard_playback` in commit 2.

use bevy::prelude::*;

/// Plugin for sequence playback controls
pub struct SequencePlaybackPlugin;

impl Plugin for SequencePlaybackPlugin {
    fn build(&self, _app: &mut App) {
        // Intentionally empty — all playback systems removed.
        // Will be rebuilt in commit 2 of the playback rebuild.
    }
}
