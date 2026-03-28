# Handle-Swap Refactor for frame_window

## Problem

`manage_frame_window` in `crates/seaview/src/lib/sequence/loader/frame_window.rs` currently spawns and despawns entities every frame tick. For meshlet sequences this triggers GPU persistent buffer write/free cycles in `perform_pending_meshlet_mesh_writes`, which eventually causes an integer overflow crash at `bevy_pbr/meshlet/persistent_buffer_impls.rs:70`. Standard mesh sequences also pay unnecessary archetype-move overhead.

## Current Architecture

- `manage_frame_window` runs in `PostUpdate`
- Each frame tick: despawn old entities → spawn new entities with `Mesh3d(handle)` or `MeshletMesh3d(handle)` + `MeshMaterial3d` + `Transform` + `SequenceMeshDisplay` + `Visibility`
- Frame window size controls how many frames have spawned entities (visible + hidden buffer)
- Entity references stored in `frame_store.frame_entities[idx]` (active series) and `series_fs.get(&id).entities[idx]` (multi-series)
- Key file: `crates/seaview/src/lib/sequence/loader/frame_window.rs` — the `manage_frame_window` system (~500 lines)

## Desired Architecture

**One persistent "display entity" per series** that lives for the lifetime of the series. On frame tick, swap the handle component rather than despawn/respawn:

```rust
// For meshlet:
commands.entity(display_entity).insert(MeshletMesh3d(new_handle));
// For standard mesh:
commands.entity(display_entity).insert(Mesh3d(new_handle));
```

For frame_window > 1, the extra hidden buffer entities around the window edges can still use spawn/despawn — only the actively displayed frame needs handle-swap.

## Key Data Structures

**`SeriesEntityState`** (`crates/seaview/src/lib/sequence/loader/series_frame_store.rs`):
- `entities: Vec<Option<Vec<Entity>>>` — per-frame entity list
- Needs a new field: `display_entity: Option<Entity>` — the persistent entity for this series

**`FrameStore`** (`crates/seaview/src/lib/sequence/loader/assets.rs`):
- `frame_entities: Vec<Option<Vec<Entity>>>` — same pattern for active series
- Needs: `display_entity: Option<Entity>`

**`SequenceKind`** (`crates/seaview/src/lib/sequence/loader/mod.rs`):
- `Mesh(MeshFormat)` vs `Meshlet` — determines which component to swap

## Implementation Steps

1. Add `display_entity: Option<Entity>` to `FrameStore` and `SeriesEntityState`

2. On first frame display (no display_entity yet): spawn the entity as now, store it as `display_entity`

3. On subsequent frame ticks: instead of despawn+spawn, do:
   ```rust
   if let Some(entity) = display_entity {
       match sequence_kind {
           SequenceKind::Meshlet => {
               commands.entity(entity).insert(MeshletMesh3d(new_handle));
           }
           SequenceKind::Mesh(_) => {
               commands.entity(entity).insert(Mesh3d(new_handle));
           }
       }
   }
   ```

4. Only despawn the display_entity when the series is removed or the sequence is unloaded

5. For frame_window > 1: hidden buffer entities still spawn/despawn as before. The display_entity is separate from the buffer entity pool.

## Meshlet-Specific Notes

- The meshlet fast-path is at line ~368 in frame_window.rs (`#[cfg(feature = "meshlet")]` block)
- Standard mesh path is at line ~460
- Both share the same `material` handle from `loader_config.shared_material`
- The `Transform` comes from `series_transform` — this doesn't change between frames within a series, so the display entity's transform stays valid

## Files to Touch

- `crates/seaview/src/lib/sequence/loader/frame_window.rs` — main refactor
- `crates/seaview/src/lib/sequence/loader/assets.rs` — add `display_entity` to `FrameStore`
- `crates/seaview/src/lib/sequence/loader/series_frame_store.rs` — add `display_entity` to `SeriesEntityState`

## What NOT to Change

- Loading/tracking systems (`load_queue.rs`, `loading_tracker.rs`) — unchanged
- Discovery (`discovery.rs`) — unchanged
- Material system (`material_system.rs`) — queries `SequenceMeshDisplay` which the display entity still has
- The `SequenceMeshDisplay` marker component stays on the display entity

## Testing

- Regular STL sequence: should play back identically
- Meshlet sequence: should play back without the overflow crash
- Frame window > 1: hidden entities still buffer correctly
- Pause + scrub: handle swap works for non-sequential frame changes
- Series visibility toggle: display entity visibility still works

## Crash Details

The overflow crash occurs in `bevy_pbr-0.18.0/src/meshlet/persistent_buffer_impls.rs:70` inside `write_bytes_le` for `Arc<[Meshlet]>`. During rapid playback (~25-30 FPS, frame advancing every ~40ms), spawn/despawn cycles cause the persistent GPU buffer to accumulate pending writes faster than it reclaims space from despawned meshes. The buffer offset computation overflows. This is a Bevy bug (should be a graceful error, not a panic) but the handle-swap pattern avoids triggering it entirely.

## Branch Context

All work is on `feature/meshlet-format`. Recent commits:
- `e0e7266` — defer MeshletPlugin registration (fixes bind group crash)
- `23a4933` — forward rendering (fixes gray mesh from deferred+meshlet)
- `73da044` — fix spurious MaterialConfig change detection (fixes panel expand going dark)

The deferred renderer + SSR were removed because Bevy 0.18's meshlet pipeline
has bind group layout incompatibilities with deferred rendering when standard
Mesh3d entities coexist with MeshletMesh3d entities. Forward rendering works
correctly. SSR plumbing (UI panel, sync system, settings persistence) remains
inert — can be re-enabled when Bevy stabilises meshlet+deferred.
