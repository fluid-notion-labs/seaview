//! Toast notification data model
//!
//! Pure data types and logic for managing toast notifications.
//! No Bevy system signatures — just plain structs and methods.

use std::collections::VecDeque;

/// Severity level for a toast notification
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ToastLevel {
    /// Informational message (blue)
    Info,
    /// Success confirmation (green)
    Success,
    /// Warning that doesn't block workflow (yellow/orange)
    Warning,
    /// Error that needs attention (red)
    Error,
}

impl ToastLevel {
    /// Icon character for this level
    pub fn icon(&self) -> &'static str {
        match self {
            ToastLevel::Info => "ℹ",
            ToastLevel::Success => "✓",
            ToastLevel::Warning => "⚠",
            ToastLevel::Error => "✗",
        }
    }

    /// Human-readable label
    pub fn label(&self) -> &'static str {
        match self {
            ToastLevel::Info => "Info",
            ToastLevel::Success => "Success",
            ToastLevel::Warning => "Warning",
            ToastLevel::Error => "Error",
        }
    }
}

/// A single toast notification
#[derive(Debug, Clone)]
pub struct Toast {
    /// The message to display
    pub message: String,
    /// Severity level
    pub level: ToastLevel,
    /// Time the toast was created (seconds since app start, from `Time::elapsed_secs_f64`)
    pub created_at: f64,
    /// How long the toast should remain visible (seconds)
    pub duration: f64,
    /// Whether the user has manually dismissed this toast
    pub dismissed: bool,
}

impl Toast {
    /// Create a new toast with the given level and message.
    /// Uses `now` (elapsed seconds) as the creation time.
    pub fn new(level: ToastLevel, message: impl Into<String>, now: f64) -> Self {
        let duration = match level {
            ToastLevel::Info => 3.0,
            ToastLevel::Success => 3.0,
            ToastLevel::Warning => 5.0,
            ToastLevel::Error => 8.0,
        };
        Self {
            message: message.into(),
            level,
            created_at: now,
            duration,
            dismissed: false,
        }
    }

    /// Create a toast with a custom duration (seconds).
    pub fn with_duration(mut self, duration: f64) -> Self {
        self.duration = duration;
        self
    }

    /// How far through its lifetime this toast is (0.0 = just created, 1.0 = expired).
    pub fn progress(&self, now: f64) -> f64 {
        let elapsed = now - self.created_at;
        (elapsed / self.duration).clamp(0.0, 1.0)
    }

    /// Whether the toast has expired (past its duration) or been dismissed.
    pub fn is_expired(&self, now: f64) -> bool {
        self.dismissed || (now - self.created_at) >= self.duration
    }

    /// Opacity for fade-out. Full opacity for most of the lifetime,
    /// then fades during the last 20%.
    pub fn opacity(&self, now: f64) -> f32 {
        if self.dismissed {
            return 0.0;
        }
        let p = self.progress(now);
        if p < 0.8 {
            1.0
        } else {
            // Linear fade from 1.0 at p=0.8 to 0.0 at p=1.0
            ((1.0 - p) / 0.2) as f32
        }
    }
}

/// Default maximum number of toasts kept in history.
const DEFAULT_MAX_HISTORY: usize = 100;

/// Default maximum number of toasts visible simultaneously.
const DEFAULT_MAX_VISIBLE: usize = 5;

/// Manages active toasts and a rolling history log.
#[derive(Debug, Clone)]
pub struct ToastQueue {
    /// Currently active (visible) toasts, newest at back.
    active: VecDeque<Toast>,
    /// History of all past toasts (including expired), newest at back.
    history: VecDeque<Toast>,
    /// Maximum number of toasts to keep in history.
    pub max_history: usize,
    /// Maximum number of toasts visible at once.
    pub max_visible: usize,
    /// Whether the history panel is open.
    pub show_history: bool,
}

impl Default for ToastQueue {
    fn default() -> Self {
        Self {
            active: VecDeque::new(),
            history: VecDeque::new(),
            max_history: DEFAULT_MAX_HISTORY,
            max_visible: DEFAULT_MAX_VISIBLE,
            show_history: false,
        }
    }
}

impl ToastQueue {
    /// Push a new toast onto the queue.
    pub fn push(&mut self, toast: Toast) {
        // Also record in history immediately
        self.history.push_back(toast.clone());
        if self.history.len() > self.max_history {
            self.history.pop_front();
        }

        self.active.push_back(toast);

        // If we exceed max visible, dismiss the oldest
        while self.active.len() > self.max_visible {
            self.active.pop_front();
        }
    }

    /// Convenience: push an info toast.
    pub fn info(&mut self, message: impl Into<String>, now: f64) {
        self.push(Toast::new(ToastLevel::Info, message, now));
    }

    /// Convenience: push a success toast.
    pub fn success(&mut self, message: impl Into<String>, now: f64) {
        self.push(Toast::new(ToastLevel::Success, message, now));
    }

    /// Convenience: push a warning toast.
    pub fn warning(&mut self, message: impl Into<String>, now: f64) {
        self.push(Toast::new(ToastLevel::Warning, message, now));
    }

    /// Convenience: push an error toast.
    pub fn error(&mut self, message: impl Into<String>, now: f64) {
        self.push(Toast::new(ToastLevel::Error, message, now));
    }

    /// Remove all expired toasts from the active list.
    /// Call this once per frame with the current time.
    pub fn drain_expired(&mut self, now: f64) {
        self.active.retain(|t| !t.is_expired(now));
    }

