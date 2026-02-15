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
  svmesh.rs                ← mmap → MeshData (zero-copy)

lib/sequence/loader.rs     ← Modified: spawn IO tasks, poll results, insert into Assets<Mesh>


┌─────────────────────────────────────────────────────────────────────┐
│                     GLB / Draco GLB / STL                          │
│                                                                     │
│  disk ──read()──► bytes ──decode()──► MeshData ──into()──► Mesh    │
│                                                                     │
│  ┌──────────┐   ┌────────────────┐   ┌──────────┐   ┌───────────┐ │
│  │ fs::read  │──►│ mesh_io::glb   │──►│ MeshData │──►│ Bevy Mesh │ │
│  │ (~20 MB)  │   │ mesh_io::draco │   │ (vecs)   │   │ MAIN_WORLD│ │
│  │           │   │ mesh_io::stl   │   │          │   │ (CPU only)│ │
│  └──────────┘   └────────────────┘   └──────────┘   └───────────┘ │
│                                                                     │
│  Cost: full read + full decode + full copy into Mesh                │
│  Latency: ~5-15 ms/frame (IO-bound)                                │
└─────────────────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────────────────┐
│                          svmesh (zero-copy)                         │
│                                                                     │
│  disk ──mmap()──► &[u8] ──view()──► MeshData<'mmap> ──into()──► Mesh
│                                                                     │
│  ┌──────────┐   ┌────────────────┐   ┌──────────┐   ┌───────────┐ │
│  │ memmap2  │──►│ mesh_io::svmesh│──►│ MeshData │──►│ Bevy Mesh │ │
│  │ (no read)│   │ (cast header,  │   │ (slices  │   │ MAIN_WORLD│ │
│  │          │   │  validate,     │   │  into    │   │ (CPU only)│ │
│  │          │   │  slice ptrs)   │   │  vecs)   │   │           │ │
│  └──────────┘   └────────────────┘   └──────────┘   └───────────┘ │
│                                                                     │
│  Cost: no read, no decode, one vec copy (or future zero-copy GPU)   │
│  Latency: ~0.1-1 ms/frame (page-fault on demand)                   │
│                                                                     │
│  File layout (page-aligned):                                        │
│  ┌────────┬───────────────┬──────────────┬──────────────┐          │
│  │ Header │  positions    │   normals    │   indices    │          │
│  │ 64 B   │  [f32;3]×N   │  [f32;3]×N  │  u32×M      │          │
│  │        │  4K-aligned   │  4K-aligned  │  4K-aligned  │          │
│  └────────┴───────────────┴──────────────┴──────────────┘          │
└─────────────────────────────────────────────────────────────────────┘

GPU residency (both paths):
  MAIN_WORLD only ──promote_to_gpu()──► MAIN_WORLD | RENDER_WORLD
       (CPU RAM)                              (CPU RAM + VRAM)
                   ◄──demote_from_gpu()──
  Only frames in the spawn window live on GPU (~5 frames).
  Budget controls CPU-resident frame count (RAM-only).
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
        Mesh::new(PrimitiveTopology::TriangleList, RenderAssetUsages::MAIN_WORLD)
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

## svmesh Compression Research

The svmesh format stores raw mesh geometry for zero-copy mmap. But raw data is
large (~20 MB/frame for our fluid sim meshes). Compression can shrink this
dramatically while keeping the mmap-friendly layout — if we decompress at the
right stage of the pipeline.

### Compression Codecs (CPU decompression)

