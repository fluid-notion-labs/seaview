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

// use crate::lib::sequence::async_cache::AsyncMeshCache;
use bevy::input::mouse::MouseMotion;
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use bevy_egui::EguiContexts;

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

/// System that handles centering the camera on the current mesh
/// TODO: Reimplement after switching to Bevy's asset loading
pub fn handle_center_on_mesh(
    mut center_events: MessageReader<CenterOnMeshEvent>,
    _camera_query: Query<(&mut Transform, &mut FpsCamera), (With<Camera3d>, With<FpsCamera>)>,
    // mesh_cache: Res<AsyncMeshCache>,
    _meshes: Res<Assets<Mesh>>,
) {
    for _event in center_events.read() {
        warn!("CenterOnMeshEvent received but handler is disabled - needs reimplementation with Bevy asset loading");
        // TODO: Reimplement mesh centering logic
        /*
        if let Some((path, mesh_handle)) = mesh_cache.cache.iter().next() {
                        debug!("📁 Using mesh from path: {:?}", path);
                        trace!("🔗 Mesh handle: {:?}", mesh_handle);

                        match meshes.get(mesh_handle) {
                            Some(mesh) => {
                                debug!("✅ Mesh found in assets");

                                // Calculate mesh centroid using 5% random sampling for performance
                                let centroid = calculate_mesh_centroid(mesh);
                                debug!("📊 Calculated centroid: {:?}", centroid);

                                let bounds_radius = calculate_mesh_bounds_radius(mesh, centroid);
                                debug!("📏 Mesh bounds radius: {:.2}", bounds_radius);

                                // Position camera at a good viewing distance (2.5x the mesh bounds radius)
                                let camera_distance = bounds_radius * 2.5;
                                let camera_position = centroid
                                    + Vec3::new(
                                        camera_distance,
                                        camera_distance * 0.5,
                                        camera_distance,
                                    );

                                debug!("📹 Moving camera to: {:?}", camera_position);
                                debug!("🎯 Looking at: {:?}", centroid);

                                // Update camera transform
                                transform.translation = camera_position;
                                transform.look_at(centroid, Vec3::Y);

                                // Disable escape mode to allow immediate camera control
                                fps_camera.escape_mode = false;
                                debug!("🔓 Disabled escape mode");

                                debug!("🎉 Camera centering completed successfully!");
                            }
                            None => {
                                error!("❌ Mesh not found in assets for handle: {:?}", mesh_handle);
                            }
                        }
                    } else {
                        warn!("⚠️ No meshes in cache");
                    }
                } else {
                    warn!("⚠️ No current mesh entity in cache");

                    // Try anyway if we have meshes in cache
                    if !mesh_cache.cache.is_empty() {
                        debug!("🔄 Trying to center on first mesh in cache anyway...");

                        if let Some((path, mesh_handle)) = mesh_cache.cache.iter().next() {
                            debug!("📁 Using mesh from path: {:?}", path);

                            if let Some(mesh) = meshes.get(mesh_handle) {
                                let centroid = calculate_mesh_centroid(mesh);
                                let bounds_radius = calculate_mesh_bounds_radius(mesh, centroid);
                                let camera_distance = bounds_radius * 2.5;
                                let camera_position = centroid
                                    + Vec3::new(
                                        camera_distance,
                                        camera_distance * 0.5,
                                        camera_distance,
                                    );

                                transform.translation = camera_position;
                                transform.look_at(centroid, Vec3::Y);
                                fps_camera.escape_mode = false;

                                debug!("🎉 Fallback camera centering completed!");
                            }
                        }
                    }
                }
            }
            Err(e) => {
                error!("❌ Camera query failed: {:?}", e);
            }
        }
        */
    }
}

/// Calculate mesh centroid using random sampling for performance
fn _calculate_mesh_centroid(mesh: &Mesh) -> Vec3 {
    if let Some(positions) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        match positions {
            bevy::mesh::VertexAttributeValues::Float32x3(vertices) => {
                if vertices.is_empty() {
                    return Vec3::ZERO;
                }

                // Use 5% sampling for large meshes, minimum 100 samples
                let sample_count = (vertices.len() / 20).max(100).min(vertices.len());
                let step = vertices.len() / sample_count;

                let mut sum = Vec3::ZERO;
                let mut count = 0;

                for (i, vertex) in vertices.iter().enumerate() {
                    if i % step == 0 {
                        sum += Vec3::from_array(*vertex);
                        count += 1;
                    }
                }

                if count > 0 {
                    sum / count as f32
                } else {
                    Vec3::ZERO
                }
            }
            _ => Vec3::ZERO,
        }
    } else {
        Vec3::ZERO
    }
}

/// Calculate approximate mesh bounds radius from centroid
fn _calculate_mesh_bounds_radius(mesh: &Mesh, centroid: Vec3) -> f32 {
    if let Some(positions) = mesh.attribute(Mesh::ATTRIBUTE_POSITION) {
        match positions {
            bevy::mesh::VertexAttributeValues::Float32x3(vertices) => {
                if vertices.is_empty() {
                    return 50.0; // Default fallback
                }

                let mut max_distance_squared: f32 = 0.0;
                let sample_count = (vertices.len() / 20).max(100).min(vertices.len());
                let step = vertices.len() / sample_count;

                for (i, vertex) in vertices.iter().enumerate() {
                    if i % step == 0 {
                        let vertex_pos = Vec3::from_array(*vertex);
                        let distance_squared = centroid.distance_squared(vertex_pos);
                        max_distance_squared = max_distance_squared.max(distance_squared);
                    }
                }

                max_distance_squared.sqrt().max(10.0) // Minimum 10 units
            }
            _ => 50.0, // Default fallback
        }
    } else {
        50.0 // Default fallback
    }
}

/// Debug system to log mesh cache status
/// TODO: Remove or reimplement after switching to Bevy asset loading
#[allow(dead_code)]
pub fn debug_mesh_cache_status(/* mesh_cache: Res<AsyncMeshCache>, */ meshes: Res<Assets<Mesh>>,) {
    /*
    if mesh_cache.is_changed() {
        debug!("🔍 MESH CACHE STATUS:");
        debug!(
            "  Current mesh entity: {:?}",
            mesh_cache.current_mesh_entity
        );
        debug!("  Cache entries: {}", mesh_cache.cache.len());

        for (path, handle) in &mesh_cache.cache {
            let mesh_exists = meshes.get(handle).is_some();
            debug!(
                "  - {:?} -> Handle {:?} (exists: {})",
                path, handle, mesh_exists
            );
        }
    }
    */
    let _ = meshes; // Silence unused warning
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
