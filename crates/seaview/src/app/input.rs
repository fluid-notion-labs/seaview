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
//!
//! ## Diagnostic logging
//!
//! When `TRACE` level is enabled, the system logs every frame where a
//! modifier shortcut fires or movement is suppressed, helping diagnose
//! input conflicts.
//!
//! ## Modifier-key suppression
//!
//! When a modifier shortcut fires (e.g. Ctrl+S → SaveView), the base key (S)
//! is added to a **consumed set**.  While a key is in the consumed set, it is
//! suppressed from producing its unmodified action (MoveBackward).  The key is
//! removed from the consumed set only after it is physically released.
//!
//! This prevents the camera-jump bug where releasing Ctrl before S causes a
//! brief MoveBackward on the frame(s) where S is still held without Ctrl.

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

    /// Keys that were consumed by a modifier shortcut and must be suppressed
    /// until physically released.  Prevents the base key from firing its
    /// unmodified action when the modifier is released first.
    consumed_keys: HashSet<KeyCode>,
}

impl Default for InputMap {
    fn default() -> Self {
        Self {
            egui_wants_keyboard: false,
            consumed_keys: HashSet::new(),
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
///
/// ### Modifier suppression
///
/// When a modifier shortcut fires (e.g. Ctrl+S), the base key is added to
/// `consumed_keys`.  While consumed, the key cannot produce its unmodified
/// action.  The key is removed from `consumed_keys` only when it is physically
/// released.  This eliminates the camera-jump bug caused by releasing Ctrl
/// before the base key.
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

    // -- Drain released keys from the consumed set ---------------------------
    input_map
        .consumed_keys
        .retain(|key| !keyboard.just_released(*key));

    // -- Modifier state (always available) -----------------------------------
    let ctrl = keyboard.pressed(KeyCode::ControlLeft) || keyboard.pressed(KeyCode::ControlRight);
    let shift = keyboard.pressed(KeyCode::ShiftLeft) || keyboard.pressed(KeyCode::ShiftRight);
    let alt = keyboard.pressed(KeyCode::AltLeft) || keyboard.pressed(KeyCode::AltRight);

    let mut active = HashSet::new();

    // Track whether any modifier shortcut fired this frame so we can
    // suppress movement even if the consumed_keys drain hasn't caught up yet.
    let mut shortcut_fired = false;

    // -- Global shortcuts (work even when egui has keyboard focus) -----------
    // These require a modifier so they can't conflict with text input.

    // Ctrl+S removed — autosave handles persistence automatically.
    // Keeping S out of the shortcut path so it never conflicts with
    // MoveBackward.
    if ctrl && !shift && keyboard.just_pressed(KeyCode::KeyC) {
        active.insert(Action::CenterOnMesh);
        input_map.consumed_keys.insert(KeyCode::KeyC);
        shortcut_fired = true;
    }
    if ctrl && shift && keyboard.just_pressed(KeyCode::KeyW) {
        active.insert(Action::ToggleWireframe);
        input_map.consumed_keys.insert(KeyCode::KeyW);
        shortcut_fired = true;
    }
    if ctrl && !shift && keyboard.just_pressed(KeyCode::KeyW) {
        active.insert(Action::ToggleWatch);
        input_map.consumed_keys.insert(KeyCode::KeyW);
        shortcut_fired = true;
    }
    if ctrl && keyboard.just_pressed(KeyCode::PageDown) {
        active.insert(Action::NextSeries);
        shortcut_fired = true;
    }
    if ctrl && keyboard.just_pressed(KeyCode::PageUp) {
        active.insert(Action::PrevSeries);
        shortcut_fired = true;
    }

    // -- Everything below is suppressed when egui wants keyboard input -------
    if !egui_kb {
        // Movement (suppressed when Ctrl is held OR when a shortcut just fired
        // OR when the key is in the consumed set from a prior shortcut).
        let suppress_movement = ctrl || shortcut_fired;

        if !suppress_movement {
            if keyboard.pressed(KeyCode::KeyW) && !input_map.consumed_keys.contains(&KeyCode::KeyW)
            {
                active.insert(Action::MoveForward);
            }
            if keyboard.pressed(KeyCode::KeyS) && !input_map.consumed_keys.contains(&KeyCode::KeyS)
            {
                active.insert(Action::MoveBackward);
            }
            if keyboard.pressed(KeyCode::KeyA) && !input_map.consumed_keys.contains(&KeyCode::KeyA)
            {
                active.insert(Action::MoveLeft);
            }
            if keyboard.pressed(KeyCode::KeyD) && !input_map.consumed_keys.contains(&KeyCode::KeyD)
            {
                active.insert(Action::MoveRight);
            }
            if keyboard.pressed(KeyCode::KeyE) && !input_map.consumed_keys.contains(&KeyCode::KeyE)
            {
                active.insert(Action::MoveUp);
            }
            if keyboard.pressed(KeyCode::KeyQ) && !input_map.consumed_keys.contains(&KeyCode::KeyQ)
            {
                active.insert(Action::MoveDown);
            }

            // Fullscreen (plain F, no modifier)
            if keyboard.just_pressed(KeyCode::KeyF) {
                active.insert(Action::ToggleFullscreen);
            }
        }

        // Speed modifiers — these are fine even during shortcuts since they
        // only affect movement magnitude (which is zero when suppressed).
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
        if keyboard.just_pressed(KeyCode::Minus) || keyboard.just_pressed(KeyCode::NumpadSubtract)
        {
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