```
┌──────────┬────────────┬────────────┬───────────┬──────────────────────────┐
│ Codec    │ Decompress │ Compress   │ Ratio on  │ Notes                    │
│          │ throughput │ throughput │ mesh data │                          │
├──────────┼────────────┼────────────┼───────────┼──────────────────────────┤
│ LZ4      │ ~4-5 GB/s  │ ~1-2 GB/s  │ 2-3×      │ Fastest decompress.      │
│          │            │            │           │ Trivially simple.        │
│          │            │            │           │ Rust: lz4_flex crate.    │
├──────────┼────────────┼────────────┼───────────┼──────────────────────────┤
│ Zstd     │ ~1-2 GB/s  │ 0.3-1 GB/s │ 4-8×      │ Best ratio/speed trade.  │
│          │            │            │           │ Dictionary mode great    │
│          │            │            │           │ for similar frames.      │
│          │            │            │           │ Rust: zstd crate.        │
├──────────┼────────────┼────────────┼───────────┼──────────────────────────┤
│ Snappy   │ ~3-4 GB/s  │ ~2-3 GB/s  │ 1.5-2.5×  │ Google. Fast but worst   │
│          │            │            │           │ ratio. Not worth it over │
│          │            │            │           │ LZ4.                     │
├──────────┼────────────┼────────────┼───────────┼──────────────────────────┤
│ Brotli   │ ~0.5 GB/s  │ slow       │ 5-10×     │ Great ratio, too slow    │
│          │            │            │           │ for real-time decompress. │
└──────────┴────────────┴────────────┴───────────┴──────────────────────────┘

Recommendation: Zstd for storage, LZ4 for hot-path.
  - Prebake with zstd (offline, maximize compression).
  - If zstd decompress is too slow at playback, transcode to LZ4 on first load.
  - Both support streaming / block-level decompression.
```

### Timing Budget

At 30 fps → 33 ms per frame. Current frame size ~20 MB raw.

```
┌───────────────────────────┬──────────┬──────────┐
│ Operation                 │ LZ4      │ Zstd     │
├───────────────────────────┼──────────┼──────────┤
│ Decompress 20 MB raw      │ ~4-5 ms  │ ~10-20ms │
│ Decompress 8 MB (quant.)  │ ~2 ms    │ ~4-8 ms  │
│ Decompress 4 MB (delta)   │ ~1 ms    │ ~2-4 ms  │
└───────────────────────────┴──────────┴──────────┘

All within budget, especially with ahead-of-playhead strategy.
```

### Mesh-Specific Techniques (applied before general compression)

These reduce the raw data size, then general compression (LZ4/zstd) is applied
on top. The combination is multiplicative.

**Quantization (lossy, ~50-70% size reduction)**

Fluid sim meshes don't need f32 precision everywhere:
- Positions: quantize to 16-bit fixed-point within bounding box
  - f32×3 = 12 bytes/vert → u16×3 = 6 bytes/vert (2× reduction)
  - Error: < 0.01 mm for a 10m bounding box (plenty for visualization)
- Normals: octahedral encoding, 3×f32 → 2×snorm16
  - f32×3 = 12 bytes/vert → i16×2 = 4 bytes/vert (3× reduction)
  - Decode is a few ALU ops — trivial on GPU or CPU
- Indices: already u32, could use u16 if vertex count < 65536

```
Raw vertex:     12 + 12 = 24 bytes (position + normal)
Quantized:       6 +  4 = 10 bytes
Reduction:       ~2.4×  (before general compression)
```

**Delta / Temporal Encoding (for sequences — potentially huge wins)**

Consecutive fluid sim frames have high spatial coherence — the mesh deforms
smoothly between timesteps. Delta encoding exploits this:

```
┌─────────────────────────────────────────────────────────┐
│ Keyframe (every N frames):  full quantized mesh data    │
│ Delta frame:                diff from previous frame    │
│                                                         │
│  K  D  D  D  D  K  D  D  D  D  K  D  D  D  D  K  ...  │
│  └──── group ────┘  └──── group ────┘                   │
│                                                         │
│ Delta = current_quantized - previous_quantized          │
│ Most deltas are small → extremely compressible          │
│ Typical: 5-20× additional compression on deltas         │
└─────────────────────────────────────────────────────────┘

Trade-off: random access requires decoding from nearest keyframe.
  - Keyframe interval of 10-30 is reasonable (~0.3-1s at 30fps)
  - Scrubbing to arbitrary frame = decode keyframe + apply N deltas
  - Sequential playback = apply 1 delta per frame (cheap)
```

**Vertex Reordering**

Reorder vertices for spatial locality (e.g. Morton/Z-curve ordering).
Adjacent vertices in memory are spatially close → much better compression.
Meshoptimizer can do this. Apply once at prebake time.

### Combined Size Estimates

