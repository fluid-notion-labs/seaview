# Meshlet File Generation & Seaview Loading

## Overview

manifold bakes FluidX3D STL frame meshes into Bevy's `.meshlet` binary format.
seaview loads these at runtime using `MeshletMesh3d` — Bevy's Nanite-style
virtual geometry renderer.

---

## 1. Generating `.meshlet` Files

### Build

```sh
cargo build --features meshlet --release
```

### Pipeline config

Copy `assets/pipelines/fluidx3d-meshlet.toml` into your run directory as
`manifold.toml`, then run:

```sh
./target/release/manifold --run-dir runs/my-sim
```

**Stage order** (all required before `meshlet_bake`):

```
raw/*.stl
  → weld_vertices       (deduplicate triangle boundary verts)
  → generate_indices    (MeshletMesh::from_mesh requires indexed mesh)
  → recalculate_normals (requires NORMAL attribute, smooth=true recommended)
  → [persist glb]       (optional intermediate for inspection)
  → meshlet_bake        → meshlets/*.meshlet
```

### Output layout

```
runs/my-sim/
  raw/                  ← FluidX3D STL output
  normals/              ← intermediate GLBs (optional)
  meshlets/
    surface_0000.meshlet
    surface_0001.meshlet
    ...
```

### Inspect output

```sh
./target/release/sv_meshlet_info meshlets/surface_0042.meshlet
./target/release/sv_meshlet_info meshlets/surface_0042.meshlet --verbose
```

### Standalone bake (single file)

```sh
./target/release/sv_stage_meshlet_bake normals/surface_0042.glb
# or with explicit output:
./target/release/sv_stage_meshlet_bake normals/surface_0042.glb \
    -o meshlets/surface_0042.meshlet \
    --quantization-factor 4
```

---

## 2. Binary Format

Files are byte-for-byte compatible with Bevy's `MeshletMeshSaver` /
`MeshletMeshLoader`. **Do not hand-edit.**

```
[header — uncompressed]
  u64  magic    = 1717551717668
  u64  version  = 3              ← MESHLET_MESH_ASSET_VERSION
  MeshletAabb   (24 bytes)       ← center: Vec3, half_extent: Vec3
  u32  bvh_depth

[body — lz4 frame compressed]
  u64 len + [u32]              vertex_positions  (bitpacked, quantised)
  u64 len + [u32]              vertex_normals    (octahedral encoded, 2×i16snorm)
  u64 len + [Vec2]             vertex_uvs        (currently zeroed — see research/uv-generation.md)
  u64 len + [u8]               indices           (meshlet-local, max 256 verts/meshlet)
  u64 len + [BvhNode]          bvh               (BVH8, root at index 0)
  u64 len + [Meshlet]          meshlets
  u64 len + [MeshletCullData]  meshlet_cull_data
```

Asset version bumps each Bevy minor release — **re-bake all frames after
upgrading Bevy**.

---

## 3. Seaview Loading — Breadcrumbs

### Plugin setup

```rust
use bevy::pbr::experimental::meshlet::MeshletPlugin;

app.add_plugins(MeshletPlugin {
    // Pre-allocated GPU cluster slots. 4 bytes each. Must not exceed 2^25.
    // Start with 1<<22 (~4M slots, 16MB VRAM) and tune up if meshes flicker.
    cluster_buffer_slots: 1 << 22,
});
```

**GPU requirements** (check on startup):
```rust
MeshletPlugin::required_wgpu_features()
// TEXTURE_INT64_ATOMIC | TEXTURE_ATOMIC | SHADER_INT64 | SUBGROUP
// | DEPTH_CLIP_CONTROL | IMMEDIATES
// Vulkan + Metal only. No WebGPU, no DX12.
```

**Camera**: must have `Msaa::Off`.

```rust
commands.spawn((
    Camera3d::default(),
    Msaa::Off,
));
```

### Asset loading

Register the loader (done automatically by `MeshletPlugin`). Load via
`AssetServer` as usual:

```rust
let handle: Handle<MeshletMesh> = asset_server.load("meshlets/surface_0042.meshlet");
```

The `.meshlet` extension is registered by `MeshletMeshLoader`. Bevy's asset
server handles async IO and lz4 decompression.

### Spawning a frame entity

```rust
use bevy::pbr::experimental::meshlet::MeshletMesh3d;

commands.spawn((
    MeshletMesh3d(handle),
    MeshMaterial3d(material_handle),  // must be opaque, no alpha
    Transform::default(),
    // Visibility, PreviousGlobalTransform auto-required
));
```

### Frame playback pattern (seaview)

For animated sequences, swap the `MeshletMesh3d` handle each frame tick:

```rust
fn advance_frame(
    mut query: Query<&mut MeshletMesh3d, With<FluidSurface>>,
    frame_handles: Res<FrameHandles>,   // Vec<Handle<MeshletMesh>>
    mut frame_index: ResMut<FrameIndex>,
) {
    frame_index.0 = (frame_index.0 + 1) % frame_handles.0.len();
    for mut mesh3d in &mut query {
        mesh3d.0 = frame_handles.0[frame_index.0].clone();
    }
}
```

Pre-load all handles at startup so assets are resident before playback:

```rust
fn load_frames(
    asset_server: Res<AssetServer>,
    mut handles: ResMut<FrameHandles>,
) {
    handles.0 = (0..FRAME_COUNT)
        .map(|i| asset_server.load(format!("meshlets/surface_{:04}.meshlet", i)))
        .collect();
}
```

### Material constraints

Meshlet materials must be opaque and implement
`Material::meshlet_mesh_fragment_shader`:

```rust
impl Material for FluidSurfaceMaterial {
    fn meshlet_mesh_fragment_shader() -> ShaderRef {
        "shaders/fluid_surface.wgsl".into()
    }
    // opaque_render_method must NOT be overridden
}
```

Fragment shader constraints:
- No `dpdx` / `dpdy` / `fwidth` — use manual chain-rule arithmetic
- No vertex shader control (position/normal are decoded by meshlet renderer)
- Tangents derived at runtime — no stored tangents
- UVs currently zeroed → use world position for triplanar shading (see
  `research/uv-generation.md` for future UV baking plan)

### Useful references

- `bevy/crates/bevy_pbr/src/meshlet/mod.rs` — `MeshletPlugin`, `MeshletMesh3d`
- `bevy/crates/bevy_pbr/src/meshlet/asset.rs` — `MeshletMesh`, `MeshletMeshLoader`
- `bevy/examples/3d/meshlet.rs` — minimal working example
- `research/virtual-geometry-bevy.md` — full frame pipeline breakdown
- `meshlet_external/` — vendored source snapshot (bevy main @ 9b279b3)

---

## 4. Known Limitations

| Limitation | Detail |
|---|---|
| Opaque only | No alpha, no transmission, no transparency |
| No UV baking | UV_0 = `[0,0]` — use triplanar in shader |
| Vulkan/Metal only | No WebGPU, no DX12 |
| No MSAA | Camera must use `Msaa::Off` |
| Asset versioning | Re-bake on Bevy upgrade |
| Slow bake | Seconds/frame for dense meshes — offline only |
| BVH tolerance | `verify_bvh` tolerance widened to `0.001` vs upstream `0.0001` |
