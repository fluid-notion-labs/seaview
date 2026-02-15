# Mesh Loading: Bypassing Bevy's Asset System

## Problem Statement

Seaview loads thousands of mesh frames in a sequence, streaming them in/out based on a timeline and memory budget. Bevy's asset system is designed for game assets — load once, cache, reference by handle. These are fundamentally different use cases.

**Current pain points:**

1. **No Draco support** — Bevy 0.18's `bevy_gltf` uses the `gltf` crate v1.4 but does NOT enable `KHR_draco_mesh_compression`. The crate itself has no Draco feature — Draco needs a separate C++ decoder library. Files compressed from 20MB → 3MB with Draco simply won't load.

2. **Format lock-in** — Adding any new format (Draco GLB, svmesh, quantized meshes) requires either patching vendored Bevy or writing `AssetLoader` impls that fight the system (extension conflicts, load state tracking, label extraction).

3. **Indirection overhead** — For GLB, the path is: `asset_server.load()` → async IO → `bevy_gltf` parses full glTF scene → extract `Mesh` via `GltfAssetLabel::Primitive { mesh: 0, primitive: 0 }`. We only ever need one mesh per file.

4. **Lifecycle mismatch** — Seaview already manages its own load/unload lifecycle via timeline sections, budget enforcement, and throttled queuing. Bevy's asset reference counting and change detection are redundant layers we work around.

## Current Pipeline

```
handle_load_requests    — Receives LoadSequenceRequest, detects format, inits timeline, enqueues indices
        ↓
sync_section_loads      — When timeline changes, enqueues new frames, drops out-of-section handles
        ↓
drain_load_queue        — 32/frame, calls asset_server.load() → Handle<Mesh>
        ↓
track_asset_loading     — Listens to AssetEvent<Mesh> for completion, estimates memory
        ↓
manage_frame_window     — Spawns/despawns entities based on spawn windows, 8/frame
```

**Where Bevy's asset system is involved:**

- `drain_load_queue` → `load_mesh_handle()` → `asset_server.load(GltfAssetLabel::Primitive {...}.from_asset(path))`
- `track_asset_loading` → `AssetEvent<Mesh>` for completion notification
- `asset_server.load_state()` for failure checking
- `meshes.get(&handle)` to read loaded mesh data

## Proposed Design: Direct Mesh Loader

### Overview

Replace the `AssetServer` → `AssetLoader` → `AssetEvent` path with direct IO + decode + insert into `Assets<Mesh>`.

```
drain_load_queue  → spawn IO task (read bytes + decode)
                            ↓
poll_completed    → mesh_io::decode() → MeshData → assets.add(mesh) → Handle<Mesh>
```

We **keep** Bevy's `Assets<Mesh>` for storage and `Handle<Mesh>` for referencing — it's just a typed slotmap. We skip the `AssetServer` orchestration.

### Architecture

```
lib/mesh_io/               ← Pure decoding library. No Bevy, no ECS.
  mod.rs                   ← MeshData struct, format detection, Into<bevy::Mesh>
  glb.rs                   ← Standard GLB → MeshData (via gltf crate)
  draco_glb.rs             ← Draco GLB → MeshData (via draco_decoder)
  stl.rs                   ← STL → MeshData (extracted from current stl_loader.rs)
  (future) svmesh.rs       ← mmap → MeshData (zero-copy)

lib/sequence/loader.rs     ← Modified: spawn IO tasks, poll results, insert into Assets<Mesh>
```

### MeshData: The Bridge Type

```rust
/// Decoded mesh geometry — format-agnostic, ready for GPU upload.
/// Lives in lib/mesh_io, no Bevy dependency required.
pub struct MeshData {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub indices: Vec<u32>,
}

/// With a `bevy` feature flag:
impl From<MeshData> for bevy::mesh::Mesh {
    fn from(data: MeshData) -> Self {
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::default())
            .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, data.positions)
            .with_inserted_attribute(Mesh::ATTRIBUTE_NORMAL, data.normals)
            .with_inserted_indices(Indices::U32(data.indices))
    }
}
```

Both seaview and seaview-pipeline can depend on `mesh_io` without pulling in Bevy.

### Format Detection

```rust
pub enum MeshFormat {
    Glb,            // Standard GLB — gltf crate
    DracoGlb,       // GLB with KHR_draco_mesh_compression
    Gltf,           // Standard glTF (text)
    Stl,            // Binary or ASCII STL
    // (future)
    SvMesh,         // Zero-copy mmap format
}

/// Detect format from extension + header sniffing.
/// Draco vs standard GLB requires reading the JSON chunk to check
/// for extensionsRequired/extensionsUsed containing KHR_draco_mesh_compression.
pub fn detect_format(path: &Path) -> io::Result<MeshFormat> { ... }

/// Unified decode entry point.
pub fn decode(bytes: &[u8], format: MeshFormat) -> Result<MeshData, MeshIoError> {
    match format {
        MeshFormat::Glb => glb::decode(bytes),
        MeshFormat::DracoGlb => draco_glb::decode(bytes),
        MeshFormat::Stl => stl::decode(bytes),
        _ => todo!(),
    }
}
```

### Draco GLB Decoding

The `KHR_draco_mesh_compression` extension on a glTF primitive looks like:

```json
{
  "primitives": [{
    "extensions": {
      "KHR_draco_mesh_compression": {
        "bufferView": 5,
        "attributes": {
          "POSITION": 0,
          "NORMAL": 1
        }
      }
    }
  }]
}
```

The `bufferView` points to compressed bytes in the GLB binary chunk. The `attributes` map semantic names to Draco attribute IDs.

**Decode flow:**