```
┌─────────────────────────────┬───────────┬──────────────────────────┐
│ Technique stack             │ Per frame │ Notes                    │
├─────────────────────────────┼───────────┼──────────────────────────┤
│ Raw (current GLB)           │ ~20 MB    │ Baseline                 │
│ + Quantization              │ ~8 MB     │ 2.4× from quant alone    │
│ + Vertex reorder + Zstd     │ ~2-3 MB   │ General compression      │
│ + Delta encoding + Zstd     │ ~0.2-1 MB │ Delta frames are tiny    │
│   (keyframes still ~2-3 MB) │           │                          │
├─────────────────────────────┼───────────┼──────────────────────────┤
│ Draco GLB (for reference)   │ ~3 MB     │ Current alternative      │
└─────────────────────────────┴───────────┴──────────────────────────┘

With delta encoding: 2133 frames × ~0.5 MB avg ≈ 1 GB total
vs current:          2133 frames × 20 MB        ≈ 42 GB total
~40× reduction.
```

### GPU Decompression (advanced — research further)

Instead of CPU decompress → upload, upload compressed data directly and
decompress on the GPU via compute shaders. This eliminates the CPU
decompression bottleneck entirely and reduces PCIe transfer size.

```
CPU path (current plan):
  mmap ──CPU decompress──► CPU buffer ──PCIe upload──► GPU vertex buffer
  Cost: CPU decompress time + full-size PCIe transfer

GPU path (future):
  mmap ──PCIe upload (compressed)──► GPU staging ──compute shader──► GPU vertex buffer
  Cost: small PCIe transfer + GPU compute (massively parallel)
```

**Approaches:**

```
┌─────────────────────┬──────────────┬─────────────────────────────────────┐
│ Approach            │ Availability │ Notes                               │
├─────────────────────┼──────────────┼─────────────────────────────────────┤
│ GDeflate            │ NVIDIA RTX   │ Part of RTX IO. Hardware-           │
│ (VK_NV_memory_      │ only         │ accelerated deflate. ~20-100 GB/s.  │
│  decompression)     │              │ Not in wgpu. Vendor-locked.         │
├─────────────────────┼──────────────┼─────────────────────────────────────┤
│ DirectStorage +     │ Windows,     │ Microsoft API. GPU decompression    │
│ GPU decompression   │ DX12 GPUs    │ built into storage pipeline.        │
│                     │              │ Not available via wgpu/Vulkan.      │
├─────────────────────┼──────────────┼─────────────────────────────────────┤
│ Custom LZ4 compute  │ Any GPU      │ LZ4 is simple enough for a compute │
│ shader              │ (wgpu)       │ shader. ~10-30 GB/s on modern GPU.  │
│                     │              │ Portable. Moderate effort.          │
├─────────────────────┼──────────────┼─────────────────────────────────────┤
│ Custom dequantize   │ Any GPU      │ Upload quantized u16 positions +   │
│ compute shader      │ (wgpu)       │ oct-encoded normals. Compute       │
│                     │              │ shader expands to f32 vertex buf.  │
│                     │              │ TRIVIAL to implement. High value.  │
├─────────────────────┼──────────────┼─────────────────────────────────────┤
│ Vertex shader       │ Any GPU      │ Dequantize in vertex shader (no    │
│ dequantize          │              │ compute needed). Upload u16 data   │
│                     │              │ as vertex attributes directly.     │
│                     │              │ SIMPLEST. Works today in wgpu.     │
└─────────────────────┴──────────────┴─────────────────────────────────────┘

Most promising near-term: vertex-shader dequantization.
  - Upload quantized vertex data (u16 positions, snorm16 normals) as-is
  - Custom vertex shader decodes to world-space in real time
  - No compute pass, no staging buffer, no CPU decompress for geometry
  - PCIe transfer: ~10 bytes/vertex instead of ~24 bytes/vertex (2.4×)
  - Pairs with general compression (zstd) for on-disk storage:
    mmap → zstd decompress (CPU, small) → upload quantized → vertex shader decode

Most promising long-term: custom LZ4 compute shader.
  - Upload compressed block directly to GPU
  - Compute shader decompresses into vertex buffer
  - Fully eliminates CPU from the hot path
  - Could decompress N frames ahead into a ring buffer of GPU vertex buffers
```

### Ahead-of-Playhead Decompression Strategy

Whether decompression happens on CPU or GPU, we decompress ahead of the
playhead to hide latency. This is analogous to video player buffering.

