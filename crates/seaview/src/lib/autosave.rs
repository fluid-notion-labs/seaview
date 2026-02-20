//! Autosave state machine — pure logic, no Bevy dependencies.
//!
//! Tracks dirty state and a debounce timer.  When something changes, the
//! caller marks the state dirty.  After a configurable quiet period with no
//! further changes, [`AutosaveState::tick`] returns [`AutosaveAction::Save`].
//!
//! History rotation keeps the last N versions of the settings file on disk
//! so the user can recover from bad saves.
//!
//! ## Usage (app layer)
//!
//! ```ignore
//! // In a system:
//! if camera_transform.is_changed() || lighting.is_changed() { autosave.mark_dirty(); }
//! match autosave.tick(time.delta_secs_f64()) {
//!     AutosaveAction::Save => { /* do the save */ autosave.saved(); }
//!     AutosaveAction::Wait | AutosaveAction::Clean => {}
//! }
//! ```

use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Configuration for the autosave system.
#[derive(Debug, Clone)]
pub struct AutosaveConfig {
    /// Seconds of quiet (no new dirty marks) before triggering a save.
    pub debounce_secs: f64,

    /// Maximum number of history files to keep (e.g. 5 → `.1` through `.5`).
    /// Set to 0 to disable history.
    pub max_history: usize,
}

impl Default for AutosaveConfig {
    fn default() -> Self {
        Self {
            debounce_secs: 2.0,
            max_history: 5,
        }
    }
}

// ---------------------------------------------------------------------------
// State machine
// ---------------------------------------------------------------------------

/// What the autosave system wants the caller to do this tick.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AutosaveAction {
    /// Nothing to do — state is clean (no pending changes).
    Clean,
    /// Changes are pending but the debounce timer hasn't expired yet.
    Wait,
    /// The debounce timer expired — caller should save now, then call
    /// [`AutosaveState::saved`].
    Save,
}

/// Pure state machine for debounced autosave.
#[derive(Debug, Clone)]
pub struct AutosaveState {
    config: AutosaveConfig,

    /// Whether unsaved changes exist.
    dirty: bool,

    /// Wall-clock time (seconds) accumulated since the last `mark_dirty`.
    /// Reset to 0 on every `mark_dirty` call.
    quiet_elapsed: f64,

    /// Total number of saves performed (for diagnostics).
    save_count: u64,

    /// Whether a save is currently pending (dirty + timer expired but
    /// `saved()` hasn't been called yet).  Prevents re-firing `Save`
    /// on subsequent ticks.
    pending_save: bool,
}

impl AutosaveState {
    /// Create a new autosave state with the given configuration.
    pub fn new(config: AutosaveConfig) -> Self {
        Self {
            config,
            dirty: false,
            quiet_elapsed: 0.0,
            save_count: 0,
            pending_save: false,
        }
    }

    /// Create with default configuration.
    pub fn with_defaults() -> Self {
        Self::new(AutosaveConfig::default())
    }

    /// Mark the state as dirty (something changed).
    ///
    /// Resets the debounce timer so the save is deferred until the user
    /// stops making changes.
    pub fn mark_dirty(&mut self) {
        self.dirty = true;
        self.quiet_elapsed = 0.0;
        self.pending_save = false; // reset pending — timer restarted
    }

    /// Advance the timer and return the action the caller should take.
    ///
    /// Call this once per frame with the frame's delta time in seconds.
    pub fn tick(&mut self, delta_secs: f64) -> AutosaveAction {
        if !self.dirty {
            return AutosaveAction::Clean;
        }

        if self.pending_save {
            // Already told the caller to save — waiting for `saved()`.
            return AutosaveAction::Save;
        }

        self.quiet_elapsed += delta_secs;

        if self.quiet_elapsed >= self.config.debounce_secs {
            self.pending_save = true;
            AutosaveAction::Save
        } else {
            AutosaveAction::Wait
        }
    }

    /// Acknowledge that the save completed.
    ///
    /// Resets the dirty flag and counters.  Must be called after a
    /// successful save to prevent repeated `Save` actions.
    pub fn saved(&mut self) {
        self.dirty = false;
        self.quiet_elapsed = 0.0;
        self.pending_save = false;
        self.save_count += 1;
    }

    /// Whether there are unsaved changes.
    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    /// Total number of completed saves.
    pub fn save_count(&self) -> u64 {
        self.save_count
    }

    /// The current debounce configuration.
    pub fn config(&self) -> &AutosaveConfig {
        &self.config
    }