1. Parse GLB with `gltf` crate (JSON structure only)
2. Find first mesh, first primitive
3. Read `KHR_draco_mesh_compression` extension from primitive JSON
4. Extract buffer view bytes from GLB binary chunk
5. Feed to `draco_decoder::decode_mesh_with_config_sync()`
6. Map decoded attributes (by Draco ID) back to POSITION, NORMAL, etc.
7. Build `MeshData`

**Decoder crate:** `draco_decoder` (v0.0.25) — wraps Google's C++ Draco library via `cxx`. Provides sync decode API:

```rust
use draco_decoder::{decode_mesh_with_config_sync, MeshDecodeResult, AttributeValues};

let result: MeshDecodeResult = decode_mesh_with_config_sync(&compressed_bytes)
    .ok_or(MeshIoError::DracoDecode)?;

// result.config.vertex_count(), index_count(), attributes()
// result.data — raw decoded buffer
// Each MeshAttribute has: dim(), data_type(), offset(), lenght()
```

### Modified Loader Pipeline

**`drain_load_queue` changes:**

Instead of `asset_server.load()`, spawn a task on Bevy's `AsyncComputeTaskPool`:

```rust
fn drain_load_queue(
    mut load_queue: ResMut<LoadQueue>,
    mut pending_decodes: ResMut<PendingDecodes>,
    // No asset_server needed
) {
    let pool = AsyncComputeTaskPool::get();

    while let Some(idx) = load_queue.pop() {
        let path = frame_paths[idx].clone();
        let task = pool.spawn(async move {
            let bytes = std::fs::read(&path)?;
            let format = mesh_io::detect_format_from_bytes(&bytes);
            let mesh_data = mesh_io::decode(&bytes, format)?;
            Ok::<(usize, MeshData), MeshIoError>((idx, mesh_data))
        });
        pending_decodes.push(task);
    }
}
```

**New `poll_completed_decodes` system:**

```rust
fn poll_completed_decodes(
    mut pending: ResMut<PendingDecodes>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut sequence_assets: ResMut<SequenceAssets>,
) {
    for (idx, mesh_data) in pending.drain_completed() {
        let mesh: Mesh = mesh_data.into();
        let handle = meshes.add(mesh);
        sequence_assets.frame_handles[idx] = Some(handle);
        sequence_assets.loaded_frames[idx] = true;
        sequence_assets.loaded_count += 1;
    }
}
```

### What We Keep From Bevy

- `Assets<Mesh>` — storage and `Handle<Mesh>` references (just a typed slotmap)
- Entity spawning — `Mesh3d(handle)`, `MeshMaterial3d`, `Transform`, etc.
- `manage_frame_window` — unchanged, still spawns/despawns entities
- Timeline, sections, budget — unchanged

### What We Remove / Bypass

- `AssetServer::load()` for sequence meshes (keep for one-off scene loads if needed)
- `AssetEvent<Mesh>` listener in `track_asset_loading` → replaced by `poll_completed_decodes`
- `GltfAssetLabel::Primitive` extraction
- `asset_id_to_frame` reverse index (no longer needed, we control insertion)
- `bevy_gltf` dependency for sequence loading (may keep for other purposes)

## Draco Decoder Crates Evaluated

| Crate | Type | API | Status | Notes |
|-------|------|-----|--------|-------|
| `draco_decoder` 0.0.25 | C++ bindings (cxx) | sync + async | WIP (25 releases) | 97% documented, clean API |
| `draco-oxide` 0.1.0-alpha.4 | Pure Rust rewrite | encode + decode | Alpha | Heavy deps (faer, image, tobj), 25% documented |
| `draco-rs` 0.1.3 | C++ bindings | compress + decompress | Stable-ish | Needs cmake + C++ draco lib |

**Recommendation:** `draco_decoder` — lightweight C++ bindings, sync API ready for task pool, no cmake needed (uses cxx-build).

## File Size Context

Source: fluid simulation output (likely marching cubes)

| Format | Per Frame | Notes |
|--------|-----------|-------|
| Raw sim output | ~60 MB | Processed by seaview-pipeline |
| Uncompressed GLB | ~20 MB | Current working format |
| Draco GLB | ~3 MB | Bevy can't load (no decoder) |
| svmesh (projected) | ~2-4 MB | Zero-copy mmap, future format |

## Implementation Plan

### Phase 1: mesh_io + Draco support
1. Create `lib/mesh_io/` module with `MeshData`, format detection
2. Port STL decoding from `stl_loader.rs` into `mesh_io/stl.rs`
3. Write `mesh_io/glb.rs` — standard GLB decode using `gltf` crate
4. Write `mesh_io/draco_glb.rs` — Draco GLB decode using `draco_decoder`
5. Add `draco_decoder` to workspace dependencies

### Phase 2: Direct loader integration
6. Add `PendingDecodes` resource (vec of spawned tasks)
7. Modify `drain_load_queue` to use `AsyncComputeTaskPool` + `mesh_io::decode`
8. Add `poll_completed_decodes` system
9. Simplify `track_asset_loading` (or remove — completion tracked in poll system)
10. Update `handle_load_requests` for new format detection

### Phase 3: svmesh format (seaview-joh)
11. Design `.svmesh` binary layout (header + page-aligned vertex/index data)
12. Write `mesh_io/svmesh.rs` with `memmap2` for zero-copy
13. Add prebake CLI tool in `bin/` (reads any format → writes `.svmesh`)
14. First-load auto-conversion + cache

## Shared Library Potential

`mesh_io` is deliberately Bevy-free at its core. The `Into<bevy::Mesh>` impl lives behind a feature flag. This means:

- **seaview** uses it with `bevy` feature for viewing
- **seaview-pipeline** uses it without Bevy for processing/conversion
- Could be published as a standalone crate for the Bevy ecosystem (there's nothing like this today)