//! Input → Action mapping system.
//!
//! Runs in **`PreUpdate`** so that all systems in `Update` and later schedules
//! can read the canonical [`ActionStateRes`] instead of raw `ButtonInput`.
//!
//! ## Design
//!
//! 1. [`InputMap`] holds the (currently hard-coded) mapping from physical keys
//!    and modifier combos to [`Action`]s.
//! 2. [`update_action_state`] reads `ButtonInput<KeyCode>`,
//!    `ButtonInput<MouseButton>`, and the previous frame's egui input state,
//!    builds a new `HashSet<Action>`, and calls [`ActionState::update`].
//! 3. Consumers read `Res<ActionStateRes>` — never raw keys.

use std::collections::HashSet;

use bevy::prelude::*;
use bevy_egui::EguiContexts;

use crate::lib::actions::{Action, ActionState};

// ---------------------------------------------------------------------------
// Resources
// ---------------------------------------------------------------------------

/// Bevy wrapper around the pure [`ActionState`].
#[derive(Resource, Debug, Clone, Default)]
pub struct ActionStateRes(pub ActionState);

/// Hard-coded input map.
///
/// Future work: make this configurable via `seaview.toml`.
#[derive(Resource, Debug, Clone)]
pub struct InputMap {
    /// If `true`, the egui UI is considered to have keyboard focus and
    /// non-global keyboard actions are suppressed.
    ///
    /// Updated each frame from the previous frame's egui context.
    pub egui_wants_keyboard: bool,
}

impl Default for InputMap {
    fn default() -> Self {
        Self {
            egui_wants_keyboard: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// Registers the input-to-action mapping system.
pub struct InputActionPlugin;

impl Plugin for InputActionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActionStateRes>()
            .init_resource::<InputMap>()
            .add_systems(PreUpdate, update_action_state);
    }
}

// ---------------------------------------------------------------------------
// System
// ---------------------------------------------------------------------------

/// Reads raw input, applies the mapping, and writes [`ActionStateRes`].
///
/// Runs in `PreUpdate` — before any gameplay or UI system.
///
/// ### egui integration
///
/// We probe `egui_ctx.wants_keyboard_input()` (which reflects the *previous*
/// frame — egui hasn't run yet this frame). When egui wants keyboard input we
/// only emit **global shortcuts** (Ctrl+S, Ctrl+C, etc.) and suppress movement
/// and other non-modified keys so they don't conflict with text fields.
pub fn update_action_state(
    keyboard: Res<ButtonInput<KeyCode>>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut contexts: EguiContexts,
    mut input_map: ResMut<InputMap>,
    mut action_state: ResMut<ActionStateRes>,
) {
    // -- Probe egui focus from previous frame --------------------------------
    input_map.egui_wants_keyboard = contexts
        .ctx_mut()
        .ok()
        .map_or(false, |ctx| ctx.wants_keyboard_input());

    let egui_kb = input_map.egui_wants_keyboard;

    // -- Modifier state (always available) -----------------------------------
    let ctrl = keyboard.pressed(KeyCode::ControlLeft) || keyboard.pressed(KeyCode::ControlRight);
    let shift = keyboard.pressed(KeyCode::ShiftLeft) || keyboard.pressed(KeyCode::ShiftRight);
    let alt = keyboard.pressed(KeyCode::AltLeft) || keyboard.pressed(KeyCode::AltRight);

    let mut active = HashSet::new();

    // -- Global shortcuts (work even when egui has keyboard focus) -----------
    // These require a modifier so they can't conflict with text input.

    if ctrl && !shift && keyboard.just_pressed(KeyCode::KeyS) {
        active.insert(Action::SaveView);
    }
    if ctrl && !shift && keyboard.just_pressed(KeyCode::KeyC) {
        active.insert(Action::CenterOnMesh);
    }
    if ctrl && shift && keyboard.just_pressed(KeyCode::KeyW) {
        active.insert(Action::ToggleWireframe);
    }
    if ctrl && !shift && keyboard.just_pressed(KeyCode::KeyW) {
        active.insert(Action::ToggleWatch);
    }

    // -- Everything below is suppressed when egui wants keyboard input -------
    if !egui_kb {
        // Movement (only when Ctrl is NOT held — Ctrl+letter = shortcut)
        if !ctrl {
            if keyboard.pressed(KeyCode::KeyW) {
                active.insert(Action::MoveForward);
            }
            if keyboard.pressed(KeyCode::KeyS) {
                active.insert(Action::MoveBackward);
            }
            if keyboard.pressed(KeyCode::KeyA) {
                active.insert(Action::MoveLeft);
            }
            if keyboard.pressed(KeyCode::KeyD) {
                active.insert(Action::MoveRight);
            }
            if keyboard.pressed(KeyCode::KeyE) {
                active.insert(Action::MoveUp);
            }
            if keyboard.pressed(KeyCode::KeyQ) {
                active.insert(Action::MoveDown);
            }

            // Fullscreen (plain F, no modifier)
            if keyboard.just_pressed(KeyCode::KeyF) {
                active.insert(Action::ToggleFullscreen);
            }
        }

        // Speed modifiers
        if alt {
            active.insert(Action::SpeedUltra);
        }
        if shift {
            active.insert(Action::SpeedFast);
        }

        // Playback — plain keys and Ctrl+Arrow combos
        if keyboard.just_pressed(KeyCode::Space) {
            active.insert(Action::PlayPause);
        }

        // Arrow frame-stepping (hold-to-repeat via `pressed`, not `just_pressed`)
        if !ctrl && keyboard.pressed(KeyCode::ArrowRight) {
            active.insert(Action::NextFrame);
        }
        if !ctrl && keyboard.pressed(KeyCode::ArrowLeft) {
            active.insert(Action::PrevFrame);
        }

        // Jump to start / end
        if keyboard.just_pressed(KeyCode::Home)
            || (ctrl && keyboard.just_pressed(KeyCode::ArrowLeft))
        {
            active.insert(Action::JumpToStart);
        }
        if keyboard.just_pressed(KeyCode::End)
            || (ctrl && keyboard.just_pressed(KeyCode::ArrowRight))
        {
            active.insert(Action::JumpToEnd);
        }

        // Speed up / slow down
        if keyboard.just_pressed(KeyCode::Equal) || keyboard.just_pressed(KeyCode::NumpadAdd) {
            active.insert(Action::SpeedUp);
        }
        if keyboard.just_pressed(KeyCode::Minus) || keyboard.just_pressed(KeyCode::NumpadSubtract) {
            active.insert(Action::SlowDown);
        }

        // Cursor grab / release
        if mouse.just_pressed(MouseButton::Left) {
            active.insert(Action::GrabCursor);
        }
        if keyboard.just_pressed(KeyCode::Escape) {
            active.insert(Action::ReleaseCursor);
        }
    }

    action_state.0.update(active);
}
