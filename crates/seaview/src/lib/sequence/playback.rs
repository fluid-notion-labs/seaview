//! Sequence playback keyboard controls
//!
//! Rebuilt as part of playback rebuild (commit 2), migrated to action system
//! (seaview-ihx). Single system: `handle_keyboard_playback` — reads
//! [`ActionStateRes`] for most bindings, with digit-key percentage jumps
//! still on raw `ButtonInput<KeyCode>` (not worth abstracting).

use super::{SequenceManager, Timeline};
use bevy::prelude::*;

use crate::app::input::ActionStateRes;
use crate::lib::actions::Action;

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

/// Action state + raw digits → `Timeline` + `SequenceManager`.
///
/// Frame movement goes through `Timeline`; play/pause and speed go through
/// `SequenceManager`. Most bindings come from [`ActionStateRes`]; digit keys
/// (1–0 = jump to percentage) still read raw `ButtonInput<KeyCode>`.
///
/// **Ctrl+S (SaveView)** is handled by the action system → event bridge in
/// `main.rs`, not here.
pub(super) fn handle_keyboard_playback(
    actions: Res<ActionStateRes>,
    keyboard: Res<ButtonInput<KeyCode>>,
    mut sm: ResMut<SequenceManager>,
    mut timeline: ResMut<Timeline>,
    time: Res<Time>,
    mut key_timer: Local<KeyHoldTimer>,
) {
    // Only process playback input if we have a sequence loaded
    if timeline.total_frames == 0 {
        return;
    }

    // Space bar — toggle play/pause
    if actions.0.just_activated(Action::PlayPause) {
        sm.toggle_playback();
        debug!(
            "Playback {}",
            if sm.is_playing { "started" } else { "stopped" }
        );
    }

    // Jump to start / end of timeline
    if actions.0.just_activated(Action::JumpToStart) {
        sm.is_playing = false;
        timeline.jump_to_frame(0);
        debug!("⏮ Jump to start");
    }
    if actions.0.just_activated(Action::JumpToEnd) {
        sm.is_playing = false;
        let last = timeline.total_frames.saturating_sub(1);
        timeline.jump_to_frame(last);
        debug!("⏭ Jump to end");
    }

    // Arrow keys — step forward/backward with hold-to-repeat.
    // The action system provides `NextFrame` / `PrevFrame` as held actions
    // (mapped from `pressed`, not `just_pressed`), so we implement the
    // repeat-delay logic here using ActionState::active + just_activated.
    let stepping = actions.0.active(Action::NextFrame) || actions.0.active(Action::PrevFrame);

    if stepping {
        // Stop auto-playback when manually stepping
        if sm.is_playing {
            sm.is_playing = false;
        }

        let just_started = actions.0.just_activated(Action::NextFrame)
            || actions.0.just_activated(Action::PrevFrame);

        if just_started {
            // Reset timer and do immediate step
            key_timer.hold_time = 0.0;
            key_timer.last_repeat = 0.0;

            if actions.0.active(Action::NextFrame) {
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
                    if actions.0.active(Action::NextFrame) {
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

    // Plus / Minus — adjust speed
    if actions.0.just_activated(Action::SpeedUp) {
        let new_speed = (sm.speed * 1.5).min(5.0);
        sm.set_speed(new_speed);
        debug!("Playback speed: {:.1}x", sm.speed);
    }
    if actions.0.just_activated(Action::SlowDown) {
        let new_speed = (sm.speed / 1.5).max(0.1);
        sm.set_speed(new_speed);
        debug!("Playback speed: {:.1}x", sm.speed);
    }

    // Number keys — jump to percentage of sequence.
    // These stay on raw ButtonInput because adding 10 Action variants for
    // digit-percentage jumps isn't worth the abstraction cost.
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
