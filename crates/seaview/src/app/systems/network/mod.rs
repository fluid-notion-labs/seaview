//! Network system for receiving mesh data in real-time

use bevy::prelude::*;

use std::sync::{Arc, Mutex};

use crate::lib::network::{
    NetworkConfig, NetworkMeshReceived, NonBlockingMeshReceiver, ReceivedMesh,
};
use baby_shark::mesh::Mesh as BabySharkMesh;
use nalgebra::Vector3;

pub struct NetworkMeshPlugin;

impl Plugin for NetworkMeshPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NetworkConfig>()
            .init_resource::<NetworkReceiver>()
            .add_message::<NetworkMeshReceived>()
            .add_systems(Startup, setup_network_receiver)
            .add_systems(Update, poll_network_meshes);
    }
}

/// Resource that holds the network receiver
#[derive(Resource, Default)]
pub struct NetworkReceiver {
    receiver: Option<Arc<Mutex<NonBlockingMeshReceiver>>>,
}

/// Component to mark entities that were created from network data
#[derive(Component)]
pub struct NetworkMesh {
    #[allow(dead_code)]
    pub simulation_uuid: String,
    #[allow(dead_code)]
    pub frame_number: u32,
}

fn setup_network_receiver(
    config: Res<NetworkConfig>,
    mut network_receiver: ResMut<NetworkReceiver>,
) {
    if !config.enabled {
        debug!("Network mesh receiving is disabled");
        return;
    }

    match NonBlockingMeshReceiver::new(config.port, config.max_message_size_mb) {
        Ok(receiver) => {
            debug!("Network mesh receiver listening on port {}", config.port);
            network_receiver.receiver = Some(Arc::new(Mutex::new(receiver)));
        }
        Err(e) => {
            error!(
                "Failed to start network receiver on port {}: {}",
                config.port, e
            );
        }
    }
}

fn poll_network_meshes(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    network_receiver: Res<NetworkReceiver>,
    mut mesh_received_events: MessageWriter<NetworkMeshReceived>,
) {
    if let Some(receiver) = &network_receiver.receiver {
        // Try to lock the receiver
        if let Ok(mut receiver_guard) = receiver.try_lock() {
            // Process available meshes
            loop {
                match receiver_guard.try_receive() {
                    Ok(Some(received_mesh)) => {
                        debug!(
                            "Received mesh via network: {} triangles for simulation {} frame {}",
                            received_mesh.triangle_count,
                            received_mesh.simulation_uuid,
                            received_mesh.frame_number
                        );

                        // Convert received mesh to Bevy mesh
                        match convert_to_bevy_mesh(&received_mesh) {
                            Ok(mesh) => {
                                let mesh_handle = meshes.add(mesh);

                                // Create a blue-ish material for network meshes
                                let material = materials.add(StandardMaterial {
                                    base_color: Color::srgb(0.3, 0.5, 0.8),
                                    metallic: 0.1,
                                    perceptual_roughness: 0.8,
                                    ..default()
                                });

                                // Spawn the mesh entity
                                let entity = commands
                                    .spawn((
                                        Mesh3d(mesh_handle),
                                        MeshMaterial3d(material),
                                        Transform::default(),
                                        NetworkMesh {
                                            simulation_uuid: received_mesh.simulation_uuid.clone(),
                                            frame_number: received_mesh.frame_number,
                                        },
                                    ))
                                    .id();

                                // Emit event
                                mesh_received_events.write(NetworkMeshReceived {
                                    entity,
                                    simulation_uuid: received_mesh.simulation_uuid,
                                    frame_number: received_mesh.frame_number,
                                    triangle_count: received_mesh.triangle_count,
                                });
                            }
                            Err(e) => {
                                error!("Failed to convert received mesh: {}", e);
                            }
                        }
                    }
                    Ok(None) => {
                        // No more meshes available
                        break;
                    }
                    Err(e) => {
                        error!("Error receiving mesh: {}", e);
                        break;
                    }
                }
            }
        }
    }
}

fn convert_to_bevy_mesh(received: &ReceivedMesh) -> Result<Mesh, String> {
    // Convert flat vertex array to Vector3 array
    let mut vertices = Vec::with_capacity(received.triangle_count as usize * 3);
    let mut indices = Vec::with_capacity(received.triangle_count as usize * 3);

    // Process each triangle
    for tri_idx in 0..received.triangle_count as usize {
        let base_idx = tri_idx * 9;

        // Extract three vertices for this triangle
        for vertex_offset in 0..3 {
            let idx = base_idx + vertex_offset * 3;
            vertices.push(Vector3::new(
                received.vertices[idx],
                received.vertices[idx + 1],
                received.vertices[idx + 2],
            ));
            indices.push(vertices.len() - 1);
        }
    }

    // Create baby_shark mesh
    let baby_shark_mesh = BabySharkMesh::new(vertices, indices);

    // Convert to Bevy mesh - baby_shark handles normals and UVs automatically
    let bevy_mesh: Mesh = baby_shark_mesh.into();

    Ok(bevy_mesh)
}
