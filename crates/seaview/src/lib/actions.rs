//! Input action model — pure data, no Bevy dependencies.
//!
//! Defines the canonical set of user actions and a frame-aware state tracker.
//! Systems in `app/` map raw input (keyboard, mouse, UI buttons) into
//! [`ActionState`], and all gameplay/UI systems read actions instead of raw keys.

use std::collections::HashSet;

// ---------------------------------------------------------------------------
// Action enum
// ---------------------------------------------------------------------------

/// Every discrete user action the application understands.
///
/// Actions are abstract — they carry no information about *which* key or button
/// produced them. The mapping from physical input to action lives in
/// [`super::super::app::input::InputMap`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Action {
    // ── Camera movement ────────────────────────────────────────────
    /// Move camera forward (default: W).
    MoveForward,
    /// Move camera backward (default: S).
    MoveBackward,
    /// Strafe left (default: A).
    MoveLeft,
    /// Strafe right (default: D).
    MoveRight,
    /// Move camera up (default: E).
    MoveUp,
    /// Move camera down (default: Q).
    MoveDown,

    // ── Speed modifiers ────────────────────────────────────────────
    /// Fast movement multiplier (default: Shift).
    SpeedFast,
    /// Ultra-fast movement multiplier (default: Alt).
    SpeedUltra,

    // ── Camera control ─────────────────────────────────────────────
    /// Grab the cursor for mouse-look (default: Left Click).
    GrabCursor,
    /// Release the cursor (default: Escape).
    ReleaseCursor,
    /// Center camera on mesh AABB (default: Ctrl+C).
    CenterOnMesh,

    // ── Playback ───────────────────────────────────────────────────
    /// Toggle play / pause (default: Space).
    PlayPause,
    /// Step one frame forward (default: Right Arrow, hold-to-repeat).
    NextFrame,
    /// Step one frame backward (default: Left Arrow, hold-to-repeat).
    PrevFrame,
    /// Jump to first frame (default: Home or Ctrl+Left).
    JumpToStart,
    /// Jump to last frame (default: End or Ctrl+Right).
    JumpToEnd,
    /// Increase playback speed (default: + / NumpadAdd).
    SpeedUp,
    /// Decrease playback speed (default: - / NumpadSubtract).
    SlowDown,

    // ── Global shortcuts ───────────────────────────────────────────
    /// Save the current view to seaview.toml (default: Ctrl+S).
    SaveView,
    /// Toggle borderless fullscreen (default: F).
    ToggleFullscreen,
    /// Toggle wireframe overlay (default: Ctrl+Shift+W).
    ToggleWireframe,
    /// Toggle directory watch mode (default: Ctrl+W).
    ToggleWatch,

    // ── Series switching ───────────────────────────────────────────
    /// Switch to the next loaded series (default: Ctrl+PageDown).
    NextSeries,
    /// Switch to the previous loaded series (default: Ctrl+PageUp).
    PrevSeries,
}

impl Action {
    /// Returns `true` for actions that represent a continuous hold rather than
    /// a one-shot trigger. Systems should read [`ActionState::active`] for
    /// held actions and [`ActionState::just_activated`] for one-shot actions.
    pub fn is_held(&self) -> bool {
        matches!(
            self,
            Action::MoveForward
                | Action::MoveBackward
                | Action::MoveLeft
                | Action::MoveRight
                | Action::MoveUp
                | Action::MoveDown
                | Action::SpeedFast
                | Action::SpeedUltra
                | Action::NextFrame
                | Action::PrevFrame
        )
    }
}

// ---------------------------------------------------------------------------
// ActionState
// ---------------------------------------------------------------------------

/// Frame-aware action state tracker.
///
/// Each frame the input system builds a new set of active actions and calls
/// [`ActionState::update`] to compute the just-activated / just-deactivated
/// deltas.
#[derive(Debug, Clone, Default)]
pub struct ActionState {
    /// Actions that are currently active (key held / button down).
    active: HashSet<Action>,
    /// Actions that became active *this* frame.
    just_activated: HashSet<Action>,
    /// Actions that stopped being active *this* frame.
    just_deactivated: HashSet<Action>,
}

impl ActionState {
    /// Create an empty action state.
    pub fn new() -> Self {
        Self::default()
    }

    /// Update the state with a new set of active actions.
    ///
    /// Computes `just_activated` (in `new` but not in previous `active`) and
    /// `just_deactivated` (in previous `active` but not in `new`).
    pub fn update(&mut self, new_active: HashSet<Action>) {
        self.just_activated = new_active.difference(&self.active).copied().collect();
        self.just_deactivated = self.active.difference(&new_active).copied().collect();
        self.active = new_active;
    }

    /// Clear all state (no actions active, no transitions).
    pub fn clear(&mut self) {
        self.active.clear();
        self.just_activated.clear();
        self.just_deactivated.clear();
    }