```
Timeline:
  ... [decoded] [decoded] [decoded] [PLAYING] [decompress] [compressed] ...
       frame-2   frame-1   frame+0   ▲         frame+1      frame+2
                                   playhead

Decompression ring buffer (CPU or GPU):
  ┌───────────────────────────────────────────────┐
  │ Slot 0: frame 100 (decoded, GPU-resident)     │
  │ Slot 1: frame 101 (decoded, GPU-resident)     │
  │ Slot 2: frame 102 (decoded, PLAYING)          │
  │ Slot 3: frame 103 (decoding async...)         │
  │ Slot 4: frame 104 (queued for decompress)     │
  │ Slot 5: frame 105 (queued for decompress)     │
  └───────────────────────────────────────────────┘
  Ring size = spawn_window_size (current default: 5)
  
  Each tick:
  1. Advance playhead → oldest slot becomes free
  2. Kick decompress for next frame into freed slot
  3. If decompress finishes before playhead reaches it → smooth
  4. If not → frame skip or stall (should be rare with 5-frame buffer)
```

**CPU async decompress (Phase 3):**
- `AsyncComputeTaskPool::spawn()` decompress tasks N frames ahead
- Poll completed tasks in `poll_completed_decodes`
- Feeds into existing promote_to_gpu / demote_from_gpu flow
- Simple, portable, works today

**GPU compute decompress (future):**
- Upload compressed blocks to GPU staging buffer
- Dispatch compute shader N frames ahead
- Output directly to vertex buffer (no CPU round-trip)
- Requires compute pipeline setup in Bevy/wgpu
- Pairs with ring buffer of pre-allocated GPU vertex buffers

### svmesh File Layout (revised with compression)

```
┌──────────────────────────────────────────────────────────────────┐
│ .svmesh file (single sequence frame)                             │
├──────────────────────────────────────────────────────────────────┤
│ Header (64 bytes, fixed):                                        │
│   magic: [u8; 4]          "SVMF"                                 │
│   version: u16            1                                      │
│   flags: u16              bitfield (quantized, delta, codec)     │
│   vertex_count: u32                                              │
│   index_count: u32                                               │
│   compression: u8         0=none, 1=lz4, 2=zstd                 │
│   encoding: u8            0=raw_f32, 1=quant_u16, 2=delta       │
│   keyframe_ref: u32       frame# of reference keyframe (or self) │
│   bbox_min: [f32; 3]      bounding box (for dequantization)     │
│   bbox_max: [f32; 3]      bounding box                          │
│   positions_offset: u32   offset to positions block              │
│   positions_size: u32     compressed size in bytes               │
│   normals_offset: u32     offset to normals block                │
│   normals_size: u32       compressed size                        │
│   indices_offset: u32     offset to indices block                │
│   indices_size: u32       compressed size                        │
│   padding to 64 bytes                                            │
├──────────────────────────────────────────────────────────────────┤
│ Positions block (4K-aligned):                                    │
│   [compressed or raw vertex position data]                       │
├──────────────────────────────────────────────────────────────────┤
│ Normals block (4K-aligned):                                      │
│   [compressed or raw vertex normal data]                         │
├──────────────────────────────────────────────────────────────────┤
│ Indices block (4K-aligned):                                      │
│   [compressed or raw index data]                                 │
└──────────────────────────────────────────────────────────────────┘

Uncompressed (encoding=raw_f32, compression=none):
  → mmap + pointer cast, true zero-copy (current svmesh plan)

Quantized + zstd (encoding=quant_u16, compression=zstd):
  → mmap + zstd decompress + dequantize (CPU or vertex shader)

Delta + zstd (encoding=delta, compression=zstd):
  → mmap + zstd decompress + apply delta to keyframe + dequantize
```

### Open Questions (research further)

**Compression benchmarking:**
- [ ] Benchmark zstd vs LZ4 on actual fluid sim mesh data (our frames)
- [ ] Benchmark meshopt vertex codec alone vs meshopt + zstd on our frames
- [ ] Measure vertex reorder impact on compression ratio (meshoptimizer)
- [ ] Is zstd dictionary mode viable? (train dictionary on N sample frames,
      use for all frames in sequence — could improve ratio significantly)

**meshopt integration:**
- [ ] Test meshopt 0.6.2 Rust crate encode/decode roundtrip on our GLB data
- [ ] Measure meshopt_encodeVertexBuffer ratio on our fluid sim vertices
      (likely ~2-4× before zstd, ~5-10× with zstd on top)
- [ ] Test meshopt filters: encodeFilterOct on normals, encodeFilterExp
      on positions — measure quality loss vs compression gain
