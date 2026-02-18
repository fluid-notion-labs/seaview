//! Generic fire-and-fade animation effect and relative-time formatting.
//!
//! Pure logic — no Bevy system signatures, no ECS queries.
//! Reusable for any UI element that should appear instantly then fade out.

/// A fire-and-fade animation effect.
///
/// Starts at full opacity and fades to zero over `duration` seconds using an
/// ease-out curve. The first 60% of the duration is at full opacity; the
/// remaining 40% fades out with a smooth ease-out.
#[derive(Debug, Clone)]
pub struct FadeEffect {
    /// Monotonic timestamp (seconds since app start) when the effect started.
    pub start_time: f64,
    /// Total duration of the effect in seconds (including the fade portion).
    pub duration: f64,
}

/// Fraction of the total duration that remains at full opacity before fading.
const HOLD_FRACTION: f64 = 0.6;

impl FadeEffect {
    /// Create a new fade effect starting at `now` with the given `duration`.
    pub fn new(now: f64, duration: f64) -> Self {
        Self {
            start_time: now,
            duration,
        }
    }

    /// How far through its lifetime this effect is (0.0 = just started, 1.0 = expired).
    pub fn progress(&self, now: f64) -> f64 {
        if self.duration <= 0.0 {
            return 1.0;
        }
        let elapsed = now - self.start_time;
        (elapsed / self.duration).clamp(0.0, 1.0)
    }

    /// Whether the effect has fully expired.
    pub fn is_expired(&self, now: f64) -> bool {
        self.progress(now) >= 1.0
    }

    /// Current opacity (1.0 → 0.0) with ease-out during the fade portion.
    ///
    /// - For the first [`HOLD_FRACTION`] of the duration: returns 1.0.
    /// - For the remaining portion: smooth ease-out from 1.0 to 0.0.
    ///
    /// The ease-out curve is `1 - t^2` where `t` is the normalised fade progress.
    pub fn opacity(&self, now: f64) -> f32 {
        let p = self.progress(now);
        if p <= HOLD_FRACTION {
            1.0
        } else {
            // Normalise the fade portion to 0.0..1.0
            let fade_t = (p - HOLD_FRACTION) / (1.0 - HOLD_FRACTION);
            // Ease-out: starts fast, decelerates
            (1.0 - fade_t * fade_t) as f32
        }
    }

    /// Restart the effect at a new timestamp, keeping the same duration.
    pub fn restart(&mut self, now: f64) {
        self.start_time = now;
    }
}

// ---------------------------------------------------------------------------
// Relative-time formatting
// ---------------------------------------------------------------------------

/// Format an elapsed duration (in seconds) as a human-readable relative time.
///
/// Examples: `"just now"`, `"12s ago"`, `"2m ago"`, `"1h ago"`, `"3d ago"`,
/// `"1mo ago"`.
pub fn format_relative_time(elapsed_secs: f64) -> String {
    if elapsed_secs < 0.0 {
        return "just now".to_string();
    }

    let secs = elapsed_secs as u64;

    match secs {
        0..=9 => "just now".to_string(),
        10..=59 => format!("{}s ago", secs),
        60..=3599 => {
            let minutes = secs / 60;
            format!("{}m ago", minutes)
        }
        3600..=86399 => {
            let hours = secs / 3600;
            format!("{}h ago", hours)
        }
        86400..=2591999 => {
            let days = secs / 86400;
            format!("{}d ago", days)
        }
        _ => {
            let months = secs / 2592000; // ~30 days
            if months <= 1 {
                "1mo ago".to_string()
            } else {
                format!("{}mo ago", months)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- FadeEffect ----------------------------------------------------------

    #[test]
    fn new_effect_starts_at_full_opacity() {
        let fx = FadeEffect::new(10.0, 10.0);
        assert!((fx.opacity(10.0) - 1.0).abs() < 1e-6);
        assert!(!fx.is_expired(10.0));
    }

    #[test]
    fn hold_phase_is_fully_opaque() {
        let fx = FadeEffect::new(0.0, 10.0);
        // At 50% of duration (within 60% hold) → still 1.0
        assert!((fx.opacity(5.0) - 1.0).abs() < 1e-6);
        // At exactly 60% → still 1.0
        assert!((fx.opacity(6.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn fade_phase_decreases_opacity() {
        let fx = FadeEffect::new(0.0, 10.0);
        let op_70 = fx.opacity(7.0); // 70% through
        let op_90 = fx.opacity(9.0); // 90% through
        assert!(op_70 < 1.0, "should be fading at 70%: {op_70}");
        assert!(
            op_90 < op_70,
            "should be lower at 90% than 70%: {op_90} vs {op_70}"
        );
        assert!(op_90 > 0.0, "should not be zero before expiry: {op_90}");
    }

    #[test]
    fn expired_at_end() {
        let fx = FadeEffect::new(0.0, 10.0);
        assert!(fx.is_expired(10.0));
        assert!(fx.is_expired(100.0));
        assert!((fx.opacity(10.0)).abs() < 1e-6);
    }

    #[test]
    fn progress_clamped() {
        let fx = FadeEffect::new(5.0, 10.0);
        assert!((fx.progress(0.0) - 0.0).abs() < 1e-6); // before start
        assert!((fx.progress(5.0) - 0.0).abs() < 1e-6); // at start
        assert!((fx.progress(10.0) - 0.5).abs() < 1e-6); // midway
        assert!((fx.progress(15.0) - 1.0).abs() < 1e-6); // at end
        assert!((fx.progress(99.0) - 1.0).abs() < 1e-6); // well past
    }

    #[test]
    fn restart_resets_animation() {
        let mut fx = FadeEffect::new(0.0, 10.0);
        assert!(fx.is_expired(10.0));
        fx.restart(10.0);
        assert!(!fx.is_expired(10.0));
        assert!((fx.opacity(10.0) - 1.0).abs() < 1e-6);
    }

    #[test]
    fn zero_duration_is_immediately_expired() {
        let fx = FadeEffect::new(0.0, 0.0);
        assert!(fx.is_expired(0.0));
    }

    // -- format_relative_time ------------------------------------------------

    #[test]
    fn just_now() {
        assert_eq!(format_relative_time(0.0), "just now");
        assert_eq!(format_relative_time(5.0), "just now");
        assert_eq!(format_relative_time(9.9), "just now");
    }

    #[test]
    fn seconds_ago() {
        assert_eq!(format_relative_time(10.0), "10s ago");
        assert_eq!(format_relative_time(30.0), "30s ago");
        assert_eq!(format_relative_time(59.0), "59s ago");
    }

    #[test]
    fn minutes_ago() {
        assert_eq!(format_relative_time(60.0), "1m ago");
        assert_eq!(format_relative_time(120.0), "2m ago");
        assert_eq!(format_relative_time(3599.0), "59m ago");
    }

    #[test]
    fn hours_ago() {
        assert_eq!(format_relative_time(3600.0), "1h ago");
        assert_eq!(format_relative_time(7200.0), "2h ago");
        assert_eq!(format_relative_time(86399.0), "23h ago");
    }

    #[test]
    fn days_ago() {
        assert_eq!(format_relative_time(86400.0), "1d ago");
        assert_eq!(format_relative_time(259200.0), "3d ago");
    }

    #[test]
    fn months_ago() {
        assert_eq!(format_relative_time(2592000.0), "1mo ago");
        assert_eq!(format_relative_time(5184000.0), "2mo ago");
    }

    #[test]
    fn negative_elapsed_is_just_now() {
        assert_eq!(format_relative_time(-5.0), "just now");
    }
}
