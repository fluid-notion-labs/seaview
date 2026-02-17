# Mesh Loading Ecosystem — Research & Strategy

> **Date**: 2026-02-16
> **Status**: Active research
> **Related issues**: seaview-bdz.4 (MeshSource trait), seaview-joh (zero-copy mmap)

## Table of Contents

1. [Bevy's Asset System — What It Solves](#bevys-asset-system--what-it-solves)
2. [Bevy Ecosystem — Who Loads What](#bevy-ecosystem--who-loads-what)
3. [What's Missing From The Ecosystem](#whats-missing-from-the-ecosystem)
4. [Bevy Roadmap / RFCs](#bevy-roadmap--rfcs)
5. [Quads, N-gons, and GPU Reality](#quads-n-gons-and-gpu-reality)
6. [Our Needs vs Bevy's Design](#our-needs-vs-bevys-design)
7. [Loader-Agnostic Architecture](#loader-agnostic-architecture)
8. [Capability Querying](#capability-querying)
9. [TriMesh Normalization](#trimesh-normalization)
10. [Recommendations](#recommendations)

---

## Bevy's Asset System — What It Solves

Bevy's asset system (`bevy_asset`) is a well-designed async pipeline:

- **`AssetLoader` trait** — the extension point. Each format implements `async fn load()` returning a typed asset. The asset server dispatches based on file extension.
- **`AssetServer`** — handles I/O, caching, ref-counting, hot-reload, dependency tracking. Loads happen off the main thread.
- **`Assets<T>` resource** — typed ECS storage. Strong/weak handles, automatic cleanup on drop.
- **`AssetEvent<T>`** — change detection. `LoadedWithDependencies`, `Modified`, `Removed`.
- **`AssetReader` trait** — pluggable I/O backends. Filesystem, HTTP, embedded, custom (we use this for `seq://` virtual paths).
- **`RenderAssetUsages`** — controls whether data lives in `MAIN_WORLD` (CPU), `RENDER_WORLD` (GPU), or both. This is key for our memory management.
- **Sub-assets** — glTF uses this: loading a `.glb` produces a top-level `Gltf` asset, which references `GltfMesh`, `GltfNode`, `Mesh`, `Image`, `Material` sub-assets.
- **Settings** — loaders can accept per-load settings (e.g. `GltfLoaderSettings`, our `StlLoaderSettings`).

### What it does well

- Async parallel loading with backpressure
- Handle-based reference counting / lifetime management
- Extension-based format dispatch (no manual registry)
- Hot reload for development
- Clean separation between I/O (`AssetReader`) and parsing (`AssetLoader`)

### What it doesn't do

- No format capability metadata ("does this format support UVs?")
- No topology awareness (the `Mesh` is already built by the time you see it)
- No multi-format abstraction (each loader is standalone)
- No concept of "mesh sequence" — that's entirely our layer
- No memory budget integration — handles are ref-counted but there's no global cap
- No streaming / partial loading (you get the whole asset or nothing)

---

## Bevy Ecosystem — Who Loads What

### Built-in

| Format | Crate | Notes |
|--------|-------|-------|
| glTF 2.0 / GLB | `bevy_gltf` | Two-phase: `Gltf` → sub-assets. Multi-mesh, materials, animations, morph targets. The "blessed" format. |

### Community plugins (crates.io, sorted by downloads)

| Crate | Format | Downloads | Latest Bevy | Status |
|-------|--------|-----------|-------------|--------|
| `bevy_stl` | STL (binary/ASCII) | 120k | 0.18 | ✅ Active, tracks releases |
| `bevy_obj` | OBJ (Wavefront, via tobj) | 65k | 0.18 | ✅ Active, tracks releases |
| `bevy_gaussian_splatting` | PLY (gaussian splats only) | 37k | ~0.15 | Specialized rendering, not mesh PLY |
| `bevy_vox_scene` | MagicaVoxel .vox → scene | 26k | 0.18 | ✅ Active |
| `bevy_vox_mesh` | MagicaVoxel .vox → mesh | 14k | 0.8 (Bevy 0.12) | ⚠️ Stale |
| `bevy_fsc_point_cloud` | Point clouds | 15k | 0.4 (Bevy 0.12) | ❌ Abandoned |
| `bevy_mod_fbx` | FBX | 4k | 0.4 (Bevy 0.10) | ❌ Abandoned |
| `bevy_ufbx` | FBX (via ufbx C lib) | 658 | 0.17 | ⚠️ New, active |
| `bevy_dae` | Collada DAE | 696 | 0.1 | ⚠️ Brand new |
| `bevy_chair` | Custom binary mesh | 9k | 0.13 | ⚠️ Stale |

### Underlying Rust crates (not Bevy-specific)

| Crate | Format | Notes |
|-------|--------|-------|
| `gltf` 1.4 | glTF 2.0 | Used by `bevy_gltf`. Mature. |
| `tobj` 4.0 | OBJ/MTL | Used by `bevy_obj` and our `ObjLoader`. Lightweight, triangulation built-in. |
| `stl_io` 0.7 | STL | Used by `bevy_stl` and our `StlLoader`. |
| `ply-rs` | PLY | Pure Rust PLY parser. Handles binary/ASCII, arbitrary properties. |
| `vdb-rs` 0.6 | OpenVDB | 41k downloads. No Bevy integration. |
| `meshopt` 0.3 | Mesh optimization | Vertex cache, overdraw, fetch optimization. We already depend on this. |

---

## What's Missing From The Ecosystem

**Nobody has built:**

- **PLY mesh loader** — PLY-as-indexed-triangles is extremely common in scientific viz, 3D scanning, point cloud research. `bevy_gaussian_splatting` only handles the gaussian splat flavor.
- **VTK/VTU** — the lingua franca of CFD, FEA, and scientific post-processing. Zero Bevy support. The `vtkio` Rust crate exists but has no Bevy integration.
- **Point cloud rendering** — PCD (Point Cloud Library), LAS/LAZ (LiDAR), E57. `bevy_fsc_point_cloud` is dead. These need `PointList` topology + custom shaders.
- **Volumetric / tetrahedral mesh** — No format support, no rendering pipeline.
- **Format capability negotiation** — Nobody asks "can this loader produce UVs?" or "is this triangulated?"
- **Unified multi-format loading abstraction** — Every plugin is standalone. No common trait.
- **Sequence-aware loading** — Bevy loads individual assets. Temporal coherence, frame budgets, prefetching — all custom.

---

## Bevy Roadmap / RFCs

There are **no open RFCs or issues** for:

- Multi-format mesh loading abstraction
- Mesh capability metadata
- A format registry / negotiation pattern
- Topology-aware loading (quads, n-gons, mixed)

The active tracking issues are about the **asset system infrastructure**:

- **#9714** — Asset V2 tracking (the current system, already shipped)
- **#11216** — Asset saving and improvements (0.19 milestone)
- **#21878** — Sub-asset lifecycle (don't unload root until all sub-assets dropped)
- **#21641** — Random-access assets (ZIP files)

The Bevy team's position on formats: **"Convert to glTF."** This is pragmatic for games but doesn't serve scientific visualization where solver output comes in whatever format the solver writes.

---

## Quads, N-gons, and GPU Reality

### How GPUs actually render

Modern GPUs only understand three primitive topologies for rasterization:

- **Triangles** (and triangle strips/fans)
- **Lines** (and line strips)
- **Points**

**Quads are old hat.** OpenGL 1.x had `GL_QUADS` but it was just syntactic sugar — the driver split them into triangles. Vulkan, Metal, WebGPU, and modern OpenGL (core profile) don't even expose quad primitives. wgpu (which Bevy uses) only supports:

```
PointList, LineList, LineStrip, TriangleList, TriangleStrip
```

### What formats store vs what gets rendered

| Format | Native geometry | What happens at load time |
|--------|----------------|--------------------------|
| STL | Triangles only | Direct: already triangulated |
| OBJ | Arbitrary n-gons | `tobj` triangulates (fan triangulation) during parse |
| glTF | Triangles only (spec requirement) | Direct: already triangulated |
| PLY | Arbitrary polygons | Must triangulate on load |
| VTK | Mixed (tri, quad, tet, hex, polygon) | Must triangulate surface on load |
| FBX | Arbitrary polygons | Must triangulate on load |
| USD | Arbitrary polygons | Must triangulate on load |

### Triangulation quality

Fan triangulation (what `tobj` does) is fast but produces poor triangles for non-convex polygons. For CFD meshes this is rarely a problem because:

1. Most CFD output is already triangulated (marching cubes, isosurface extraction)
2. Quad-dominant meshes (structured grids) are convex, so fan triangulation is correct
3. The visual difference is negligible for smooth surfaces

For cases where it matters, ear-clipping or constrained Delaunay triangulation is better but slower. The `earcutr` crate handles this in Rust.

### Bottom line

**Normalizing everything to TriangleList is correct and performant.** There's no GPU advantage to keeping quads — they'd just be split by the driver anyway. The only consideration is triangulation quality for non-convex polygons, which is a niche concern for our use case.

---

## Our Needs vs Bevy's Design

### Where we align

| Aspect | Bevy provides | We use it |
|--------|--------------|-----------|
| Async loading | `AssetLoader` + `AssetServer` | ✅ Both STL and OBJ loaders are `AssetLoader` impls |
| Extension dispatch | Automatic by file extension | ✅ `.stl` → `StlLoader`, `.obj` → `ObjLoader`, `.glb` → `GltfLoader` |
| Handle lifecycle | `Handle<Mesh>` ref-counting | ✅ `FrameStore` holds handles, drop = unload |
| GPU residency control | `RenderAssetUsages` flags | ✅ `promote_to_gpu` / `demote_from_gpu` |
| Change detection | `AssetEvent<Mesh>` | ✅ `track_asset_loading` watches for `LoadedWithDependencies` |
| Custom I/O | `AssetReader` trait | ✅ `seq://` virtual filesystem for sequence directories |

### Where we diverge

| Our need | Bevy's answer | Our solution |
|----------|--------------|-------------|
| Memory budget | None (load everything) | `MemoryBudget` resource, `LoadQueue` throttling, section-based loading |
| Sequence awareness | None (individual assets) | `FrameStore`, `Timeline`, `LoadSection`, frame window management |
| Format detection | Extension only, no capability query | `MeshFileFormat` enum, `is_gltf()`, `is_direct_mesh()` |
| Loading strategy branching | Each loader is independent | `drain_load_queue` branches on format (direct mesh vs two-phase glTF) |
| Spatial acceleration | None | `SpatialMesh` / `SpatialIndex` (parry3d) built post-load |
| Coordinate transforms | glTF has a known coord system; others don't | `SourceOrientation` + cached `coord_transform` in `LoaderConfig` |
| Multi-mesh per frame | glTF handles this; others don't | `Vec<Handle<Mesh>>` per frame, `extract_gltf_meshes` for GLB |
| Unload + memory reclaim | Drop handle → eventual cleanup | Immediate `Assets::remove()` + `malloc_trim()` for RSS control |

### The gap: no loader-agnostic abstraction

Currently our `drain_load_queue` does this:

```rust
match format {
    MeshFileFormat::Stl | MeshFileFormat::Obj => {
        let handle = load_direct_mesh_handle(&asset_server, &filename);
        asset_index.asset_id_to_frame.insert(handle.id(), idx);
        frame_store.frame_handles[idx] = Some(vec![handle]);
    }
    MeshFileFormat::Gltf | MeshFileFormat::Glb => {
        let gltf_handle = load_gltf_handle(&asset_server, &filename);
        asset_index.gltf_id_to_frame.insert(gltf_handle.id(), idx);
        asset_index.gltf_handles[idx] = Some(gltf_handle);
    }
}
```

Every new format adds another match arm, another tracking path, another set of event handlers. This doesn't scale.

---

## Loader-Agnostic Architecture

### The MeshSource abstraction (seaview-bdz.4)

The trait sits **above** the asset system, describing *loading strategy*, not replacing `AssetLoader`:

```rust
/// Describes how a mesh format is loaded through Bevy's asset system.
pub trait MeshSource: Send + Sync + 'static {
    /// Format identifier for logging / UI.
    fn label(&self) -> &'static str;

    /// Does this format produce `Handle<Mesh>` directly, or go through
    /// an intermediate asset (like glTF's two-phase loading)?
    fn loading_strategy(&self) -> LoadingStrategy;

    /// Kick the load for a single frame. Returns a tracking token.
    fn kick_load(
        &self,
        asset_server: &AssetServer,
        filename: &str,
    ) -> LoadToken;

    /// Check if loading is complete and extract mesh handles.
    /// Called each frame until it returns `Some`.
    fn try_extract_meshes(
        &self,
        token: &LoadToken,
        // ... asset access
    ) -> Option<Vec<Handle<Mesh>>>;

    /// Capabilities of meshes produced by this format.
    fn capabilities(&self) -> MeshCapabilities;
}

pub enum LoadingStrategy {
    /// Produces `Handle<Mesh>` directly (STL, OBJ, PLY, future mmap).
    DirectMesh,
    /// Produces an intermediate asset that must be further extracted (glTF/GLB).
    TwoPhase,
}
```

### Implementations

| Format | Strategy | Notes |
|--------|----------|-------|
| `StlSource` | `DirectMesh` | `asset_server.load()` → `Handle<Mesh>` |
| `ObjSource` | `DirectMesh` | `asset_server.load()` → `Handle<Mesh>` |
| `GltfSource` | `TwoPhase` | `asset_server.load()` → `Handle<Gltf>` → extract `Handle<Mesh>` per primitive |
| `PlySource` (future) | `DirectMesh` | Custom `AssetLoader` using `ply-rs` |
| `SvmeshSource` (future) | `DirectMesh` | mmap zero-copy (seaview-joh) |

### What this eliminates

- The `match format` in `drain_load_queue` becomes `source.kick_load()`
- The separate `extract_gltf_meshes` system merges into `try_extract_meshes`
- The `AssetIndex` reverse maps become internal to each `MeshSource` impl
- Adding a new format = implement the trait, register it. No loader system changes.

---

## Capability Querying

### What we want to know about a loaded mesh

```rust
pub struct MeshCapabilities {
    /// Topology: triangles, lines, points, mixed.
    pub topology: TopologyKind,
    /// Whether the format provides normals (vs us computing them).
    pub has_normals: bool,
    /// Whether the format provides UV coordinates.
    pub has_uvs: bool,
    /// Whether the format provides vertex colors.
    pub has_vertex_colors: bool,
    /// Whether the format supports multiple objects per file.
    pub multi_object: bool,
    /// Whether the format supports materials (that we could use).
    pub has_materials: bool,
    /// Estimated relative parse cost (for loading priority / scheduling).
    pub parse_cost: ParseCost,
}

pub enum TopologyKind {
    TrianglesOnly,     // STL, glTF
    MayContainQuads,   // OBJ, PLY, VTK — triangulated at load time
    PointCloud,        // PCD, LAS
    Mixed,             // VTK unstructured
}

pub enum ParseCost {
    Cheap,     // STL binary, mmap
    Moderate,  // OBJ, GLB
    Expensive, // ASCII STL, large glTF scenes
}
```

### Per-format capability table

| Format | Topology | Normals | UVs | Vertex Color | Multi-Object | Materials |
|--------|----------|---------|-----|-------------|-------------|-----------|
| STL | TrianglesOnly | ✅ (face normals) | ❌ | ❌ | ❌ | ❌ |
| OBJ | MayContainQuads | ✅ (optional) | ✅ (optional) | ❌ | ✅ (groups) | ✅ (MTL, ignored) |
| GLB/glTF | TrianglesOnly | ✅ | ✅ | ✅ | ✅ (primitives) | ✅ (PBR) |
| PLY | MayContainQuads | ✅ (optional) | ✅ (optional) | ✅ (optional) | ❌ | ❌ |
| VTK | Mixed | ✅ (cell/point data) | ❌ | ✅ (scalar fields) | ❌ | ❌ |

### How this helps

- **UI**: Show relevant controls. No UV editor controls when format has no UVs.
- **Material system**: Know whether to use vertex colors, texture mapping, or flat shading.
- **Spatial queries**: Know if normals need computing before building BVH.
- **Loading scheduler**: Prioritize cheap formats; batch expensive ones.

---

## TriMesh Normalization

### The question

Should every format normalize to an identical internal representation before entering the loader pipeline?

### The answer: yes, and it's already happening

Every format already produces `Mesh` with `PrimitiveTopology::TriangleList`. The normalization is:

1. **Positions**: `ATTRIBUTE_POSITION` as `Float32x3`. Always present.
2. **Indices**: `Indices::U32`. Always present (we index everything).
3. **Normals**: `ATTRIBUTE_NORMAL` as `Float32x3`. Computed if not provided by format.
4. **UVs**: `ATTRIBUTE_UV_0` as `Float32x2`. Optional, only if format provides them.

This happens inside each `AssetLoader::load()`:
- `StlLoader`: builds indexed mesh, accumulates face normals per vertex.
- `ObjLoader`: `tobj` triangulates quads/n-gons, `single_index: true` merges attribute indices, Bevy computes normals if OBJ lacks them.
- `GltfLoader` (Bevy built-in): glTF is already triangulated per spec.

### Performance of normalization

| Step | Cost | When it matters |
|------|------|----------------|
| Triangulation (fan) | O(n) per face | Only for quads/n-gons. Most CFD output is already tri. |
| Index merging | O(n log n) or hash-based | OBJ has separate pos/normal/uv indices → must merge to single-index. |
| Normal computation | O(triangles) | STL: accumulate face normals. OBJ without normals: smooth normals via Bevy. |
| Vertex dedup | O(n) hash | `tobj` does this with `single_index: true`. |

All of this is **dwarfed by I/O time** for meshes in the 100k–10M vertex range. A 5M vertex STL is ~240MB of disk I/O; triangulation and normal computation on the parsed data takes <50ms. Not a bottleneck.

### Where it could become a bottleneck

- **mmap/zero-copy format (seaview-joh)**: If we design a custom binary format where the on-disk layout matches `Mesh` attribute layout exactly, we can skip all normalization. The `SvmeshSource` would `mmap` the file and hand Bevy a `Mesh` backed by the mapped memory. This is the endgame for sequence playback performance.

---

## Recommendations

### Immediate (this session)

- [x] OBJ loader landed with `tobj` — follows same pattern as STL.
- [x] `MeshFileFormat::Obj` variant, discovery patterns, binary tool support.
- [x] `load_direct_mesh_handle` unifies STL/OBJ loading path.
- [x] `is_direct_mesh()` helper distinguishes direct-load from two-phase.

### Short-term (seaview-bdz.4)

1. **Design the `MeshSource` trait** as outlined above. Keep it minimal — don't over-abstract.
2. **Implement for existing formats**: `StlSource`, `ObjSource`, `GltfSource`.
3. **Refactor `drain_load_queue`** to use `source.kick_load()` instead of match arms.
4. **Add `MeshCapabilities`** — even if only used for logging initially.

### Medium-term

1. **PLY loader** — use `ply-rs`, produce `Handle<Mesh>`. Triangulate polygon faces. Support vertex colors (map to `ATTRIBUTE_COLOR`). This would make us the first Bevy project with proper PLY mesh loading.
2. **Capability-driven UI** — hide UV/material controls when format doesn't support them.
3. **VTK/VTU loader** — huge for CFD users. `vtkio` crate exists. Surface extraction from unstructured grids is the hard part.

### Long-term (seaview-joh)

1. **Custom `.svmesh` format** — memory-mappable, zero-normalization, pre-indexed, pre-triangulated. The `SvmeshSource` implementation of `MeshSource`.
2. **Temporal compression** — delta encoding between frames, PCA on vertex displacements.
3. **GPU-direct loading** — bypass CPU mesh entirely, DMA from mmap to GPU buffer.

---

## References

- Bevy `AssetLoader` trait: `bevy_asset/src/loader.rs`
- Bevy `Mesh` struct: `bevy_mesh/src/mesh.rs` — `PrimitiveTopology`, vertex attributes, `compute_normals()`
- Bevy Asset V2 tracking: [github.com/bevyengine/bevy#9714](https://github.com/bevyengine/bevy/issues/9714)
- wgpu `PrimitiveTopology`: PointList, LineList, LineStrip, TriangleList, TriangleStrip (no quads)
- Our loader architecture: `lib/sequence/loader/` — mod.rs, assets.rs, load_queue.rs, loading_tracker.rs
- Our asset loaders: `lib/asset_loaders/` — stl_loader.rs, obj_loader.rs
- Existing format comparison: `docs/research/mesh_formats_comparison.md`
- Spatial queries research: `research/spatial_queries.md`