    /// Force an immediate save on the next tick, ignoring the debounce timer.
    ///
    /// Used by the "save now" UI button.
    pub fn save_now(&mut self) {
        if !self.dirty {
            // Mark dirty so the next tick returns Save.
            self.dirty = true;
        }
        self.quiet_elapsed = self.config.debounce_secs;
        self.pending_save = false; // let tick() set it
    }
}

// ---------------------------------------------------------------------------
// History rotation
// ---------------------------------------------------------------------------

/// Rotate history files before a save.
///
/// Given a base path like `/data/seaview.toml` and `max_history = 5`:
///
/// 1. Delete `seaview.toml.5` if it exists
/// 2. Rename `seaview.toml.4` → `seaview.toml.5`
/// 3. Rename `seaview.toml.3` → `seaview.toml.4`
/// 4. … and so on …
/// 5. Copy `seaview.toml` → `seaview.toml.1`
///
/// The current file is **copied** (not renamed) so it stays in place for
/// the caller to overwrite.  This avoids any window where the file doesn't
/// exist.
///
/// Returns `Ok(rotated_count)` — the number of files that were actually
/// moved or copied.  Returns `Ok(0)` if `max_history == 0` or the base
/// file doesn't exist.
pub fn rotate_history(base_path: &Path, max_history: usize) -> std::io::Result<usize> {
    if max_history == 0 {
        return Ok(0);
    }

    if !base_path.exists() {
        return Ok(0);
    }

    let mut count = 0usize;

    // Delete the oldest if it exists.
    let oldest = history_path(base_path, max_history);
    if oldest.exists() {
        fs::remove_file(&oldest)?;
    }

    // Shift N-1 → N, N-2 → N-1, … , 1 → 2
    for i in (1..max_history).rev() {
        let from = history_path(base_path, i);
        let to = history_path(base_path, i + 1);
        if from.exists() {
            fs::rename(&from, &to)?;
            count += 1;
        }
    }

    // Copy current → .1 (copy, not rename, so the file stays for overwrite)
    let first = history_path(base_path, 1);
    fs::copy(base_path, &first)?;
    count += 1;

    Ok(count)
}