- [ ] Can meshopt's vertex codec handle varying vertex counts per frame?
      (fluid sim meshes may change topology between frames)
- [ ] Evaluate meshopt_simplify for LOD generation in seaview-pipeline

**GPU decode path:**
- [ ] Port meshopt vertex decoder to wgpu compute shader (WGSL)
      — reference: js/meshopt_decoder_reference.js in meshopt repo
      — key: prefix-sum within workgroup for delta decode
- [ ] Prototype quantized vertex upload + vertex shader dequantize in wgpu
      (simpler than compute decode — may be sufficient near-term)
- [ ] Benchmark GPU decode throughput vs CPU decode on our GPU

**Temporal / sequence-specific:**
- [ ] Evaluate delta encoding: how correlated are consecutive frames?
      (run cross-frame XOR + compression test on real data)
- [ ] Can meshopt's delta encoding within a block exploit temporal coherence
      if we interleave vertices from consecutive frames? (probably not —
      better to do explicit frame-delta as a layer above meshopt)
- [ ] Keyframe + delta: does meshopt encode deltas better than raw frames?
      (small residuals → more zeros → better byte-lane compression)

**Format / architecture:**
- [ ] Memory layout: per-frame files vs single packed sequence file?
      (single file = one mmap, better for OS page cache; but harder to
      append/modify; per-frame = current model, simpler)
- [ ] Should svmesh support both raw (v1, zero-copy) and meshopt (v2)
      layouts in the same format? Or separate extensions (.svmesh / .svmeshc)?