    /// Dismiss the toast at the given index in the active list.
    pub fn dismiss(&mut self, index: usize) {
        if let Some(toast) = self.active.get_mut(index) {
            toast.dismissed = true;
        }
    }

    /// Dismiss all active toasts.
    pub fn dismiss_all(&mut self) {
        for toast in self.active.iter_mut() {
            toast.dismissed = true;
        }
    }

    /// Currently active (visible) toasts, oldest first.
    pub fn active(&self) -> &VecDeque<Toast> {
        &self.active
    }

    /// Number of active toasts.
    pub fn active_count(&self) -> usize {
        self.active.len()
    }

    /// Full history, oldest first.
    pub fn history(&self) -> &VecDeque<Toast> {
        &self.history
    }

    /// Number of history entries.
    pub fn history_count(&self) -> usize {
        self.history.len()
    }

    /// Clear all history.
    pub fn clear_history(&mut self) {
        self.history.clear();
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn toast_expiry() {
        let t = Toast::new(ToastLevel::Info, "hello", 0.0);
        assert!(!t.is_expired(1.0));
        assert!(!t.is_expired(2.9));
        assert!(t.is_expired(3.0));
        assert!(t.is_expired(10.0));
    }

    #[test]
    fn toast_progress() {
        let t = Toast::new(ToastLevel::Info, "hello", 10.0);
        assert!((t.progress(10.0) - 0.0).abs() < 1e-6);
        assert!((t.progress(11.5) - 0.5).abs() < 1e-6);
        assert!((t.progress(13.0) - 1.0).abs() < 1e-6);
        // Clamped above 1.0
        assert!((t.progress(20.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn toast_opacity_fade() {
        let t = Toast::new(ToastLevel::Info, "hello", 0.0); // duration = 3.0
        // Before 80%: full opacity
        assert!((t.opacity(0.0) - 1.0).abs() < 1e-6);
        assert!((t.opacity(2.0) - 1.0).abs() < 1e-6);
        // At 80% (2.4s): start fading
        assert!((t.opacity(2.4) - 1.0).abs() < 0.01);
        // At 90% (2.7s): half faded
        assert!((t.opacity(2.7) - 0.5).abs() < 0.01);
        // At 100% (3.0s): fully faded
        assert!(t.opacity(3.0) < 0.01);
    }

    #[test]
    fn toast_dismissed() {
        let mut t = Toast::new(ToastLevel::Info, "hello", 0.0);
        assert!(!t.is_expired(0.0));
        t.dismissed = true;
        assert!(t.is_expired(0.0));
        assert!((t.opacity(0.0) - 0.0).abs() < 1e-6);
    }

    #[test]
    fn toast_custom_duration() {
        let t = Toast::new(ToastLevel::Info, "hello", 0.0).with_duration(10.0);
        assert!(!t.is_expired(9.0));
        assert!(t.is_expired(10.0));
    }

    #[test]
    fn queue_push_and_drain() {
        let mut q = ToastQueue::default();
        q.info("one", 0.0);
        q.success("two", 0.0);
        assert_eq!(q.active_count(), 2);
        assert_eq!(q.history_count(), 2);

        // After 3 seconds, info + success expire (duration = 3.0)
        q.drain_expired(3.0);
        assert_eq!(q.active_count(), 0);
        // History is preserved
        assert_eq!(q.history_count(), 2);
    }

    #[test]
    fn queue_max_visible() {
        let mut q = ToastQueue::default();
        q.max_visible = 3;
        for i in 0..5 {
            q.info(format!("toast {i}"), 0.0);
        }
        assert_eq!(q.active_count(), 3);
        // All 5 recorded in history
        assert_eq!(q.history_count(), 5);
    }

    #[test]
    fn queue_dismiss() {
        let mut q = ToastQueue::default();
        q.info("one", 0.0);
        q.error("two", 0.0);
        assert_eq!(q.active_count(), 2);

        q.dismiss(0);
        q.drain_expired(0.0);
        assert_eq!(q.active_count(), 1);
        assert_eq!(q.active()[0].message, "two");
    }

    #[test]
    fn queue_max_history() {
        let mut q = ToastQueue::default();
        q.max_history = 3;
        for i in 0..5 {
            q.info(format!("toast {i}"), 0.0);
        }
        assert_eq!(q.history_count(), 3);
        // Oldest should have been evicted
        assert_eq!(q.history()[0].message, "toast 2");
    }

    #[test]
    fn queue_clear_history() {
        let mut q = ToastQueue::default();
        q.info("one", 0.0);
        q.info("two", 0.0);
        assert_eq!(q.history_count(), 2);
        q.clear_history();
        assert_eq!(q.history_count(), 0);
    }

    #[test]
    fn error_duration_longer_than_info() {
        let info = Toast::new(ToastLevel::Info, "i", 0.0);
        let err = Toast::new(ToastLevel::Error, "e", 0.0);
        assert!(err.duration > info.duration);
    }

    #[test]
    fn level_icons_and_labels() {
        assert_eq!(ToastLevel::Info.icon(), "ℹ");
        assert_eq!(ToastLevel::Success.icon(), "✓");
        assert_eq!(ToastLevel::Warning.icon(), "⚠");
        assert_eq!(ToastLevel::Error.icon(), "✗");

        assert_eq!(ToastLevel::Info.label(), "Info");
        assert_eq!(ToastLevel::Error.label(), "Error");
    }
}