/// Build a history path: `base.ext` → `base.ext.N`
fn history_path(base: &Path, n: usize) -> PathBuf {
    let mut p = base.as_os_str().to_owned();
    p.push(format!(".{n}"));
    PathBuf::from(p)
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // -- AutosaveState -------------------------------------------------------

    #[test]
    fn starts_clean() {
        let state = AutosaveState::with_defaults();
        assert!(!state.is_dirty());
        assert_eq!(state.save_count(), 0);
    }

    #[test]
    fn clean_tick_returns_clean() {
        let mut state = AutosaveState::with_defaults();
        assert_eq!(state.tick(1.0), AutosaveAction::Clean);
    }

    #[test]
    fn dirty_then_wait() {
        let mut state = AutosaveState::new(AutosaveConfig {
            debounce_secs: 2.0,
            max_history: 0,
        });
        state.mark_dirty();
        assert!(state.is_dirty());

        // 1 second — still waiting
        assert_eq!(state.tick(1.0), AutosaveAction::Wait);
    }

    #[test]
    fn dirty_then_save_after_debounce() {
        let mut state = AutosaveState::new(AutosaveConfig {
            debounce_secs: 2.0,
            max_history: 0,
        });
        state.mark_dirty();

        assert_eq!(state.tick(1.0), AutosaveAction::Wait);
        assert_eq!(state.tick(1.5), AutosaveAction::Save);

        // Still returns Save until saved() is called
        assert_eq!(state.tick(0.1), AutosaveAction::Save);
    }

    #[test]
    fn saved_resets_state() {
        let mut state = AutosaveState::new(AutosaveConfig {
            debounce_secs: 1.0,
            max_history: 0,
        });
        state.mark_dirty();
        assert_eq!(state.tick(2.0), AutosaveAction::Save);

        state.saved();
        assert!(!state.is_dirty());
        assert_eq!(state.save_count(), 1);
        assert_eq!(state.tick(0.1), AutosaveAction::Clean);
    }

    #[test]
    fn re_dirty_resets_debounce() {
        let mut state = AutosaveState::new(AutosaveConfig {
            debounce_secs: 2.0,
            max_history: 0,
        });
        state.mark_dirty();
        assert_eq!(state.tick(1.5), AutosaveAction::Wait);

        // User makes another change — timer resets
        state.mark_dirty();
        assert_eq!(state.tick(1.5), AutosaveAction::Wait);
        assert_eq!(state.tick(1.0), AutosaveAction::Save);
    }

    #[test]
    fn save_now_skips_debounce() {
        let mut state = AutosaveState::new(AutosaveConfig {
            debounce_secs: 10.0,
            max_history: 0,
        });
        state.mark_dirty();
        assert_eq!(state.tick(0.1), AutosaveAction::Wait);

        state.save_now();
        assert_eq!(state.tick(0.0), AutosaveAction::Save);
    }

    #[test]
    fn save_now_when_clean_marks_dirty() {
        let mut state = AutosaveState::new(AutosaveConfig {
            debounce_secs: 10.0,
            max_history: 0,
        });
        // Not dirty — save_now should still trigger
        state.save_now();
        assert_eq!(state.tick(0.0), AutosaveAction::Save);
    }

    #[test]
    fn multiple_save_cycles() {
        let mut state = AutosaveState::new(AutosaveConfig {
            debounce_secs: 0.5,
            max_history: 0,
        });

        // Cycle 1
        state.mark_dirty();
        assert_eq!(state.tick(1.0), AutosaveAction::Save);
        state.saved();
        assert_eq!(state.save_count(), 1);

        // Cycle 2
        state.mark_dirty();
        assert_eq!(state.tick(1.0), AutosaveAction::Save);
        state.saved();
        assert_eq!(state.save_count(), 2);

        assert_eq!(state.tick(1.0), AutosaveAction::Clean);
    }

    // -- History rotation ----------------------------------------------------

    #[test]
    fn history_path_format() {
        let base = Path::new("/data/seaview.toml");
        assert_eq!(history_path(base, 1), PathBuf::from("/data/seaview.toml.1"));
        assert_eq!(history_path(base, 5), PathBuf::from("/data/seaview.toml.5"));
    }

    #[test]
    fn rotate_zero_history_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("seaview.toml");
        fs::write(&base, "original").unwrap();

        let count = rotate_history(&base, 0).unwrap();
        assert_eq!(count, 0);
        assert_eq!(fs::read_to_string(&base).unwrap(), "original");
    }

    #[test]
    fn rotate_no_file_is_noop() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("seaview.toml");
        // File doesn't exist
        let count = rotate_history(&base, 5).unwrap();
        assert_eq!(count, 0);
    }

    #[test]
    fn rotate_first_save() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("seaview.toml");
        fs::write(&base, "v1").unwrap();

        let count = rotate_history(&base, 3).unwrap();
        assert_eq!(count, 1); // copy to .1

        // .1 should exist with original content
        assert_eq!(
            fs::read_to_string(dir.path().join("seaview.toml.1")).unwrap(),
            "v1"
        );
        // Original still exists (was copied, not renamed)
        assert!(base.exists());
    }

    #[test]
    fn rotate_multiple_saves() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("seaview.toml");

        // Save 1
        fs::write(&base, "v1").unwrap();
        rotate_history(&base, 3).unwrap();
        fs::write(&base, "v2").unwrap();

        // Save 2
        rotate_history(&base, 3).unwrap();
        fs::write(&base, "v3").unwrap();

        // Save 3
        rotate_history(&base, 3).unwrap();
        fs::write(&base, "v4").unwrap();

        // .1 = v3 (most recent backup)
        // .2 = v2
        // .3 = v1 (oldest)
        assert_eq!(
            fs::read_to_string(dir.path().join("seaview.toml.1")).unwrap(),
            "v3"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("seaview.toml.2")).unwrap(),
            "v2"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("seaview.toml.3")).unwrap(),
            "v1"
        );
    }

    #[test]
    fn rotate_evicts_oldest() {
        let dir = tempfile::tempdir().unwrap();
        let base = dir.path().join("seaview.toml");

        // Fill up history (max_history = 2)
        fs::write(&base, "v1").unwrap();
        rotate_history(&base, 2).unwrap();
        fs::write(&base, "v2").unwrap();
        rotate_history(&base, 2).unwrap();
        fs::write(&base, "v3").unwrap();
        rotate_history(&base, 2).unwrap();
        fs::write(&base, "v4").unwrap();

        // Only .1 and .2 should exist
        assert!(dir.path().join("seaview.toml.1").exists());
        assert!(dir.path().join("seaview.toml.2").exists());
        assert!(!dir.path().join("seaview.toml.3").exists());

        // .1 = v3, .2 = v2 (v1 was evicted)
        assert_eq!(
            fs::read_to_string(dir.path().join("seaview.toml.1")).unwrap(),
            "v3"
        );
        assert_eq!(
            fs::read_to_string(dir.path().join("seaview.toml.2")).unwrap(),
            "v2"
        );
    }
}
