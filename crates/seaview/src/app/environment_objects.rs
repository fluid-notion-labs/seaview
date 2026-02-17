//! Environment objects — Bevy integration
//!
//! All GLB/glTF **scene** loading lives in this one file.  The pure data model
//! is in [`crate::lib::environment_objects`].  When `loadfu` gains multi-object
//! scene support, only this module needs to change.
//!
//! # How it works
//!
//! 1. The UI (or file dialog) adds an [`EnvironmentObject`] to the
//!    [`EnvironmentObjectStore`] resource.
//! 2. [`spawn_new_objects`] detects the new entry (via generation counter),
//!    loads the GLB as a `Handle<Gltf>` through Bevy's built-in `bevy_gltf`
//!    loader, and spawns a [`SceneRoot`] entity tagged with
//!    [`EnvironmentObjectMarker`].
//! 3. [`discover_child_meshes`] runs once the scene is instantiated, walks the
//!    child entity tree, and populates the store's [`MeshEntry`] list.
//! 4. [`sync_visibility`] / [`sync_transforms`] / [`sync_mesh_visibility`]
//!    keep the ECS world in sync with the authoritative store data each frame.
//! 5. [`despawn_removed_objects`] cleans up entities when objects are removed
//!    from the store.
//!
//! # Asset source
//!
//! Environment objects can live anywhere on disk.  At app startup we register
//! an `"env"` asset source pointing to the filesystem root (`/`), so any
//! absolute path can be loaded as `env://absolute/path/to/file.glb`.

use std::collections::HashMap;
use std::path::PathBuf;

use bevy::gltf::Gltf;
use bevy::prelude::*;
use bevy::scene::{SceneInstance, SceneRoot};

use crate::lib::environment_objects::{
    EnvironmentObject, EnvironmentObjectId, EnvironmentObjectStore, MeshEntry,
};

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// Registers all environment-object resources and systems.
///
/// Add this plugin in `main.rs` alongside the other app plugins.
pub struct EnvironmentObjectPlugin;

