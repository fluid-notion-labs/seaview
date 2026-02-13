# Plan: Visibility-Swap Playback (eliminate GPU alloc/free churn)

## Problem

During sequence playback, swapping the `Mesh3d` handle on a single entity triggers
Bevy's `allocate_and_free_meshes` in the render world — freeing the old GPU buffer
and allocating a new one every frame change. This costs 50ms avg, spikes to 2.6s,
and gets worse over time due to GPU memory fragmentation.

**Branch:** `perf/visibility-swap-playback`  
**Parent:** `perf-and-logging`

## Root Cause (from chrome trace analysis)

- 91% of frames: `allocate_and_free_meshes` ≤ 0.1ms (idle, no mesh swap)
- Frame-change spikes: 100ms → 2600ms (escalating due to fragmentation)
- The handle swap in `update_mesh_display` / `handle_frame_changes` triggers free+alloc
- Each frame: ~700K verts, ~1.4M tris, ~33MB GPU buffer

## Solution: Pre-spawn hidden entities, toggle Visibility

Instead of 1 entity with handle-swapping, spawn 1 entity per loaded frame at load
time with `Visibility::Hidden`. On frame change, hide the old entity and show the
new one. GPU buffer allocation is amortized across the loading phase. Zero
alloc/free during playback.

## Files to Change

### 1. `crates/seaview/src/lib/sequence/loader.rs`

**SequenceAssets changes:**
- Replace `mesh_entity: Option<Entity>` with `frame_entities: Vec<Option<Entity>>`
- Keep `displayed_frame: Option<usize>` (unchanged semantics)
- Add helper: `fn displayed_entity(&self) -> Option<Entity>`

**`track_asset_loading` system changes:**
- Currently spawns a single entity when frame 0 loads, then reuses it
- New: when ANY frame loads (`LoadedWithDependencies`), spawn a hidden entity for
  that frame:
  ```
  commands.spawn((
      Mesh3d(handle),
      MeshMaterial3d(material.clone()),
      transform,
      SequenceMeshDisplay,
      Visibility::Hidden,
      Name::new(format!("Frame {}", index)),
  ));
  ```
- Store entity in `sequence_assets.frame_entities[index]`
- Special-case frame 0: spawn with `Visibility::Visible` instead, set
  `displayed_frame = Some(0)`
- Material and transform: create once and clone for each entity (same as current
  single-entity approach)

**`update_mesh_display` system — rewrite:**
- Current: swaps `Mesh3d` handle on single entity (THE BOTTLENECK)
- New: if `displayed_frame != current_frame`:
  1. Set old entity `Visibility::Hidden`
  2. Set new entity `Visibility::Visible` (if it exists / is spawned)
  3. Update `displayed_frame`
- Query changes: `Query<&mut Visibility, With<SequenceMeshDisplay>>` is insufficient
  — we need to target specific entities. Use `commands.entity(id).insert(Visibility::...)` 
  or store entity IDs and use a direct query with `Entity` filter.
- Simplest: `commands.entity(old).insert(Visibility::Hidden)` + 
  `commands.entity(new).insert(Visibility::Visible)`

**`handle_frame_changes` system — rewrite:**
- Same visibility-toggle approach instead of handle swap
- Can share logic with `update_mesh_display` (or merge them — they both respond to
  frame index changes from different sources)

**`SequenceAssets::reset` changes:**
- Despawn all `frame_entities` (need `Commands` or queue despawn)
- Alternatively, add a cleanup system that watches for reset

### 2. `crates/seaview/src/lib/sequence/mod.rs`

- No structural changes needed. `SequenceManager` frame tracking is unchanged.

### 3. `crates/seaview/src/app/ui/systems/material_system.rs`

- Check how `apply_material_config` targets the mesh entity — it currently queries
  `With<SequenceMeshDisplay>`. Since ALL frame entities have this component, material
  changes will apply to all of them. **This is actually correct** — we want all frames
  to share the same material appearance.
- But: currently it might mutate `MeshMaterial3d` on many entities. Verify perf is OK
  or switch to a shared `Handle<StandardMaterial>` resource.

### 4. `crates/seaview/src/app/systems/diagnostics.rs`

- `update_rendering_stats` queries `(&Mesh3d, &Visibility), With<Mesh3d>` — it needs
  to only count `Visibility::Visible` meshes for the "visible" count, which it already
  does (checks `visibility == Visibility::Hidden`). Should be fine as-is.

### 5. `crates/seaview/src/lib/mesh_info.rs`

- Grep for `SequenceMeshDisplay` / `Mesh3d` queries to check nothing assumes a
  single entity. Anything computing bounds should target only the visible frame.

## Memory Considerations

- 388 frames × 33MB = ~12.8GB GPU VRAM for full pre-allocation
- This is fine for workstation GPUs (16-24GB common)
- **Future optimization (not this PR):** ring-buffer strategy — keep N frames
  (e.g. 30) in GPU memory centered on current position, despawn distant frames.
  Would add a `max_gpu_frames: usize` config to `SequenceAssets`.

## Cleanup / Sequence Reset

When loading a new sequence, all old frame entities must be despawned:
- Add a `cleanup_old_sequence` step at the top of `handle_load_requests`
- Iterate `sequence_assets.frame_entities`, despawn each `Some(entity)`
- Then call `sequence_assets.reset()`

## Testing

1. Load a sequence, verify all frames render (step through with arrow keys)
2. Playback should be smooth — no stutter on frame changes
3. Check GPU memory usage (nvidia-smi or similar) — should be ~12-13GB for the
   gold dataset, stable (no growth over time)
4. Material changes should apply to all frames uniformly
5. Mesh info / bounds should report only the visible frame
6. Loading a second sequence should cleanly despawn the first

## Order of Implementation

1. Update `SequenceAssets` struct (field changes)
2. Rewrite `track_asset_loading` (per-frame entity spawn)
3. Rewrite `update_mesh_display` (visibility toggle)
4. Rewrite `handle_frame_changes` (visibility toggle)
5. Add cleanup in `handle_load_requests`
6. Verify `material_system`, `diagnostics`, `mesh_info` still work
7. `cargo check`, test manually