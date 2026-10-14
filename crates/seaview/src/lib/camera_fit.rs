//! Camera fit-to-view math (pure logic, no ECS).
//!
//! Computes the camera distance needed so a world-space AABB fits the
//! viewport "at about the right zoom", keeping the camera's current viewing
//! direction. Used by the 🔍 Focus scene button.

use bevy::math::{Mat4, Vec3};

/// Margin added around the fitted view so the scene is not flush with the
/// screen edges.
pub const FIT_MARGIN: f32 = 1.1;

/// Fallback viewing direction when the camera sits exactly at the scene
/// centre (a "nice 3/4 angle" like the center-on-mesh offset).
const FALLBACK_DIRECTION: Vec3 = Vec3::new(0.75, 0.5, 1.0);

/// Result of a fit-to-view computation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneFit {
    /// Centre of the scene AABB (world space).
    pub center: Vec3,
    /// Unit vector from the scene centre towards the camera.
    pub direction: Vec3,
    /// Distance to place the camera back along `direction`.
    pub distance: f32,
}

/// Compute the camera placement that fits a world-space AABB in view.
///
/// Keeps the camera's current viewing direction (from `eye` towards the box
/// centre) and moves the camera back along it until the whole box fits inside
/// the vertical and horizontal FOV, scaled by `margin`.
///
/// Returns `None` when the inputs are degenerate (invalid FOV/aspect, empty
/// or non-finite box).
pub fn compute_fit(
    box_min: Vec3,
    box_max: Vec3,
    eye: Vec3,
    fov_y: f32,
    aspect: f32,
    margin: f32,
) -> Option<SceneFit> {
    if !(fov_y.is_finite() && fov_y > 0.0 && fov_y < std::f32::consts::PI)
        || !(aspect.is_finite() && aspect > 0.0)
        || !(margin.is_finite() && margin > 0.0)
    {
        return None;
    }
    if !(box_min.is_finite() && box_max.is_finite()) {
        return None;
    }
    // Degenerate / empty box on any axis is fine as long as it is finite
    // (a flat plane is legitimate scene content), but a fully zero-size box
    // still has a well-defined fit, so no rejection needed here.

    let center = (box_min + box_max) * 0.5;
    let half_extents = (box_max - box_min) * 0.5;

    // Viewing direction from the eye towards the scene; fall back to a nice
    // 3/4 angle when the camera sits exactly on the centre.
    let to_center = center - eye;
    let direction = if to_center.length_squared() > 1e-12 {
        -to_center.normalize()
    } else {
        FALLBACK_DIRECTION.normalize()
    };

    // Orthonormal camera basis for the view direction, Y-up.
    let forward = -direction; // eye -> centre
    let right = forward
        .cross(Vec3::Y)
        .try_normalize()
        // Looking straight down Y: pick X as right.
        .unwrap_or(Vec3::X);
    let up = right.cross(forward).normalize();

    // Half-extent of an axis-aligned box along a unit axis u is
    // Σ |half_extent_i * u_i|.
    let proj_right = half_extents.dot(right.abs());
    let proj_up = half_extents.dot(up.abs());

    let tan_half_y = (fov_y * 0.5).tan();
    let tan_half_x = tan_half_y * aspect;

    let dist_right = if tan_half_x > 1e-9 { proj_right / tan_half_x } else { 0.0 };
    let dist_up = if tan_half_y > 1e-9 { proj_up / tan_half_y } else { 0.0 };

    let distance = margin * dist_right.max(dist_up);

    Some(SceneFit {
        center,
        direction,
        distance,
    })
}

