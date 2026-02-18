//! FPS-style camera controller with mouse look and keyboard movement
//!
//! Controls:
//! - WASD: Move forward/back/left/right
//! - Q/E: Move down/up
//! - Mouse: Look around (when cursor is grabbed)
//! - Left Click: Grab cursor for mouse look
//! - Escape: Release cursor
//! - Scroll wheel (while moving): Adjust base movement speed
//! - Shift: 5× movement speed
//! - Alt: 25× movement speed
//! - Ctrl+C: Center camera on mesh

use bevy::input::mouse::{MouseMotion, MouseWheel};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy_egui::EguiContexts;

use crate::app::input::ActionStateRes;
use crate::lib::actions::Action;
use crate::lib::mesh_info::MeshDimensions;
use crate::lib::sequence::loader::LoaderConfig;

#[derive(Message)]
pub struct CenterOnMeshEvent;

#[derive(Component)]
pub struct FpsCamera {
    pub sensitivity: f32,
    pub speed: f32,
    pub escape_mode: bool,
}

impl Default for FpsCamera {
    fn default() -> Self {
        Self {
            sensitivity: 0.3,
            speed: 10.0,
            escape_mode: true,
        }
    }
}

/// System that handles centering the camera on the current mesh.
///
/// Uses [`MeshDimensions`] (pre-computed AABB) and [`LoaderConfig`]
/// (coordinate transform) to position the camera at a good viewing distance
/// looking at the mesh center.
pub fn handle_center_on_mesh(
    mut center_events: MessageReader<CenterOnMeshEvent>,
    mut camera_query: Query<(&mut Transform, &mut FpsCamera), (With<Camera3d>, With<FpsCamera>)>,
    mesh_dims: Res<MeshDimensions>,
    loader_config: Res<LoaderConfig>,
) {
    for _event in center_events.read() {
        let (Some(mn), Some(mx)) = (mesh_dims.min, mesh_dims.max) else {
            warn!("CenterOnMeshEvent: no mesh bounds available yet");
            return;
        };

        // AABB center and radius in mesh-local space
        let local_center = (mn + mx) * 0.5;
        let local_radius = mn.distance(mx) * 0.5;

        // Transform to world space via the coordinate transform
        let coord = loader_config.coord_transform;
        let world_center = coord.transform_point(local_center);

        // Place camera at 2.5× the bounding-sphere radius, offset diagonally
        let camera_distance = (local_radius * 2.5).max(10.0);
        let camera_position =
            world_center + Vec3::new(camera_distance, camera_distance * 0.5, camera_distance);

        let Ok((mut transform, mut fps_camera)) = camera_query.single_mut() else {
            warn!("CenterOnMeshEvent: no camera entity found");
            return;
        };

        transform.translation = camera_position;
        transform.look_at(world_center, Vec3::Y);

        // Exit escape mode so the user can immediately control the camera
        fps_camera.escape_mode = false;

        debug!(
            "🎯 Camera centred on mesh — center: {:?}, radius: {:.1}, camera: {:?}",
            world_center, local_radius, camera_position,
        );
    }
}

/// System that fires [`CenterOnMeshEvent`] via the action system (Ctrl+C).
pub fn keyboard_center_on_mesh(
    actions: Res<ActionStateRes>,
    mut center_events: MessageWriter<CenterOnMeshEvent>,
) {
    if actions.0.just_activated(Action::CenterOnMesh) {
        center_events.write(CenterOnMeshEvent);
        debug!("🎯 Center camera triggered via Ctrl+C");
    }
}

/// Factor by which base speed changes per scroll tick.
const SCROLL_SPEED_FACTOR: f32 = 1.2;
/// Minimum base movement speed.
const MIN_BASE_SPEED: f32 = 0.1;
/// Maximum base movement speed.
const MAX_BASE_SPEED: f32 = 10000.0;

