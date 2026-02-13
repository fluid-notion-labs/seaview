//! Session management module for Seaview
//!
//! This module handles the creation, storage, and management of mesh viewing sessions.
//! A session represents a collection of mesh frames from a specific source (network, file, etc.)
//! along with metadata and playback state.

use bevy::prelude::*;

use uuid::Uuid;

pub mod manager;
pub mod types;

pub use manager::SessionManager;
pub use types::*;

/// Plugin that adds session management functionality
pub struct SessionPlugin;

impl Plugin for SessionPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<SessionManager>()
            .add_message::<SessionCreatedEvent>()
            .add_message::<SessionUpdatedEvent>()
            .add_message::<SessionDeletedEvent>()
            .add_message::<FrameReceivedEvent>()
            .add_systems(
                Update,
                (handle_create_session_requests, update_session_frame_counts),
            );
    }
}

/// Event emitted when a new session is created
#[derive(Message)]
pub struct SessionCreatedEvent {
    pub session_id: Uuid,
}

/// Event emitted when a session is updated
#[derive(Message)]
pub struct SessionUpdatedEvent {
    pub session_id: Uuid,
}

/// Event emitted when a session is deleted
#[derive(Message)]
pub struct SessionDeletedEvent {
    pub session_id: Uuid,
}

/// Event emitted when a new frame is received for a session
#[derive(Message)]
pub struct FrameReceivedEvent {
    pub session_id: Uuid,
    pub frame_index: usize,
}

/// System that handles UI requests to create new sessions
fn handle_create_session_requests(
    mut create_events: MessageReader<crate::app::ui::state::CreateSessionEvent>,
    mut session_manager: ResMut<SessionManager>,
    mut created_events: MessageWriter<SessionCreatedEvent>,
) {
    for event in create_events.read() {
        match &event.source_type {
            crate::app::ui::state::SessionSourceType::Network { port } => {
                match session_manager.create_network_session(&event.name, *port) {
                    Ok(session_id) => {
                        debug!("Created network session '{}' on port {}", event.name, port);
                        created_events.write(SessionCreatedEvent { session_id });
                    }
                    Err(e) => {
                        error!("Failed to create network session: {}", e);
                    }
                }
            }
            crate::app::ui::state::SessionSourceType::File { path } => {
                match session_manager.create_file_session(&event.name, path.clone()) {
                    Ok(session_id) => {
                        debug!("Created file session '{}' for path {:?}", event.name, path);
                        created_events.write(SessionCreatedEvent { session_id });
                    }
                    Err(e) => {
                        error!("Failed to create file session: {}", e);
                    }
                }
            }
            crate::app::ui::state::SessionSourceType::DataLake { connection_string } => {
                warn!(
                    "Data lake sessions not yet implemented: {}",
                    connection_string
                );
            }
        }
    }
}

/// System that bridges NetworkMeshReceived events to our session system
// fn bridge_network_to_session(
//     mut network_events: MessageReader<crate::lib::network::NetworkMeshReceived>,
//     mut session_manager: ResMut<SessionManager>,
//     mut frame_events: MessageWriter<FrameReceivedEvent>,
//     mut commands: Commands,
//     mesh_query: Query<&Mesh3d>,
//     meshes: Res<Assets<Mesh>>,
//     network_config: Res<crate::lib::network::NetworkConfig>,
// ) {
//     for event in network_events.read() {
//         // Find the session associated with the network port
//         let session_id = session_manager
//             .find_session_by_port(network_config.port)
//             .or_else(|| {
//                 // Auto-create a session if none exists for this port
//                 let name = format!("Network Stream (Port {})", network_config.port);
//                 session_manager
//                     .create_network_session(&name, network_config.port)
//                     .ok()
//             });

//         if let Some(session_id) = session_id {
//             // Get the mesh from the entity that was already spawned
//             if let Ok(mesh_handle) = mesh_query.get(event.entity) {
//                 if let Some(mesh) = meshes.get(&mesh_handle.0) {
//                     // Add the mesh to the session
//                     let frame_index = session_manager
//                         .add_mesh_to_session(session_id, mesh.clone())
//                         .unwrap_or(0);

//                     // Add our session marker to the existing entity
//                     commands.entity(event.entity).insert(SessionMeshMarker {
//                         session_id,
//                         frame_index,
//                     });

//                     frame_events.write(FrameReceivedEvent {
//                         session_id,
//                         frame_index,
//                     });

//                     info!(
//                         "Added mesh to session {} (frame {}) from network",
//                         session_id, frame_index
//                     );
//                 }
//             }
//         } else {
//             warn!(
//                 "No session found for port {} and failed to auto-create",
//                 network_config.port
//             );
//         }
//     }
// }

/// Component marker for meshes that belong to a session
#[derive(Component)]
pub struct SessionMeshMarker {
    pub session_id: Uuid,
    pub frame_index: usize,
}

/// System that updates session frame counts in the UI
fn update_session_frame_counts(
    session_manager: Res<SessionManager>,
    mut ui_state: ResMut<crate::app::ui::state::UiState>,
    frame_events: MessageReader<FrameReceivedEvent>,
) {
    if !frame_events.is_empty() {
        // Update total frames for active session
        if let Some(active_id) = ui_state.active_session {
            if let Some(session) = session_manager.get_session(active_id) {
                ui_state.playback.total_frames = session.frame_count();
            }
        }
    }
}