- [ ] EXT_meshopt_compression in GLB: should mesh_io support loading these
      directly? (would let us load gltfpack-optimized GLBs without Bevy's
      gltf loader, which doesn't support the extension)

## Draco vs meshopt: Internals and GPU Decode

### How Draco Works

Google Draco is an offline mesh compression library optimized for maximum ratio.

```
Encode (offline, slow):
  1. Connectivity encoding (EdgeBreaker algorithm)
     - Traverses mesh topology, encodes triangle adjacency
     - Produces a compact traversal sequence (C, L, R, S, E symbols)
     - Fundamentally sequential — must visit triangles in order

  2. Attribute quantization
     - Positions: quantized to N-bit integers within bounding box
     - Normals: octahedral projection → 2 components
     - Configurable bit depth per attribute

  3. Prediction coding
     - Parallelogram prediction for positions:
       P_predicted = A + B - C  (from adjacent decoded triangle)
     - Only residuals (corrections) are stored
     - Requires sequential access to previously decoded vertices

  4. Entropy coding (rANS)
     - Asymmetric numeral systems on residual symbols
     - Excellent compression but inherently sequential decode

Decode (runtime):
  rANS decode → undo prediction → dequantize → reconstruct connectivity
  All steps are sequential. ~50-200 MB/s decoded output.
```

**Why Draco CANNOT do GPU decompression:**
- EdgeBreaker connectivity decoding is graph traversal — not parallelizable
- rANS entropy decoding is inherently sequential (each symbol depends on prior state)
- Prediction requires already-decoded neighbors
- No production GPU decoder exists. Academic papers on parallel EdgeBreaker
  exist but are impractical for real-time use.

### How meshopt Works

meshoptimizer (by Arseny Kapoulkine / zeux) is designed for fast runtime
decode, including GPU compute shader decode. Very different philosophy.

```
Vertex buffer codec (meshopt_encodeVertexBuffer / meshopt_decodeVertexBuffer):

  1. Byte-level deinterleaving
     For stride-S vertices, create S separate byte streams:
       stream[0] = byte 0 of vertex 0, byte 0 of vertex 1, ...
       stream[1] = byte 1 of vertex 0, byte 1 of vertex 1, ...
       ...
     This groups identical byte positions together (exploits structure)

  2. Delta encoding per byte stream
     Each byte stream: store delta from previous value
     Consecutive vertices with similar values → small deltas → compressible

  3. Block-based variable-length encoding
     Vertices grouped into blocks of up to 256
     Each block has a small header + packed delta data
     KEY: blocks are independently decodable → GPU-parallel

  No entropy coding. Deliberately simple for fast decode.

Index buffer codec (meshopt_encodeIndexBuffer / meshopt_decodeIndexBuffer):
  - Encodes triangle connectivity as edge-based references
  - Targets ~1 byte per triangle (6× smaller than raw u32×3)
  - Also block-based and fast to decode

Decode: ~3-6 GB/s on modern CPU (can target write-combined memory directly)
```

**Why meshopt CAN do GPU decompression:**
- Block-of-256-vertices format: each block decoded independently
- No entropy coding — just byte shuffles and delta adds
- ~20 ALU ops per vertex group → trivial for GPU compute
- Arseny explicitly designed the format with GPU decode in mind
- Reference JS decoder shows the algorithm; GLSL/WGSL port is straightforward
- `EXT_meshopt_compression` glTF extension uses this format

### Head-to-Head Comparison

```
┌─────────────────────────┬──────────────────┬──────────────────────────┐
│                         │ Draco            │ meshopt                  │
├─────────────────────────┼──────────────────┼──────────────────────────┤
│ Compression ratio       │ 10-20×           │ 2-4× (vertex codec)     │
│ (vs raw f32)            │                  │ + zstd → 5-10× total    │
├─────────────────────────┼──────────────────┼──────────────────────────┤
│ CPU decode speed        │ ~200 MB/s        │ 3-6 GB/s                │
│                         │                  │ (15-30× faster)          │
├─────────────────────────┼──────────────────┼──────────────────────────┤
│ GPU decode              │ ❌ Impossible     │ ✅ Designed for it       │
│                         │ (sequential alg) │ (block-parallel)         │
├─────────────────────────┼──────────────────┼──────────────────────────┤
│ glTF extension          │ KHR_draco_mesh_  │ EXT_meshopt_compression  │
│                         │ compression      │ (also KHR_meshopt_...)   │
├─────────────────────────┼──────────────────┼──────────────────────────┤
│ Lossy?                  │ Yes (quantize)   │ Codec: lossless          │
│                         │                  │ Filters: lossy (opt-in)  │
├─────────────────────────┼──────────────────┼──────────────────────────┤
│ Vertex order preserved? │ ❌ Reorders       │ ✅ Preserved             │
│                         │ (kills GPU perf) │ (cache-friendly order)   │
├─────────────────────────┼──────────────────┼──────────────────────────┤
│ Rust crate              │ draco_decoder    │ meshopt 0.6.2            │
│                         │ (C++ via cxx)    │ (C++ via FFI, mature)    │
├─────────────────────────┼──────────────────┼──────────────────────────┤
│ Best use                │ Archival,        │ Runtime playback,        │
│                         │ distribution     │ GPU-friendly storage     │
└─────────────────────────┴──────────────────┴──────────────────────────┘
```

### meshopt Vertex Filters (lossy pre-processing)

meshopt provides filters that reduce entropy before encoding. These are
lossy transforms applied at encode time, reversed at decode time:

```
┌───────────────────┬──────────────────────────────────────────────────┐
│ Filter            │ What it does                                     │
├───────────────────┼──────────────────────────────────────────────────┤
│ Octahedral        │ 3×f32 normal → 2 components (snorm N-bit)       │
│ (encodeFilterOct) │ Any bit depth 2-16. Decode: ~5-10 GB/s          │
├───────────────────┼──────────────────────────────────────────────────┤
│ Exponential       │ f32 values → shared-exponent representation     │
│ (encodeFilterExp) │ Reduces mantissa entropy. Good for positions.   │
├───────────────────┼──────────────────────────────────────────────────┤
│ Quantize helpers  │ meshopt_quantizeHalf (f32→f16)                  │
│                   │ meshopt_quantizeSnorm (f32→snormN)              │
│                   │ meshopt_quantizeFloat (strip low mantissa bits) │
└───────────────────┴──────────────────────────────────────────────────┘

Pipeline for our meshes:
  1. meshopt_optimizeVertexCache (reorder triangles)
  2. meshopt_optimizeVertexFetch (reorder vertices for locality)
  3. meshopt_quantizeHalf or meshopt_quantizeFloat on positions
  4. meshopt_encodeFilterOct on normals (oct-encode, 10-12 bits)
  5. meshopt_encodeVertexBuffer + meshopt_encodeIndexBuffer
  6. (optional) zstd compress on top for storage

  Result: ~5-10× compression, 3-6 GB/s decode, GPU-decodable.
```

### GPU Decode Path for meshopt (concrete plan)

```
meshopt's vertex codec block layout (simplified):

  ┌─────────┬──────────────────────────────────────────┐
  │ Header  │ Up to 256 vertices, byte-deinterleaved   │
  │ (1 byte)│ delta-encoded per byte lane               │
  └─────────┴──────────────────────────────────────────┘
  │←─────── one group, independently decodable ────────►│

wgpu compute shader decode:

  @compute @workgroup_size(256)
  fn decode_vertex_group(
      @builtin(global_invocation_id) gid: vec3<u32>,
  ) {
      let group_idx = gid.x / 256;   // which group
      let vert_idx  = gid.x % 256;   // which vertex in group

      // 1. Read header byte for this group
      // 2. For each byte lane of stride:
      //    a. Read delta from encoded buffer
      //    b. Prefix-sum within workgroup (parallel scan)
      //    c. Add to anchor value
      // 3. Reinterleave bytes into vertex struct
      // 4. Write to output vertex buffer

      output_vertices[base + vert_idx] = decoded_vertex;
  }

  Dispatch: ceil(vertex_count / 256) workgroups
  Cost: ~20 ALU ops per vertex, fully parallel
  Throughput: ~10-50 GB/s on modern GPU (memory-bound)
```

### Revised Recommendation: meshopt for svmesh

```
Previous plan:  raw f32 data + optional zstd/LZ4 compression
Revised plan:   meshopt-encoded vertex/index buffers (+ optional zstd on top)

Why meshopt over rolling our own:
  ✓ Battle-tested (used by glTF ecosystem, Three.js, gltfpack)
  ✓ GPU-decodable by design (our future path)
  ✓ meshopt 0.6.2 Rust crate is mature (310k downloads)
  ✓ Lossless codec + opt-in lossy filters = flexible
  ✓ Pairs with zstd for additional 2-3× on disk
  ✓ Vertex reorder + cache optimization included free

Why not Draco for runtime:
  ✗ Sequential decode kills real-time playback throughput
  ✗ No GPU decode path (ever)
  ✗ Reorders vertices (defeats GPU vertex cache optimization)
  ✗ Only useful for distribution / archival format

Draco role: Import only.
  - mesh_io::draco_glb.rs decodes Draco GLB → MeshData
  - Pipeline can accept Draco input and re-encode as svmesh
  - Not used for runtime playback format
```

### svmesh Revised Format (meshopt-based)

```
┌──────────────────────────────────────────────────────────────────┐
│ .svmesh file layout (meshopt-encoded)                            │
├──────────────────────────────────────────────────────────────────┤
│ Header (64 bytes):                                               │
│   magic: [u8; 4]          "SVMF"                                 │
│   version: u16            2 (meshopt-based)                      │
│   flags: u16              bitfield                               │
│   vertex_count: u32                                              │
│   index_count: u32                                               │
│   vertex_stride: u16      bytes per vertex (after quantization)  │
│   outer_compression: u8   0=none, 1=zstd (wraps meshopt blocks)  │
│   filter_flags: u8        bit 0=oct_normals, bit 1=exp_positions │
│   keyframe_ref: u32       for delta encoding (0 = self/keyframe) │
│   bbox_min: [f32; 3]      for dequantization                    │
│   bbox_max: [f32; 3]                                             │
│   vertex_block_offset: u32                                       │
│   vertex_block_size: u32  meshopt-encoded vertex data size       │
│   index_block_offset: u32                                        │
│   index_block_size: u32   meshopt-encoded index data size        │
│   padding to 64 bytes                                            │
├──────────────────────────────────────────────────────────────────┤
│ Vertex data (meshopt_encodeVertexBuffer output):                 │
│   [possibly zstd-wrapped meshopt blocks]                         │
│   Internally: byte-deinterleaved, delta-encoded, block-of-256   │
│   GPU-decodable as-is (without outer zstd)                       │
├──────────────────────────────────────────────────────────────────┤
│ Index data (meshopt_encodeIndexBuffer output):                   │
│   [possibly zstd-wrapped meshopt blocks]                         │
│   Targets ~1 byte per triangle                                   │
└──────────────────────────────────────────────────────────────────┘

Two runtime paths:

  CPU path (Phase 3):
    mmap → [zstd decompress →] meshopt_decodeVertexBuffer
         → meshopt_decodeIndexBuffer → MeshData → Bevy Mesh
    Speed: 3-6 GB/s decode. For 8 MB encoded frame: ~1-2 ms.

  GPU path (future):
    mmap → upload meshopt blocks to GPU staging buffer
         → compute shader decode → vertex/index buffer
    Speed: 10-50 GB/s. For 8 MB encoded frame: ~0.2 ms.
    Requires: no outer zstd (or GPU zstd first), wgpu compute pipeline.
```

## GLB Zero-Copy-ish Bridge (From<gltf::Buffer> → Bevy Mesh)

For uncompressed GLB files (our current format), we can skip most of Bevy's
gltf loading machinery and go nearly zero-copy from the GLB binary chunk:

```
Standard Bevy GLB path (current):
  file → AssetServer → bevy_gltf (parse full scene graph, materials,
  cameras, lights, animations) → extract Mesh primitive → Handle<Mesh>

  Overhead: full scene parse, material creation, label resolution

Direct GLB path (mesh_io::glb):
  file → gltf crate (parse JSON header only) → locate buffer views
       → bytemuck::cast_slice(&[u8] → &[[f32; 3]]) → MeshData → Mesh

  The binary chunk of a GLB contains raw vertex data:
  ┌─────────────┬────────────────────────────────────┐
  │ JSON chunk  │ Binary chunk                       │
  │ (metadata)  │ [positions][normals][indices]       │
  │             │  ← accessor offsets point here →    │
  └─────────────┴────────────────────────────────────┘

  For standard uncompressed GLB with f32 attributes:
  - positions accessor → &[u8] in binary chunk → cast to &[[f32; 3]]
  - normals accessor   → &[u8] in binary chunk → cast to &[[f32; 3]]
  - indices accessor   → &[u8] in binary chunk → cast to &[u32]

  These casts are zero-copy (just pointer reinterpretation).
  The only allocation is building the final Vec for Bevy's Mesh.
```

```rust
// mesh_io::glb — direct GLB decode (sketch)
pub fn decode(bytes: &[u8]) -> Result<MeshData, MeshIoError> {
    let (document, buffers, _) = gltf::import_slice(bytes)?;

    let mesh = document.meshes().next().ok_or(MeshIoError::NoMesh)?;
    let prim = mesh.primitives().next().ok_or(MeshIoError::NoPrimitive)?;

    let reader = prim.reader(|buf| buffers.get(buf.index()).map(|d| &d[..]));

    // These iterate over the raw buffer — minimal copying
    let positions: Vec<[f32; 3]> = reader.read_positions()
        .ok_or(MeshIoError::NoPositions)?
        .collect();

    let normals: Vec<[f32; 3]> = reader.read_normals()
        .unwrap_or_default()  // generate later if missing
        .collect();

    let indices: Vec<u32> = reader.read_indices()
        .ok_or(MeshIoError::NoIndices)?
        .into_u32()
        .collect();

    Ok(MeshData { positions, normals, indices })
}
```

For **meshopt-compressed GLB** (`EXT_meshopt_compression`):
```
  Buffer views point to meshopt-encoded data instead of raw f32.
  Extension JSON on each buffer view specifies:
    { "buffer": N, "byteOffset": ..., "byteLength": ...,
      "byteStride": 12, "count": 50000, "mode": "ATTRIBUTES" }

  Decode: read encoded bytes → meshopt_decodeVertexBuffer → raw vertices
  Not zero-copy, but decode at 3-6 GB/s makes it near-instant.
```

For **Draco-compressed GLB** (`KHR_draco_mesh_compression`):
```
  Extension on each primitive specifies:
    { "bufferView": N, "attributes": { "POSITION": 0, "NORMAL": 1 } }

  Decode: read compressed bytes → draco_decoder::decode_mesh_with_config_sync
       → map Draco attribute IDs back to POSITION/NORMAL → MeshData
  Slower (~200 MB/s) but still faster than current Bevy path.
  Import-only: re-encode as svmesh for runtime playback.
```

## Shared Library Potential

`mesh_io` is deliberately Bevy-free at its core. The `Into<bevy::Mesh>` impl lives behind a feature flag. This means:

- **seaview** uses it with `bevy` feature for viewing
- **seaview-pipeline** uses it without Bevy for processing/conversion
- Could be published as a standalone crate for the Bevy ecosystem (there's nothing like this today)