    // -- Queries -------------------------------------------------------------

    /// Is the action currently active (held)?
    pub fn active(&self, action: Action) -> bool {
        self.active.contains(&action)
    }

    /// Did the action become active this frame?
    pub fn just_activated(&self, action: Action) -> bool {
        self.just_activated.contains(&action)
    }

    /// Did the action stop being active this frame?
    pub fn just_deactivated(&self, action: Action) -> bool {
        self.just_deactivated.contains(&action)
    }

    // -- Derived queries -----------------------------------------------------

    /// Is any movement action currently active?
    pub fn is_moving(&self) -> bool {
        self.active(Action::MoveForward)
            || self.active(Action::MoveBackward)
            || self.active(Action::MoveLeft)
            || self.active(Action::MoveRight)
            || self.active(Action::MoveUp)
            || self.active(Action::MoveDown)
    }

    /// Compute a normalised movement vector from active movement actions.
    ///
    /// Returns `(right, up, forward)` with each axis in `−1.0..=1.0`.
    /// The caller is responsible for transforming this into world space
    /// using the camera's orientation.
    ///
    /// The vector is **not** normalised to unit length — the caller should
    /// normalise if needed (to avoid √2 diagonal speed).
    pub fn movement_vector(&self) -> (f32, f32, f32) {
        let mut right: f32 = 0.0;
        let mut up: f32 = 0.0;
        let mut forward: f32 = 0.0;

        if self.active(Action::MoveForward) {
            forward += 1.0;
        }
        if self.active(Action::MoveBackward) {
            forward -= 1.0;
        }
        if self.active(Action::MoveRight) {
            right += 1.0;
        }
        if self.active(Action::MoveLeft) {
            right -= 1.0;
        }
        if self.active(Action::MoveUp) {
            up += 1.0;
        }
        if self.active(Action::MoveDown) {
            up -= 1.0;
        }

        (right, up, forward)
    }

    /// The active speed modifier multiplier.
    ///
    /// - [`Action::SpeedUltra`] → 25.0
    /// - [`Action::SpeedFast`] → 5.0
    /// - Neither → 1.0
    ///
    /// Ultra takes precedence over fast.
    pub fn speed_multiplier(&self) -> f32 {
        if self.active(Action::SpeedUltra) {
            25.0
        } else if self.active(Action::SpeedFast) {
            5.0
        } else {
            1.0
        }
    }

    // -- Introspection -------------------------------------------------------

    /// Number of currently active actions.
    pub fn active_count(&self) -> usize {
        self.active.len()
    }

    /// Iterate over currently active actions.
    pub fn active_iter(&self) -> impl Iterator<Item = &Action> {
        self.active.iter()
    }

    /// Iterate over actions that just activated this frame.
    pub fn just_activated_iter(&self) -> impl Iterator<Item = &Action> {
        self.just_activated.iter()
    }

