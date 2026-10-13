//! Environment objects — Bevy integration
//!
//! Supports **GLB/glTF** (full scenes with materials) and **STL/OBJ/PLY**
//! (single meshes loaded via loadfu).  The pure data model is in
//! [`crate::lib::environment_objects`].
//!
//! # How it works
//!
//! 1. The UI (or file dialog) adds an [`EnvironmentObject`] to the
//!    [`EnvironmentObjectStore`] resource.
//! 2. [`spawn_new_objects`] detects the new entry (via generation counter),
//!    checks the file extension, and starts loading the appropriate asset:
//!    - **GLB/glTF** → `Handle<Gltf>` via Bevy's built-in `bevy_gltf` loader
//!    - **STL/OBJ/PLY** → `Handle<Mesh>` via loadfu's asset loaders
//! 3. For GLB/glTF: [`poll_gltf_assets`] spawns a [`SceneRoot`] once the Gltf
//!    asset is ready, then [`discover_child_meshes`] walks the entity tree.
//!    For direct-mesh formats: [`poll_direct_mesh_assets`] spawns a child
//!    entity with `Mesh3d` + default material once the mesh is loaded.
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

use bevy::asset::RenderAssetUsages;
use bevy::gltf::Gltf;
use bevy::mesh::VertexAttributeValues;
use bevy::prelude::*;
use bevy::world_serialization::{WorldAssetRoot, WorldInstance};

/// File extensions that go through loadfu's direct-mesh pipeline.
const DIRECT_MESH_EXTENSIONS: &[&str] = &["stl", "obj", "ply"];

/// File extensions that go through Bevy's Gltf scene pipeline.
const GLTF_EXTENSIONS: &[&str] = &["glb", "gltf"];

use crate::lib::environment_objects::{
    EnvironmentObject, EnvironmentObjectId, EnvironmentObjectStore, MaterialOverride, MeshEntry,
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
        // NOTE: The "env" asset source (filesystem root "/") is registered in
        // main.rs *before* DefaultPlugins, because AssetPlugin finalises
        // sources during its own build and ignores late registrations.

        app.init_resource::<EnvironmentObjectStoreRes>()
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
                    poll_direct_mesh_assets,
                    discover_child_meshes,
                    despawn_removed_objects,
                )
                    .chain(),
            )
            .add_systems(
                Update,
                (
                    sync_visibility,
                    sync_transforms,
                    sync_mesh_visibility,
                    sync_materials,
                ),
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

/// Tracks which kind of asset was loaded for an environment object.
#[derive(Debug)]
pub enum LoadedAsset {
    /// GLB/glTF: full scene with materials, textures, hierarchy.
    Gltf(Handle<Gltf>),
    /// STL/OBJ/PLY: single mesh loaded directly via loadfu.
    DirectMesh(Handle<Mesh>),
}

/// ECS state for one spawned environment object.
#[derive(Debug)]
pub struct SpawnedObject {
    /// The root entity (parent of all mesh children).
    pub root_entity: Entity,
    /// The loaded asset handle — kept alive so Bevy doesn't drop it.
    pub loaded_asset: LoadedAsset,
    /// Child entities that carry `Mesh3d` (populated by [`discover_child_meshes`]
    /// or [`poll_direct_mesh_assets`]).
    pub mesh_entities: Vec<Entity>,
    /// Whether child-mesh discovery has completed.
    pub meshes_discovered: bool,
    /// Original material handles per mesh entity (saved on first discovery
    /// so we can restore them when switching back to `MaterialOverride::Default`).
    pub original_materials: Vec<Option<Handle<StandardMaterial>>>,
    /// The last applied `MaterialOverride` — used to detect changes and avoid
    /// re-creating material assets every frame.
    pub last_applied_override: Option<MaterialOverride>,
    /// Last applied flip_normals state.
    pub last_flip_normals: Option<bool>,
    /// Last applied double_sided state.
    pub last_double_sided: Option<bool>,
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
        let id = store.add(obj);
        info!(
            "  → Added to store with id={id}, generation={}",
            store.generation()
        );
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

        // Determine pipeline from file extension.
        let ext = obj
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        let loaded_asset = if DIRECT_MESH_EXTENSIONS.contains(&ext.as_str()) {
            info!(
                "Loading direct mesh ({ext}) for '{}': {asset_path}",
                obj.name
            );
            let mesh_handle: Handle<Mesh> = asset_server.load(&asset_path);
            LoadedAsset::DirectMesh(mesh_handle)
        } else if GLTF_EXTENSIONS.contains(&ext.as_str()) {
            info!(
                "Loading Gltf scene ({ext}) for '{}': {asset_path}",
                obj.name
            );
            let gltf_handle: Handle<Gltf> = asset_server.load(&asset_path);
            LoadedAsset::Gltf(gltf_handle)
        } else {
            warn!(
                "Unsupported file extension '.{ext}' for environment object '{}' — \
                 supported: .glb, .gltf, .stl, .obj, .ply",
                obj.name
            );
            continue;
        };

        // Spawn a root entity now (invisible until the mesh/scene is attached).
        let transform = bevy_transform_from(&obj.transform, &obj.swizzle);
        let root_entity = commands
            .spawn((
                EnvironmentObjectMarker { id: obj.id },
                transform,
                Visibility::Inherited,
                Name::new(format!("EnvObj:{}", obj.name)),
            ))
            .id();

        info!(
            "  → Spawned root entity {:?} for '{}' (id={}), transform={:?}",
            root_entity, obj.name, obj.id, transform
        );

        spawned.map.insert(
            obj.id,
            SpawnedObject {
                root_entity,
                loaded_asset,
                mesh_entities: Vec::new(),
                meshes_discovered: false,
                original_materials: Vec::new(),
                last_applied_override: None,
                last_flip_normals: None,
                last_double_sided: None,
            },
        );
    }
}