impl Plugin for EnvironmentObjectPlugin {
    fn build(&self, app: &mut App) {
        // Register the "env" asset source pointing to filesystem root so we
        // can load GLBs from arbitrary absolute paths.
        app.register_asset_source(
            "env",
            bevy::asset::io::AssetSourceBuilder::platform_default("/", None),
        );

        app.insert_resource(EnvironmentObjectStoreRes::default())
            .insert_resource(SpawnedEnvironmentObjects::default())
            .insert_resource(PendingSceneDiscovery::default())
            .add_message::<ImportEnvironmentObjectEvent>()
            // Split into two system chains to stay within Bevy's tuple size
            // limits.  The first chain handles lifecycle (import → spawn →
            // scene attach → mesh discovery → despawn).  The second chain
            // handles per-frame sync (visibility, transforms, per-mesh vis).
            .add_systems(
                Update,
                (
                    handle_import_events,
                    spawn_new_objects,
                    poll_gltf_assets,
                    discover_child_meshes,
                    despawn_removed_objects,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (sync_visibility, sync_transforms, sync_mesh_visibility),
            );
    }
}

// ---------------------------------------------------------------------------
// Resources & Components
// ---------------------------------------------------------------------------

/// Newtype wrapper so `EnvironmentObjectStore` can be a Bevy `Resource`.
///
/// Deref/DerefMut give transparent access to all store methods.
#[derive(Resource, Debug, Clone, Default, Deref, DerefMut)]
pub struct EnvironmentObjectStoreRes(pub EnvironmentObjectStore);

/// Tracks spawned ECS entities for each environment object.
#[derive(Resource, Debug, Default)]
pub struct SpawnedEnvironmentObjects {
    /// Object ID → spawned state.
    pub map: HashMap<EnvironmentObjectId, SpawnedObject>,
    /// Last observed store generation (to detect additions/removals).
    last_generation: u64,
}

/// ECS state for one spawned environment object.
#[derive(Debug)]
pub struct SpawnedObject {
    /// The root entity that holds the `SceneRoot`.
    pub root_entity: Entity,
    /// Handle to the loaded `Gltf` asset (kept alive so the asset isn't dropped).
    pub gltf_handle: Handle<Gltf>,
    /// Child entities that carry `Mesh3d` (populated by [`discover_child_meshes`]).
    pub mesh_entities: Vec<Entity>,
    /// Whether child-mesh discovery has completed.
    pub meshes_discovered: bool,
}

/// Objects whose `Gltf` asset has loaded but whose scene hasn't been spawned yet.
///
/// We need to wait for the `Gltf` asset to be ready so we can extract the
/// default scene handle before spawning `SceneRoot`.
#[derive(Resource, Debug, Default)]
struct PendingSceneDiscovery {
    /// Objects waiting for their scene to be instantiated.
    awaiting_scene: Vec<EnvironmentObjectId>,
}

/// Marker component on environment-object root entities.
#[derive(Component, Debug, Clone)]
pub struct EnvironmentObjectMarker {
    pub id: EnvironmentObjectId,
}

/// Message fired to request importing a new environment object from a file path.
///
/// The UI / file-dialog system fires this; the plugin handles the rest.
#[derive(Message, Debug, Clone)]
pub struct ImportEnvironmentObjectEvent {
    pub path: PathBuf,
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Converts [`ImportEnvironmentObjectEvent`] messages into store entries.
fn handle_import_events(
    mut events: MessageReader<ImportEnvironmentObjectEvent>,
    mut store: ResMut<EnvironmentObjectStoreRes>,
) {
    for event in events.read() {
        let obj = EnvironmentObject::new(&event.path);
        info!(
            "Importing environment object '{}' from {:?}",
            obj.name, obj.path
        );
        store.add(obj);
    }
}

/// Detects new objects in the store and kicks off Gltf asset loads.
///
/// For each object that doesn't have a spawned entry yet, we:
/// 1. Start loading the `Gltf` asset via Bevy's built-in `bevy_gltf` loader.
/// 2. Create a placeholder entry in `SpawnedEnvironmentObjects`.
///
/// The actual `SceneRoot` spawn happens in [`poll_gltf_assets`] once the
/// `Gltf` asset is ready, because we need the scene handle from the loaded
/// asset.
fn spawn_new_objects(
    store: Res<EnvironmentObjectStoreRes>,
    mut spawned: ResMut<SpawnedEnvironmentObjects>,
    mut commands: Commands,
    asset_server: Res<AssetServer>,
) {
    let current_gen = store.generation();
    if current_gen == spawned.last_generation {
        return;
    }
    spawned.last_generation = current_gen;

    for obj in store.iter() {
        if spawned.map.contains_key(&obj.id) {
            continue;
        }

        // Build the asset path through the "env" source so we can load from
        // any absolute path on disk.
        let abs_path = match obj.path.canonicalize() {
            Ok(p) => p,
            Err(e) => {
                warn!(
                    "Cannot resolve path for '{}': {:?} — {}",
                    obj.name, obj.path, e
                );
                continue;
            }
        };

        // Strip the leading `/` so the "env" source (rooted at `/`) resolves
        // it correctly:  env://home/user/models/file.glb
        let rel = abs_path
            .strip_prefix("/")
            .unwrap_or(&abs_path)
            .to_string_lossy();
        let asset_path = format!("env://{rel}");

        debug!("Loading Gltf asset for '{}': {}", obj.name, asset_path);
        let gltf_handle: Handle<Gltf> = asset_server.load(&asset_path);

        // Spawn a root entity now (invisible until the scene is attached).
        let transform = bevy_transform_from(&obj.transform);
        let root_entity = commands
            .spawn((
                EnvironmentObjectMarker { id: obj.id },
                transform,
                Visibility::Inherited,
                Name::new(format!("EnvObj:{}", obj.name)),
            ))
            .id();

        spawned.map.insert(
            obj.id,
            SpawnedObject {
                root_entity,
                gltf_handle: gltf_handle.clone(),
                mesh_entities: Vec::new(),
                meshes_discovered: false,
            },
        );
    }
}

/// Polls pending `Gltf` assets and spawns `SceneRoot` once ready.
///
/// We can't spawn the scene at the same time we start the asset load because
/// we need the loaded `Gltf` to extract the default (or first) scene handle.
fn poll_gltf_assets(
    mut spawned: ResMut<SpawnedEnvironmentObjects>,
    mut pending: ResMut<PendingSceneDiscovery>,
    gltf_assets: Res<Assets<Gltf>>,
    mut commands: Commands,
) {
    for (id, state) in spawned.map.iter_mut() {
        // Skip if already discovered or already waiting for scene instantiation.
        if state.meshes_discovered || pending.awaiting_scene.contains(id) {
            continue;
        }

        let gltf = match gltf_assets.get(&state.gltf_handle) {
            Some(g) => g,
            None => continue, // Still loading.
        };

        let scene_handle = gltf
            .default_scene
            .clone()
            .or_else(|| gltf.scenes.first().cloned());

        let Some(scene_handle) = scene_handle else {
            warn!("Gltf asset for env object {id} has no scenes");
            continue;
        };

        debug!("Spawning SceneRoot for env object {id}");
        commands
            .entity(state.root_entity)
            .insert(SceneRoot(scene_handle));

        pending.awaiting_scene.push(*id);
    }
}

/// After a scene is fully instantiated, walk the entity tree to discover child
/// meshes and populate the store's [`MeshEntry`] list.
fn discover_child_meshes(
    mut store: ResMut<EnvironmentObjectStoreRes>,
    mut spawned: ResMut<SpawnedEnvironmentObjects>,
    mut pending: ResMut<PendingSceneDiscovery>,
    children_query: Query<&Children>,
    mesh_query: Query<(Entity, Option<&Name>), With<Mesh3d>>,
    scene_instance_query: Query<&SceneInstance>,
) {
    // Only process objects that are awaiting discovery.
    pending.awaiting_scene.retain(|id| {
        let Some(state) = spawned.map.get_mut(id) else {
            return false; // Object was removed.
        };
        if state.meshes_discovered {
            return false; // Already done.
        }

        // The scene is ready when SceneInstance is present on the root entity.
        if scene_instance_query.get(state.root_entity).is_err() {
            return true; // Keep waiting.
        }

        // Walk descendants to find all entities with Mesh3d.
        let mut mesh_entities = Vec::new();
        let mut mesh_entries = Vec::new();
        collect_mesh_children(
            state.root_entity,
            &children_query,
            &mesh_query,
            &mut mesh_entities,
            &mut mesh_entries,
        );

        debug!(
            "Discovered {} meshes for env object {id}",
            mesh_entities.len()
        );

        state.mesh_entities = mesh_entities;
        state.meshes_discovered = true;

        // Write mesh entries back to the store.
        if let Some(obj) = store.get_mut(*id) {
            obj.meshes = mesh_entries;
        }
        store.bump_generation();

        false // Remove from pending.
    });
}

/// Recursively collect entities with `Mesh3d` in the hierarchy.
fn collect_mesh_children(
    entity: Entity,
    children_query: &Query<&Children>,
    mesh_query: &Query<(Entity, Option<&Name>), With<Mesh3d>>,
    mesh_entities: &mut Vec<Entity>,
    mesh_entries: &mut Vec<MeshEntry>,
) {
    // Check if this entity itself has a mesh.
    if let Ok((e, name)) = mesh_query.get(entity) {
        let display_name = name
            .map(|n| n.as_str().to_string())
            .unwrap_or_else(|| format!("Mesh {}", mesh_entities.len()));
        mesh_entities.push(e);
        mesh_entries.push(MeshEntry::new(display_name));
    }

    // Recurse into children.
    if let Ok(children) = children_query.get(entity) {
        for child in children.iter() {
            collect_mesh_children(
                child,
                children_query,
                mesh_query,
                mesh_entities,
                mesh_entries,
            );
        }
    }
}

/// Remove ECS entities for objects that have been deleted from the store.
fn despawn_removed_objects(
    store: Res<EnvironmentObjectStoreRes>,
    mut spawned: ResMut<SpawnedEnvironmentObjects>,
    mut commands: Commands,
) {
    let store_ids: std::collections::HashSet<_> = store.ids().into_iter().collect();
    let spawned_ids: Vec<_> = spawned.map.keys().copied().collect();

    for id in spawned_ids {
        if !store_ids.contains(&id) {
            if let Some(state) = spawned.map.remove(&id) {
                debug!("Despawning env object {id}");
                // Despawning the root recursively removes all children (scene entities).
                if let Ok(mut entity_commands) = commands.get_entity(state.root_entity) {
                    entity_commands.despawn();
                }
            }
        }
    }
}

/// Sync master visibility from the store to ECS entities.
fn sync_visibility(
    store: Res<EnvironmentObjectStoreRes>,
    spawned: Res<SpawnedEnvironmentObjects>,
    mut visibility_query: Query<&mut Visibility>,
) {
    for obj in store.iter() {
        let Some(state) = spawned.map.get(&obj.id) else {
            continue;
        };
        if let Ok(mut vis) = visibility_query.get_mut(state.root_entity) {
            let desired = if obj.visible {
                Visibility::Inherited
            } else {
                Visibility::Hidden
            };
            if *vis != desired {
                *vis = desired;
            }
        }
    }
}

/// Sync object transforms from the store to ECS entities.
fn sync_transforms(
    store: Res<EnvironmentObjectStoreRes>,
    spawned: Res<SpawnedEnvironmentObjects>,
    mut transform_query: Query<&mut Transform>,
) {
    for obj in store.iter() {
        let Some(state) = spawned.map.get(&obj.id) else {
            continue;
        };
        if let Ok(mut transform) = transform_query.get_mut(state.root_entity) {
            let desired = bevy_transform_from(&obj.transform);
            if *transform != desired {
                *transform = desired;
            }
        }
    }
}

/// Sync per-mesh visibility from the store to individual mesh child entities.
fn sync_mesh_visibility(
    store: Res<EnvironmentObjectStoreRes>,
    spawned: Res<SpawnedEnvironmentObjects>,
    mut visibility_query: Query<&mut Visibility>,
) {
    for obj in store.iter() {
        let Some(state) = spawned.map.get(&obj.id) else {
            continue;
        };
        if !state.meshes_discovered {
            continue;
        }
        for (i, mesh_entity) in state.mesh_entities.iter().enumerate() {
            let Some(mesh_entry) = obj.meshes.get(i) else {
                continue;
            };
            if let Ok(mut vis) = visibility_query.get_mut(*mesh_entity) {
                let desired = if mesh_entry.visible {
                    Visibility::Inherited
                } else {
                    Visibility::Hidden
                };
                if *vis != desired {
                    *vis = desired;
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a lib-side [`Transform3D`] to a Bevy [`Transform`].
fn bevy_transform_from(t: &crate::lib::environment_objects::Transform3D) -> Transform {
    Transform {
        translation: Vec3::new(t.position[0], t.position[1], t.position[2]),
        rotation: Quat::from_euler(
            EulerRot::XYZ,
            t.rotation_degrees[0].to_radians(),
            t.rotation_degrees[1].to_radians(),
            t.rotation_degrees[2].to_radians(),
        ),
        scale: Vec3::new(t.scale[0], t.scale[1], t.scale[2]),
    }
}
