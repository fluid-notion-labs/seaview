//! Sequence playback keyboard controls
//!
//! Rebuilt as part of playback rebuild (commit 2).
//! Single system: `handle_keyboard_playback` — keyboard input writes directly
//! to `Timeline` for frame movement and `SequenceManager` for playback state.
//! No events, no indirection.

use super::{SequenceManager, Timeline};
use bevy::prelude::*;

use crate::lib::settings::SaveViewEvent;

/// Plugin for sequence playback controls
pub struct SequencePlaybackPlugin;

impl Plugin for SequencePlaybackPlugin {
    fn build(&self, _app: &mut App) {
        // Registration handled by SequencePlugin in mod.rs
        // (handle_keyboard_playback needs ordering relative to other sequence systems)
    }
}

/// Resource for tracking key hold timing (arrow key repeat)
#[derive(Resource, Default)]
pub(super) struct KeyHoldTimer {
    /// Time since arrow key was first pressed
    hold_time: f32,
    /// Time of last repeated frame change
    last_repeat: f32,
}

/// Initial delay before key repeat starts (seconds)
const REPEAT_DELAY: f32 = 0.5;
/// Interval between repeated frame changes when holding (seconds)
const REPEAT_INTERVAL: f32 = 0.05;

/// Keyboard input → `Timeline` + `SequenceManager`. Frame movement goes
/// through `Timeline`; play/pause and speed go through `SequenceManager`.
///
/// Key bindings:
/// - **Space** — toggle play/pause
/// - **← / →** — step backward/forward (hold to repeat)
/// - **Home** — jump to first frame
/// - **End** — jump to last frame
/// - **+ / -** — increase/decrease playback speed
/// - **1–0** — jump to 0%–90% of sequence
/// - **Ctrl+S** — save view to seaview.toml
pub(super) fn handle_keyboard_playback(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut sm: ResMut<SequenceManager>,
    mut timeline: ResMut<Timeline>,
    time: Res<Time>,
    mut key_timer: Local<KeyHoldTimer>,
    mut save_events: MessageWriter<SaveViewEvent>,
) {
    // Ctrl+S — save view (must run before the early return so saving works
    // even when no sequence is loaded, e.g. env-object-only sessions).
    let ctrl = keyboard.pressed(KeyCode::ControlLeft) || keyboard.pressed(KeyCode::ControlRight);
    if ctrl && keyboard.just_pressed(KeyCode::KeyS) {
        save_events.write(SaveViewEvent);
        debug!("💾 Save view triggered via Ctrl+S");
    }

    // Only process playback input if we have a sequence loaded
    if timeline.total_frames == 0 {
        return;
    }

    // Space bar — toggle play/pause
    if keyboard.just_pressed(KeyCode::Space) {
        sm.toggle_playback();
        debug!(
            "Playback {}",
            if sm.is_playing { "started" } else { "stopped" }
        );
    }

    // Ctrl+Arrow — jump to start/end of timeline
    if ctrl && keyboard.just_pressed(KeyCode::ArrowLeft) {
        sm.is_playing = false;
        timeline.jump_to_frame(0);
        debug!("⏮ Jump to start via Ctrl+←");
    }
    if ctrl && keyboard.just_pressed(KeyCode::ArrowRight) {
        sm.is_playing = false;
        let last = timeline.total_frames.saturating_sub(1);
        timeline.jump_to_frame(last);
        debug!("⏭ Jump to end via Ctrl+→");
    }

    // Arrow keys — step forward/backward with hold-to-repeat
    let right = !ctrl && keyboard.pressed(KeyCode::ArrowRight);
    let left = !ctrl && keyboard.pressed(KeyCode::ArrowLeft);

    if right || left {
        // Stop auto-playback when manually stepping
        if sm.is_playing {
            sm.is_playing = false;
        }

        if keyboard.just_pressed(KeyCode::ArrowRight) || keyboard.just_pressed(KeyCode::ArrowLeft) {
            // Reset timer and do immediate step
            key_timer.hold_time = 0.0;
            key_timer.last_repeat = 0.0;

            if right {
                timeline.next_frame();
            } else {
                timeline.previous_frame();
            }
        } else {
            // Holding — accumulate time and repeat after delay
            key_timer.hold_time += time.delta_secs();

            if key_timer.hold_time >= REPEAT_DELAY {
                let elapsed_since_repeat = key_timer.hold_time - key_timer.last_repeat;
                if elapsed_since_repeat >= REPEAT_INTERVAL {
                    key_timer.last_repeat = key_timer.hold_time;
                    if right {
                        timeline.next_frame();
                    } else {
                        timeline.previous_frame();
                    }
                }
            }
        }
    } else {
        key_timer.hold_time = 0.0;
        key_timer.last_repeat = 0.0;
    }

    // Home — jump to first frame
    if keyboard.just_pressed(KeyCode::Home) {
        sm.is_playing = false;
        timeline.jump_to_frame(0);
    }

    // End — jump to last frame
    if keyboard.just_pressed(KeyCode::End) {
        sm.is_playing = false;
        let last = timeline.total_frames.saturating_sub(1);
        timeline.jump_to_frame(last);
    }

    // Plus / Minus — adjust speed
    if keyboard.just_pressed(KeyCode::Equal) || keyboard.just_pressed(KeyCode::NumpadAdd) {
        let new_speed = (sm.speed * 1.5).min(5.0);
        sm.set_speed(new_speed);
        debug!("Playback speed: {:.1}x", sm.speed);
    }
    if keyboard.just_pressed(KeyCode::Minus) || keyboard.just_pressed(KeyCode::NumpadSubtract) {
        let new_speed = (sm.speed / 1.5).max(0.1);
        sm.set_speed(new_speed);
        debug!("Playback speed: {:.1}x", sm.speed);
    }

    // Number keys — jump to percentage of sequence
    let digit_keys: [(KeyCode, f32); 10] = [
        (KeyCode::Digit1, 0.0),
        (KeyCode::Digit2, 0.1),
        (KeyCode::Digit3, 0.2),
        (KeyCode::Digit4, 0.3),
        (KeyCode::Digit5, 0.4),
        (KeyCode::Digit6, 0.5),
        (KeyCode::Digit7, 0.6),
        (KeyCode::Digit8, 0.7),
        (KeyCode::Digit9, 0.8),
        (KeyCode::Digit0, 0.9),
    ];

    for (key, pct) in digit_keys {
        if keyboard.just_pressed(key) {
            let frame = (timeline.total_frames as f32 * pct) as usize;
            timeline.jump_to_frame(frame);
        }
    }
}