/// Polls pending `Gltf` assets and spawns `SceneRoot` once ready.
///
/// Only processes objects loaded via the GLB/glTF pipeline ([`LoadedAsset::Gltf`]).
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

        // Only handle Gltf assets here; direct meshes are handled by
        // poll_direct_mesh_assets.
        let LoadedAsset::Gltf(ref gltf_handle) = state.loaded_asset else {
            continue;
        };

        let gltf = match gltf_assets.get(gltf_handle) {
            Some(g) => g,
            None => {
                debug!("Gltf asset for env object {id} still loading...");
                continue;
            }
        };

        info!(
            "Gltf asset loaded for env object {id}: {} scene(s), default_scene={}, {} meshes, {} materials, {} nodes",
            gltf.scenes.len(),
            gltf.default_scene.is_some(),
            gltf.meshes.len(),
            gltf.materials.len(),
            gltf.nodes.len(),
        );

        let scene_handle = gltf
            .default_scene
            .clone()
            .or_else(|| gltf.scenes.first().cloned());

        let Some(scene_handle) = scene_handle else {
            warn!("Gltf asset for env object {id} has no scenes — nothing to render!");
            continue;
        };

        info!(
            "Spawning SceneRoot for env object {id} on entity {:?}",
            state.root_entity
        );
        commands
            .entity(state.root_entity)
            .insert(WorldAssetRoot(scene_handle));

        pending.awaiting_scene.push(*id);
    }
}