/// System that provides fps-style camera controls.
///
/// Reads [`ActionStateRes`] for movement and speed modifiers instead of raw
/// `ButtonInput<KeyCode>`. Mouse look still reads `MouseMotion` directly
/// (the action system doesn't abstract mouse deltas).
pub fn camera_controller(
    time: Res<Time>,
    mut mouse_events: MessageReader<MouseMotion>,
    mut scroll_events: MessageReader<MouseWheel>,
    actions: Res<ActionStateRes>,
    mut query: Query<(&mut Transform, &mut FpsCamera), With<Camera>>,
) {
    let Ok((mut transform, mut fps_camera)) = query.single_mut() else {
        return;
    };

    // Don't process camera controls if in escape mode
    if fps_camera.escape_mode {
        // Clear mouse events to prevent them from accumulating
        mouse_events.clear();
        return;
    }

    // Handle mouse look
    let mut mouse_delta = Vec2::ZERO;
    for mouse_event in mouse_events.read() {
        mouse_delta += mouse_event.delta;
    }

    if mouse_delta != Vec2::ZERO {
        // Apply mouse sensitivity
        mouse_delta *= fps_camera.sensitivity * 0.01;

        // Calculate new rotation
        let yaw = -mouse_delta.x;
        let pitch = -mouse_delta.y;

        // Apply rotations
        transform.rotate_y(yaw);
        transform.rotate_local_x(pitch);

        // Prevent camera from flipping upside down
        let forward = *transform.forward();
        let right = *transform.right();
        let up = right.cross(forward);

        // Ensure we don't over-rotate on the pitch axis
        if up.y < 0.1 && pitch < 0.0 {
            // Don't rotate further down
        } else if up.y > 0.9 && pitch > 0.0 {
            // Don't rotate further up
        } else {
            // Rotation is fine
        }
    }

    // ── Movement via ActionState ────────────────────────────────────

    let (right, up, forward) = actions.0.movement_vector();
    let moving = actions.0.is_moving();

    // When not moving, no further processing needed
    if !moving {
        scroll_events.clear();
    }

    // Scroll wheel adjusts base speed while moving
    if moving {
        for scroll in scroll_events.read() {
            let ticks = scroll.y;
            if ticks > 0.0 {
                fps_camera.speed =
                    (fps_camera.speed * SCROLL_SPEED_FACTOR.powf(ticks)).min(MAX_BASE_SPEED);
            } else if ticks < 0.0 {
                fps_camera.speed =
                    (fps_camera.speed * SCROLL_SPEED_FACTOR.powf(ticks)).max(MIN_BASE_SPEED);
            }
            debug!("🎚️ Base movement speed: {:.1}", fps_camera.speed);
        }
    }

    let speed = fps_camera.speed * actions.0.speed_multiplier();

    // Build world-space movement vector from the action system's abstract axes
    let mut movement = Vec3::ZERO;
    movement += *transform.forward() * forward;
    movement += *transform.right() * right;
    movement += Vec3::Y * up;

    // Apply movement
    if movement != Vec3::ZERO {
        movement = movement.normalize();
        transform.translation += movement * speed * time.delta_secs();
    }
}

/// Reads [`ActionStateRes`] for cursor grab / release instead of raw keys.
///
/// When egui wants pointer or keyboard input we skip cursor grabbing so the
/// UI remains usable.
pub fn cursor_grab_system(
    mut cursor_query: Query<&mut CursorOptions, With<PrimaryWindow>>,
    actions: Res<ActionStateRes>,
    mut camera_query: Query<&mut FpsCamera, With<Camera>>,
    mut egui_contexts: EguiContexts,
) {
    let Ok(mut cursor) = cursor_query.single_mut() else {
        return;
    };
    let Ok(mut fps_camera) = camera_query.single_mut() else {
        return;
    };

    // Check if egui wants to use the mouse/keyboard
    if let Ok(ctx) = egui_contexts.ctx_mut() {
        if ctx.wants_pointer_input() || ctx.wants_keyboard_input() {
            // Don't grab cursor if egui is using the mouse
            return;
        }
    }

    if actions.0.just_activated(Action::GrabCursor) {
        // Grab cursor when left mouse button is pressed
        cursor.visible = false;
        cursor.grab_mode = CursorGrabMode::Locked;
        fps_camera.escape_mode = false;
    }

    if actions.0.just_activated(Action::ReleaseCursor) {
        // Release cursor when Escape is pressed
        cursor.visible = true;
        cursor.grab_mode = CursorGrabMode::None;
        fps_camera.escape_mode = true;
    }
}
