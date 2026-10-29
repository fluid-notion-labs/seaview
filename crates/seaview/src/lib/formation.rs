//! Formation layouts for batch-imported environment objects.
//!
//! Pure logic: takes an index and count, returns a world-space position.
//! No ECS, no system signatures — usable from import handling and UI alike.

use bevy::prelude::Vec3;

/// How a batch of newly added objects is arranged in the scene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Formation {
    /// Square-ish grid on the XZ plane, centred on the origin.
    #[default]
    Grid,
    /// Single row along X.
    Line,
    /// Circle around the origin (radius grows with count).
    Ring,
    /// Grid cells with deterministic per-object jitter (still deterministic
    /// for a given index, so re-layouts are stable).
    Scatter,
}

impl Formation {
    pub const ALL: [Formation; 4] = [Self::Grid, Self::Line, Self::Ring, Self::Scatter];

    pub fn label(self) -> &'static str {
        match self {
            Self::Grid => "Grid",
            Self::Line => "Line",
            Self::Ring => "Ring",
            Self::Scatter => "Scatter",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "grid" => Some(Self::Grid),
            "line" | "row" => Some(Self::Line),
            "ring" | "circle" => Some(Self::Ring),
            "scatter" => Some(Self::Scatter),
            _ => None,
        }
    }
}

/// Default spacing between neighbouring objects, world units.
pub const DEFAULT_SPACING: f32 = 3.0;

/// Position of object `index` in a formation of `count` objects.
///
/// `count` should be the size of the whole batch, so ring radius and grid
/// columns stay stable as objects trickle in (e.g. one per poll tick).
pub fn placement(index: usize, count: usize, formation: Formation, spacing: f32) -> Vec3 {
    let count = count.max(1);
    match formation {
        Formation::Grid | Formation::Scatter => {
            if count == 1 {
                return Vec3::ZERO;
            }
            let cols = (count as f32).sqrt().ceil() as usize;
            let rows = count.div_ceil(cols);
            let col = index % cols;
            let row = index / cols;
            let mut x = (col as f32 - (cols - 1) as f32 / 2.0) * spacing;
            let mut z = (row as f32 - (rows - 1) as f32 / 2.0) * spacing;
            if formation == Formation::Scatter {
                // Deterministic jitter from the index (±40% of spacing).
                let (jx, jz) = jitter(index);
                x += jx * spacing * 0.4;
                z += jz * spacing * 0.4;
            }
            Vec3::new(x, 0.0, z)
        }
        Formation::Line => {
            let x = (index as f32 - (count - 1) as f32 / 2.0) * spacing;
            Vec3::new(x, 0.0, 0.0)
        }
        Formation::Ring => {
            if count == 1 {
                return Vec3::ZERO;
            }
            let radius = (count as f32 * spacing / std::f32::consts::TAU).max(spacing);
            let angle = index as f32 / count as f32 * std::f32::consts::TAU;
            Vec3::new(angle.cos() * radius, 0.0, angle.sin() * radius)
        }
    }
}

/// Positions for a whole batch of `count` objects.
pub fn layout(count: usize, formation: Formation, spacing: f32) -> Vec<Vec3> {
    (0..count)
        .map(|i| placement(i, count, formation, spacing))
        .collect()
}

/// Splitmix-style two-channel hash of the index → [-0.5, 0.5]² jitter.
fn jitter(index: usize) -> (f32, f32) {
    let mut z = (index as u64).wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^= z >> 31;
    let a = (z & 0xFFFF) as f32 / 65535.0 - 0.5;
    let b = ((z >> 16) & 0xFFFF) as f32 / 65535.0 - 0.5;
    (a, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_object_at_origin() {
        for f in Formation::ALL {
            assert_eq!(placement(0, 1, f, 3.0), Vec3::ZERO, "{f:?}");
        }
    }

    #[test]
    fn grid_is_centered() {
        // 3x3 grid: first cell at -spacing, last at +spacing, centre cell at origin.
        let p = placement(4, 9, Formation::Grid, 3.0);
        assert_eq!(p, Vec3::ZERO);
        let corners = [placement(0, 9, Formation::Grid, 3.0), placement(8, 9, Formation::Grid, 3.0)];
        assert_eq!(corners[0], -corners[1]);
    }

    #[test]
    fn line_is_evenly_spaced() {
        let v = layout(4, Formation::Line, 2.0);
        assert_eq!(v[0].x, -3.0);
        assert_eq!(v[3].x, 3.0);
        assert_eq!(v[1].x, -1.0);
    }

    #[test]
    fn ring_points_on_circle() {
        let v = layout(8, Formation::Ring, 3.0);
        let expected = (8.0f32 * 3.0 / std::f32::consts::TAU).max(3.0);
        for p in &v {
            let r = (p.x * p.x + p.z * p.z).sqrt();
            assert!((r - expected).abs() < 1e-4, "radius {r} vs {expected}");
        }
    }

    #[test]
    fn scatter_is_deterministic_and_bounded() {
        let a = placement(3, 16, Formation::Scatter, 3.0);
        let b = placement(3, 16, Formation::Scatter, 3.0);
        assert_eq!(a, b);
        assert!(a.x.abs() < 16.0 && a.z.abs() < 16.0);
    }

    #[test]
    fn parse() {
        assert_eq!(Formation::parse("grid"), Some(Formation::Grid));
        assert_eq!(Formation::parse("Ring"), Some(Formation::Ring));
        assert_eq!(Formation::parse("nope"), None);
    }
}