/// Polls pending direct-mesh assets (STL/OBJ/PLY) and spawns `Mesh3d` +
/// default material once the mesh is loaded.
///
/// Unlike the GLB pipeline which goes through SceneRoot → child discovery,
/// direct meshes are simple: one `Handle<Mesh>` → one child entity with
/// `Mesh3d` + `MeshMaterial3d`.
fn poll_direct_mesh_assets(
    mut store: ResMut<EnvironmentObjectStoreRes>,
    mut spawned: ResMut<SpawnedEnvironmentObjects>,
    mut mesh_assets: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut commands: Commands,
) {
    for (id, state) in spawned.map.iter_mut() {
        if state.meshes_discovered {
            continue;
        }

        let LoadedAsset::DirectMesh(ref mesh_handle) = state.loaded_asset else {
            continue;
        };

        // Wait until the mesh asset is actually loaded.
        let Some(mesh) = mesh_assets.get(mesh_handle) else {
            debug!("Direct mesh asset for env object {id} still loading...");
            continue;
        };

        // ── Diagnostic: mesh stats ────────────────────────────────
        let vertex_count = mesh.count_vertices();
        let index_count = mesh.indices().map(|i| i.len()).unwrap_or(0);
        // Compute a rough AABB from positions if available.
        let bounds_info = mesh
            .attribute(Mesh::ATTRIBUTE_POSITION)
            .and_then(|attr| {
                if let VertexAttributeValues::Float32x3(positions) = attr {
                    let (mut mn, mut mx) = (Vec3::splat(f32::MAX), Vec3::splat(f32::MIN));
                    for p in positions {
                        let v = Vec3::from(*p);
                        mn = mn.min(v);
                        mx = mx.max(v);
                    }
                    Some(format!("min={mn:?}, max={mx:?}, size={:?}", mx - mn))
                } else {
                    None
                }
            })
            .unwrap_or_else(|| "no positions".to_string());
        info!(
            "Direct mesh loaded for env object {id}: \
             {vertex_count} vertices, {index_count} indices, \
             bounds=[{bounds_info}]"
        );
        if vertex_count == 0 {
            warn!("  ⚠ Mesh has 0 vertices — nothing to render!");
        }

        // Loadfu loads meshes with MAIN_WORLD only (CPU-resident).
        // Promote to GPU so the render pipeline can actually draw it.
        if let Some(mut mesh) = mesh_assets.get_mut(mesh_handle) {
            mesh.asset_usage = RenderAssetUsages::MAIN_WORLD | RenderAssetUsages::RENDER_WORLD;
        }

        // Create a default material (light grey, slightly rough).
        let default_material = materials.add(StandardMaterial {
            base_color: Color::LinearRgba(LinearRgba::new(0.7, 0.7, 0.7, 1.0)),
            perceptual_roughness: 0.5,
            metallic: 0.0,
            ..default()
        });

        // Spawn a child entity with the mesh + material under the root.
        let mesh_entity = commands
            .spawn((
                Mesh3d(mesh_handle.clone()),
                MeshMaterial3d(default_material),
                Transform::default(),
                Visibility::Inherited,
                Name::new(format!("EnvObjMesh:{id}")),
            ))
            .id();

        commands.entity(state.root_entity).add_child(mesh_entity);

        // ── Diagnostic: root entity state ─────────────────────────
        if let Some(obj) = store.get(*id) {
            let transform = bevy_transform_from(&obj.transform, &obj.swizzle);
            info!(
                "  Root entity {:?}: visible={}, transform={:?}, \
                 swizzle_identity={}",
                state.root_entity,
                obj.visible,
                transform,
                obj.swizzle.is_identity(),
            );
        }

        state.mesh_entities = vec![mesh_entity];
        state.meshes_discovered = true;

        // Build the display name from the object's filename.
        let mesh_name = if let Some(obj) = store.get(*id) {
            obj.path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Mesh 0")
                .to_string()
        } else {
            "Mesh 0".to_string()
        };

        // Populate the store's mesh entries (single mesh).
        if let Some(obj) = store.get_mut(*id) {
            if obj.meshes.is_empty() {
                obj.meshes = vec![MeshEntry::new(mesh_name)];
            }
            // If meshes were already populated (e.g. from saved settings),
            // keep them as-is — the visibility state is already correct.
        }
        store.bump_generation();

        info!(
            "  → Direct mesh child {:?} spawned for env object {id}",
            mesh_entity
        );
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
    scene_instance_query: Query<&WorldInstance>,
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
            debug!("SceneInstance not yet present on root entity {:?} for env object {id}", state.root_entity);
            return true; // Keep waiting.
        }

        info!("SceneInstance ready for env object {id} (root={:?}), discovering meshes...", state.root_entity);

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

        if mesh_entities.is_empty() {
            warn!(
                "Discovered 0 meshes for env object {id} — scene may be empty or meshes not yet spawned"
            );
        } else {
            info!(
                "Discovered {} meshes for env object {id}: {:?}",
                mesh_entities.len(),
                mesh_entries.iter().map(|m| &m.name).collect::<Vec<_>>()
            );
        }

        state.mesh_entities = mesh_entities;
        state.meshes_discovered = true;

        // Write mesh entries back to the store, preserving any saved
        // visibility state from seaview.toml.  Match by name so that
        // meshes whose visibility was persisted keep their saved state.
        if let Some(obj) = store.get_mut(*id) {
            if obj.meshes.is_empty() {
                // No saved state — use freshly-discovered entries as-is.
                obj.meshes = mesh_entries;
            } else {
                // Merge: carry over saved visibility for matching names,
                // default to visible for newly-discovered meshes.
                let saved: std::collections::HashMap<&str, bool> = obj
                    .meshes
                    .iter()
                    .map(|m| (m.name.as_str(), m.visible))
                    .collect();
                for entry in &mut mesh_entries {
                    if let Some(&vis) = saved.get(entry.name.as_str()) {
                        entry.visible = vis;
                    }
                }
                obj.meshes = mesh_entries;
            }
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
            let desired = bevy_transform_from(&obj.transform, &obj.swizzle);
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

/// Sync material overrides, flip_normals, and double_sided from the store
/// to mesh child entities.
///
/// When `MaterialOverride::Default`, restores the original GLB materials
/// (but still applies flip_normals / double_sided on top).
/// For `Preset` or `Custom`, replaces all mesh materials with a flat
/// `StandardMaterial` built from the resolved values.
fn sync_materials(
    store: Res<EnvironmentObjectStoreRes>,
    mut spawned: ResMut<SpawnedEnvironmentObjects>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    material_query: Query<&MeshMaterial3d<StandardMaterial>>,
    mut commands: Commands,
) {
    for obj in store.iter() {
        let Some(state) = spawned.map.get_mut(&obj.id) else {
            continue;
        };
        if !state.meshes_discovered || state.mesh_entities.is_empty() {
            continue;
        }

        // Snapshot original materials on first encounter.
        if state.original_materials.is_empty() {
            state.original_materials = state
                .mesh_entities
                .iter()
                .map(|e| material_query.get(*e).ok().map(|mat| mat.0.clone()))
                .collect();
        }

        let override_changed = state.last_applied_override.as_ref() != Some(&obj.material_override);
        let normals_changed = state.last_flip_normals != Some(obj.flip_normals);
        let sided_changed = state.last_double_sided != Some(obj.double_sided);

        if !override_changed && !normals_changed && !sided_changed {
            continue;
        }

        // Determine cull mode from flip_normals and double_sided.
        let cull_mode = if obj.double_sided || obj.flip_normals {
            None // Disable backface culling
        } else {
            Some(bevy::render::render_resource::Face::Back)
        };

        match obj.material_override.resolve() {
            None => {
                // Restore original materials, but apply normals/sided overrides.
                for (i, entity) in state.mesh_entities.iter().enumerate() {
                    if let Some(Some(original)) = state.original_materials.get(i) {
                        if normals_changed || sided_changed {
                            // Clone the original material and patch cull_mode +
                            // double_sided.
                            if let Some(orig_mat) = materials.get(original) {
                                let mut patched = orig_mat.clone();
                                patched.cull_mode = cull_mode;
                                patched.double_sided = obj.flip_normals || obj.double_sided;
                                let handle = materials.add(patched);
                                commands.entity(*entity).insert(MeshMaterial3d(handle));
                            }
                        } else {
                            commands
                                .entity(*entity)
                                .insert(MeshMaterial3d(original.clone()));
                        }
                    }
                }
            }
            Some((color, roughness, metallic)) => {
                let override_material = materials.add(StandardMaterial {
                    base_color: Color::LinearRgba(LinearRgba::new(
                        color[0], color[1], color[2], color[3],
                    )),
                    perceptual_roughness: roughness,
                    metallic,
                    alpha_mode: if color[3] < 1.0 {
                        AlphaMode::Blend
                    } else {
                        AlphaMode::Opaque
                    },
                    cull_mode,
                    double_sided: obj.flip_normals || obj.double_sided,
                    ..default()
                });
                for entity in &state.mesh_entities {
                    commands
                        .entity(*entity)
                        .insert(MeshMaterial3d(override_material.clone()));
                }
            }
        }

        state.last_applied_override = Some(obj.material_override.clone());
        state.last_flip_normals = Some(obj.flip_normals);
        state.last_double_sided = Some(obj.double_sided);
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Convert a lib-side [`Transform3D`] + [`AxisSwizzle`] to a Bevy [`Transform`].
///
/// The swizzle is applied *first* (as a basis change), then position/rotation/scale
/// are composed on top.  This lets users fix axis conventions and then fine-tune
/// placement with the normal transform controls.
fn bevy_transform_from(
    t: &crate::lib::environment_objects::Transform3D,
    swizzle: &crate::lib::environment_objects::AxisSwizzle,
) -> Transform {
    let user_transform = Transform {
        translation: Vec3::new(t.position[0], t.position[1], t.position[2]),
        rotation: Quat::from_euler(
            EulerRot::XYZ,
            t.rotation_degrees[0].to_radians(),
            t.rotation_degrees[1].to_radians(),
            t.rotation_degrees[2].to_radians(),
        ),
        scale: Vec3::new(t.scale[0], t.scale[1], t.scale[2]),
    };

    if swizzle.is_identity() {
        return user_transform;
    }

    // Build the swizzle as a 3×3 signed permutation matrix (det = ±1).
    let m = swizzle.to_matrix();
    let swizzle_mat = Mat3::from_cols(
        Vec3::new(m[0][0], m[0][1], m[0][2]),
        Vec3::new(m[1][0], m[1][1], m[1][2]),
        Vec3::new(m[2][0], m[2][1], m[2][2]),
    );

    // Decompose into rotation (quaternion) + scale.
    // det = +1 → proper rotation, Quat::from_mat3 works directly.
    // det = -1 → improper rotation (reflection); absorb one axis flip into
    //            scale so the remaining matrix is a proper rotation.
    let det = swizzle_mat.determinant();
    let (rot_mat, swizzle_scale) = if det < 0.0 {
        let flipped = Mat3::from_cols(swizzle_mat.col(0), swizzle_mat.col(1), -swizzle_mat.col(2));
        (flipped, Vec3::new(1.0, 1.0, -1.0))
    } else {
        (swizzle_mat, Vec3::ONE)
    };
    let swizzle_rot = Quat::from_mat3(&rot_mat);

    // Compose: user_transform * swizzle_transform
    // Swizzle is applied to the mesh first, then user transform on top.
    Transform {
        translation: user_transform.translation,
        rotation: user_transform.rotation * swizzle_rot,
        scale: user_transform.scale * swizzle_scale,
    }
}