    /// Iterate over actions that just deactivated this frame.
    pub fn just_deactivated_iter(&self) -> impl Iterator<Item = &Action> {
        self.just_deactivated.iter()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn set(actions: &[Action]) -> HashSet<Action> {
        actions.iter().copied().collect()
    }

    // -- ActionState::update -------------------------------------------------

    #[test]
    fn empty_to_empty() {
        let mut state = ActionState::new();
        state.update(HashSet::new());
        assert_eq!(state.active_count(), 0);
        assert_eq!(state.just_activated.len(), 0);
        assert_eq!(state.just_deactivated.len(), 0);
    }

    #[test]
    fn activate_actions() {
        let mut state = ActionState::new();
        state.update(set(&[Action::MoveForward, Action::SpeedFast]));

        assert!(state.active(Action::MoveForward));
        assert!(state.active(Action::SpeedFast));
        assert!(state.just_activated(Action::MoveForward));
        assert!(state.just_activated(Action::SpeedFast));
        assert!(!state.just_deactivated(Action::MoveForward));
    }

    #[test]
    fn deactivate_actions() {
        let mut state = ActionState::new();
        state.update(set(&[Action::MoveForward]));
        // Next frame: MoveForward released
        state.update(HashSet::new());

        assert!(!state.active(Action::MoveForward));
        assert!(!state.just_activated(Action::MoveForward));
        assert!(state.just_deactivated(Action::MoveForward));
    }

    #[test]
    fn held_action_not_re_activated() {
        let mut state = ActionState::new();
        state.update(set(&[Action::MoveForward]));
        // Still held next frame
        state.update(set(&[Action::MoveForward]));

        assert!(state.active(Action::MoveForward));
        assert!(
            !state.just_activated(Action::MoveForward),
            "should not be just_activated when held across frames"
        );
        assert!(!state.just_deactivated(Action::MoveForward));
    }

    #[test]
    fn mixed_activate_deactivate() {
        let mut state = ActionState::new();
        state.update(set(&[Action::MoveForward, Action::MoveLeft]));
        // Next frame: release MoveLeft, press MoveRight
        state.update(set(&[Action::MoveForward, Action::MoveRight]));

        assert!(state.active(Action::MoveForward));
        assert!(!state.active(Action::MoveLeft));
        assert!(state.active(Action::MoveRight));

        assert!(!state.just_activated(Action::MoveForward)); // held
        assert!(state.just_activated(Action::MoveRight)); // new
        assert!(state.just_deactivated(Action::MoveLeft)); // released
    }

    #[test]
    fn clear_resets_everything() {
        let mut state = ActionState::new();
        state.update(set(&[Action::SaveView, Action::MoveForward]));
        state.clear();

        assert_eq!(state.active_count(), 0);
        assert!(!state.active(Action::SaveView));
        assert!(!state.just_activated(Action::SaveView));
    }

    // -- Derived queries -----------------------------------------------------

    #[test]
    fn is_moving() {
        let mut state = ActionState::new();
        assert!(!state.is_moving());

        state.update(set(&[Action::SpeedFast]));
        assert!(!state.is_moving(), "speed modifier alone is not moving");

        state.update(set(&[Action::MoveUp]));
        assert!(state.is_moving());

        state.update(set(&[Action::MoveForward, Action::MoveLeft]));
        assert!(state.is_moving());
    }

    #[test]
    fn movement_vector_forward_right() {
        let mut state = ActionState::new();
        state.update(set(&[Action::MoveForward, Action::MoveRight]));

        let (right, up, forward) = state.movement_vector();
        assert!((forward - 1.0).abs() < f32::EPSILON);
        assert!((right - 1.0).abs() < f32::EPSILON);
        assert!((up - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn movement_vector_cancel_out() {
        let mut state = ActionState::new();
        state.update(set(&[Action::MoveForward, Action::MoveBackward]));

        let (_, _, forward) = state.movement_vector();
        assert!(
            forward.abs() < f32::EPSILON,
            "opposing directions should cancel"
        );
    }

    #[test]
    fn movement_vector_empty() {
        let state = ActionState::new();
        let (r, u, f) = state.movement_vector();
        assert!((r).abs() < f32::EPSILON);
        assert!((u).abs() < f32::EPSILON);
        assert!((f).abs() < f32::EPSILON);
    }

    #[test]
    fn speed_multiplier_default() {
        let state = ActionState::new();
        assert!((state.speed_multiplier() - 1.0).abs() < f32::EPSILON);
    }

    #[test]
    fn speed_multiplier_fast() {
        let mut state = ActionState::new();
        state.update(set(&[Action::SpeedFast]));
        assert!((state.speed_multiplier() - 5.0).abs() < f32::EPSILON);
    }

    #[test]
    fn speed_multiplier_ultra() {
        let mut state = ActionState::new();
        state.update(set(&[Action::SpeedUltra]));
        assert!((state.speed_multiplier() - 25.0).abs() < f32::EPSILON);
    }

    #[test]
    fn speed_multiplier_ultra_takes_precedence() {
        let mut state = ActionState::new();
        state.update(set(&[Action::SpeedFast, Action::SpeedUltra]));
        assert!(
            (state.speed_multiplier() - 25.0).abs() < f32::EPSILON,
            "ultra should win when both held"
        );
    }

    // -- Action::is_held -----------------------------------------------------

    #[test]
    fn held_actions() {
        assert!(Action::MoveForward.is_held());
        assert!(Action::NextFrame.is_held());
        assert!(Action::SpeedFast.is_held());
    }

    #[test]
    fn non_held_actions() {
        assert!(!Action::SaveView.is_held());
        assert!(!Action::PlayPause.is_held());
        assert!(!Action::ToggleFullscreen.is_held());
        assert!(!Action::CenterOnMesh.is_held());
        assert!(!Action::NextSeries.is_held());
        assert!(!Action::PrevSeries.is_held());
    }

    // -- Iterators -----------------------------------------------------------

    #[test]
    fn iterators() {
        let mut state = ActionState::new();
        state.update(set(&[Action::MoveForward, Action::SaveView]));

        let active: HashSet<_> = state.active_iter().copied().collect();
        assert_eq!(active.len(), 2);
        assert!(active.contains(&Action::MoveForward));
        assert!(active.contains(&Action::SaveView));

        let just_act: HashSet<_> = state.just_activated_iter().copied().collect();
        assert_eq!(just_act.len(), 2);

        // Next frame: release SaveView
        state.update(set(&[Action::MoveForward]));
        let just_deact: HashSet<_> = state.just_deactivated_iter().copied().collect();
        assert_eq!(just_deact.len(), 1);
        assert!(just_deact.contains(&Action::SaveView));
    }
}
