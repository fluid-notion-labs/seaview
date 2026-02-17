//! FPS-style camera controller with mouse look and keyboard movement
//!
//! Controls:
//! - WASD: Move forward/back/left/right
//! - Q/E: Move down/up
//! - Mouse: Look around (when cursor is grabbed)
//! - Left Click: Grab cursor for mouse look
//! - Escape: Release cursor
//! - Shift: 10x movement speed boost
//! - Alt: 0.1x movement speed (precision mode)
//! - Ctrl+C: Center camera on mesh

use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy_egui::EguiContexts;

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

/// System that fires [`CenterOnMeshEvent`] on Ctrl+C.
pub fn keyboard_center_on_mesh(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut center_events: MessageWriter<CenterOnMeshEvent>,
) {
    let ctrl = keyboard.pressed(KeyCode::ControlLeft) || keyboard.pressed(KeyCode::ControlRight);
    if ctrl && keyboard.just_pressed(KeyCode::KeyC) {
        center_events.write(CenterOnMeshEvent);
        debug!("🎯 Center camera triggered via Ctrl+C");
    }
}

/// System that provides fps-style camera controls
pub fn camera_controller(
    time: Res<Time>,
    mut mouse_events: MessageReader<MouseMotion>,
    input: Res<ButtonInput<KeyCode>>,
    mut query: Query<(&mut Transform, &FpsCamera), With<Camera>>,
) {
    let Ok((mut transform, fps_camera)) = query.single_mut() else {
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

    // Handle movement
    let mut movement = Vec3::ZERO;
    let base_speed = fps_camera.speed;

    // Apply speed modifiers
    // Shift: 10x speed for fast movement
    // Alt: 0.1x speed for precision movement
    let speed_modifier = if input.pressed(KeyCode::ShiftLeft) || input.pressed(KeyCode::ShiftRight)
    {
        10.0 // Shift for 10x speed
    } else if input.pressed(KeyCode::AltLeft) || input.pressed(KeyCode::AltRight) {
        0.1 // Alt for slow movement (all directions)
    } else {
        1.0
    };

    let speed = base_speed * speed_modifier;

    if input.pressed(KeyCode::KeyW) {
        movement += *transform.forward();
    }
    if input.pressed(KeyCode::KeyS) {
        movement -= *transform.forward();
    }
    if input.pressed(KeyCode::KeyA) {
        movement -= *transform.right();
    }
    if input.pressed(KeyCode::KeyD) {
        movement += *transform.right();
    }

    // Handle vertical movement
    if input.pressed(KeyCode::KeyE) {
        movement += Vec3::Y;
    }
    if input.pressed(KeyCode::KeyQ) {
        movement -= Vec3::Y;
    }

    // Apply movement
    if movement != Vec3::ZERO {
        movement = movement.normalize();
        transform.translation += movement * speed * time.delta_secs();
    }
}

pub fn cursor_grab_system(
    mut cursor_query: Query<&mut CursorOptions, With<PrimaryWindow>>,
    input: Res<ButtonInput<MouseButton>>,
    key_input: Res<ButtonInput<KeyCode>>,
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

    if input.just_pressed(MouseButton::Left) {
        // Grab cursor when left mouse button is pressed
        cursor.visible = false;
        cursor.grab_mode = CursorGrabMode::Locked;
        fps_camera.escape_mode = false;
    }

    if key_input.just_pressed(KeyCode::Escape) {
        // Release cursor when Escape is pressed
        cursor.visible = true;
        cursor.grab_mode = CursorGrabMode::None;
        fps_camera.escape_mode = true;
    }
}