/// Transform a mesh-local AABB into world space corners via a transform
/// matrix, returning the enclosing world-space (min, max).
///
/// Handles rotation/scale by transforming all 8 corners.
pub fn world_aabb(transform: &Mat4, local_min: Vec3, local_max: Vec3) -> (Vec3, Vec3) {
    let mut wmin = Vec3::INFINITY;
    let mut wmax = Vec3::NEG_INFINITY;
    for sx in [local_min.x, local_max.x] {
        for sy in [local_min.y, local_max.y] {
            for sz in [local_min.z, local_max.z] {
                let w = transform.transform_point3(Vec3::new(sx, sy, sz));
                wmin = wmin.min(w);
                wmax = wmax.max(w);
            }
        }
    }
    (wmin, wmax)
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::math::{Mat4, Vec3};

    fn approx(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-4
    }

    #[test]
    fn fit_axis_aligned_front_view() {
        // Box of half extents (1, 1, 1) centred at origin, camera on +Z,
        // 90° vertical FOV, 1:1 aspect → projected half extent 1, needs
        // tan(45°) = 1 → distance 1, × margin.
        let fit = compute_fit(
            Vec3::splat(-1.0),
            Vec3::splat(1.0),
            Vec3::new(0.0, 0.0, 100.0),
            std::f32::consts::FRAC_PI_2,
            1.0,
            1.0,
        )
        .unwrap();
        assert!(approx(fit.distance, 1.0), "distance = {}", fit.distance);
        assert!(approx(fit.direction.z, 1.0));
        assert!(approx(fit.center.x, 0.0));
    }

    #[test]
    fn fit_horizontal_dominate_with_aspect() {
        // Wide box (x half extent 4, y 1), camera on +Z, aspect 2: the
        // horizontal constraint dominates and cancels the aspect factor.
        let fit = compute_fit(
            Vec3::new(-4.0, -1.0, -1.0),
            Vec3::new(4.0, 1.0, 1.0),
            Vec3::new(0.0, 0.0, 100.0),
            std::f32::consts::FRAC_PI_2,
            2.0,
            1.0,
        )
        .unwrap();
        // proj_right = 4, tan_half_x = tan(45°)*2 = 2 → dist = 2.
        // proj_up = 1 → dist 1. Horizontal wins: 2.
        assert!(approx(fit.distance, 2.0), "distance = {}", fit.distance);
    }

    #[test]
    fn fit_keeps_direction_and_margin() {
        let fit = compute_fit(
            Vec3::splat(-1.0),
            Vec3::splat(1.0),
            Vec3::new(10.0, 10.0, 10.0),
            std::f32::consts::FRAC_PI_2,
            1.0,
            2.0,
        )
        .unwrap();
        let dir = (Vec3::new(10.0, 10.0, 10.0) - Vec3::ZERO).normalize();
        assert!(approx(fit.direction.x, dir.x));
        assert!(approx(fit.direction.y, dir.y));
        assert!(approx(fit.direction.z, dir.z));
        // Camera is on the ray from center through the original eye, at
        // `distance` (which includes the margin, so ≠ |eye| here).
        let cam = fit.center + fit.direction * fit.distance;
        let eye = Vec3::new(10.0, 10.0, 10.0);
        assert!(cam.cross(eye).length() < 1e-3); // center is origin
        assert!(approx((cam - fit.center).length(), fit.distance));
        assert!(fit.distance > 0.0);
    }

    #[test]
    fn fit_degenerate_eye_at_center_uses_fallback() {
        let fit = compute_fit(
            Vec3::splat(-1.0),
            Vec3::splat(1.0),
            Vec3::ZERO,
            std::f32::consts::FRAC_PI_2,
            1.0,
            1.0,
        )
        .unwrap();
        let expected = FALLBACK_DIRECTION.normalize();
        assert!(approx(fit.direction.x, expected.x));
        assert!(approx(fit.direction.y, expected.y));
        assert!(approx(fit.direction.z, expected.z));
    }

    #[test]
    fn fit_rejects_invalid_inputs() {
        assert!(compute_fit(
            Vec3::ZERO,
            Vec3::ONE,
            Vec3::Z,
            -1.0,
            1.0,
            1.0
        )
        .is_none());
        assert!(compute_fit(
            Vec3::ZERO,
            Vec3::ONE,
            Vec3::Z,
            1.0,
            0.0,
            1.0
        )
        .is_none());
        assert!(compute_fit(
            Vec3::ZERO,
            Vec3::ONE,
            Vec3::Z,
            1.0,
            1.0,
            f32::NAN
        )
        .is_none());
        assert!(compute_fit(
            Vec3::splat(f32::NAN),
            Vec3::ONE,
            Vec3::Z,
            1.0,
            1.0,
            1.0
        )
        .is_none());
    }

    #[test]
    fn world_aabb_translation() {
        let m = Mat4::from_translation(Vec3::new(5.0, 0.0, -3.0));
        let (mn, mx) = world_aabb(&m, Vec3::splat(-1.0), Vec3::splat(1.0));
        assert!(approx(mn.x, 4.0) && approx(mx.x, 6.0));
        assert!(approx(mn.y, -1.0) && approx(mx.y, 1.0));
        assert!(approx(mn.z, -4.0) && approx(mx.z, -2.0));
    }

    #[test]
    fn world_aabb_rotation() {
        // Unit cube rotated 90° around Y: local x extent becomes world z.
        let m = Mat4::from_rotation_y(std::f32::consts::FRAC_PI_2);
        let (mn, mx) = world_aabb(&m, Vec3::new(-2.0, -1.0, -1.0), Vec3::new(2.0, 1.0, 1.0));
        // x in [-1, 1] (from local z), z in [-2, 2] (from local x).
        assert!(approx(mn.x, -1.0) && approx(mx.x, 1.0));
        assert!(approx(mn.y, -1.0) && approx(mx.y, 1.0));
        assert!(approx(mn.z, -2.0) && approx(mx.z, 2.0));
    }

    #[test]
    fn world_aabb_scale() {
        let m = Mat4::from_scale(Vec3::new(2.0, 3.0, 0.5));
        let (mn, mx) = world_aabb(&m, Vec3::new(-1.0, -1.0, -1.0), Vec3::new(1.0, 1.0, 1.0));
        assert!(approx(mn.x, -2.0) && approx(mx.x, 2.0));
        assert!(approx(mn.y, -3.0) && approx(mx.y, 3.0));
        assert!(approx(mn.z, -0.5) && approx(mx.z, 0.5));
    }
}
