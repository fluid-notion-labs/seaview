# Seaview Architecture

Bevy-based 3D mesh viewer for time-series simulation data. Loads sequences of mesh files (STL, glTF/GLB) and plays them back with memory-budget-aware frame management.

## Layers

```
src/
├── lib/          Pure logic. Testable without Bevy.
├── app/          Bevy systems (thin wrappers) + egui UI.
└── bin/          Standalone tools. Import lib/ only.
```

| Layer | Allowed | Forbidden |
|-------|---------|-----------|
| `lib/` | Pure types, plain methods, `derive(Resource)` | System signatures, `Res<>`, `Query<>`, `Commands` |
| `app/` | Read resources → call `lib/` → write back | Business logic beyond glue |
| `bin/` | Import from crate public API (`seaview::`) | Import from `app/`, Bevy runtime |

## Key Files

| File | What |
|------|------|
| `lib/sequence/mod.rs` | `Timeline`, `LoadSection`, `compute_spawn_window()`, `compute_spawn_windows()`, `distribute_sections()`, `union_frames_to_load()` |
| `lib/sequence/loader/` | Loader module: `assets.rs` (FrameStore, LoaderConfig, AssetIndex), `mesh_cache.rs` (MeshCache, CacheEntry), `series_frame_store.rs` (SeriesFrameStore, SeriesEntityState), `frame_window.rs` (manage_frame_window), `load_queue.rs` (sync_section_loads, drain_load_queue), `loading_tracker.rs` (track_asset_loading) |
| `lib/sequence/playback.rs` | Keyboard playback controls |
| `lib/sequence/discovery.rs` | Filesystem sequence detection |
| `lib/sequence/watch.rs` | Directory watch mode, incremental loading |
| `lib/memory_budget.rs` | `MemoryBudget`, `VramDetector`/`RamDetector` traits, `NvmlBackend` |
| `lib/settings.rs` | `Settings`, `LoadSectionSettings`, seaview.toml persistence |
| `lib/lighting/` | `NightLightingConfig`, placement algorithms |
| `app/ui/state.rs` | `UiState`, `RenderConfig`, `MaterialConfig`, `LoadSectionEdge` |
| `app/ui/systems/playback_controls/` | Bottom panel module: `mod.rs` (orchestrator), `transport.rs`, `timeline_bar.rs`, `load_section_bar.rs` |
| `app/ui/systems/environment_panel/` | Left panel: lighting, materials, rendering, perf |

## Resource Ownership

One authority per domain. `UiState` fields are read-only mirrors synced once per tick.

| Domain | Authority | Mirror |
|--------|-----------|--------|
| Timeline | `Timeline` (`total_frames`, `sections`, `playhead`) | `UiState.playback` |
| Playback | `SequenceManager` (`is_playing`, `speed`, `loop_enabled`, `Sequence`) | `UiState.playback` |
| Mesh assets | `MeshCache` (shared `Handle<Mesh>` keyed by `PathBuf`) | `FrameStore.frame_handles` (mirror) |
| Entity tracking | `SeriesFrameStore` (per-series spawned entities) | `FrameStore.frame_entities` (mirror) |
| Asset loading | `FrameStore` (loading state, progress, paths) | `UiState.loading` |
| Memory budget | `MemoryBudget` | — |
| Appearance | `MaterialConfig` + `NightLightingConfig` | — |
| UI interaction | `UiState` (`load_section_dragging`, panel visibility, etc.) | — |

## Data Flow

```
CLI path → discovery → LoadSequenceRequest → AssetServer (async)
  → Timeline.init_sequence(total, max_budget)
  → SequenceAssets tracks handles + loaded state

Update:    advance_playback → Timeline.playhead++
           update_timeline_budget → Timeline.enforce_budget()
           sync_ui → Timeline + SequenceManager → UiState

PostUpdate: manage_frame_window → compute_spawn_window() → spawn/despawn entities
```

## Feature-Gated Plugins

Optional subsystems use the **plugin + optional resource** pattern to minimise `cfg` churn:

1. Bundle all resources, systems, and UI for the feature into a single `Plugin`.
2. Gate the plugin module declaration (`lib.rs`) and registration (`main.rs`) with `#[cfg(feature = "...")]` — **two `cfg` gates total**.
3. Systems that *consume* the optional resources use `Option<Res<T>>` / `Option<ResMut<T>>` — no `cfg` needed, they just skip work when the resource is absent.

Example: `SpatialAcceleratorPlugin` (`app/spatial_accelerator.rs`) owns `SpatialIndex`, `BvhGizmoConfig`, `build_spatial_meshes`, and `draw_bvh_gizmos`. Enabled with `--features spatial`.

## Standards

**Pure function extraction.** >20 lines of logic in a system → extract to testable `lib/` function. System becomes read → call → write.

**No UI state in domain structs.** Interaction state (drag, hover) lives in `UiState`.

**Settings ↔ Runtime.** `From`/`Into` impls between TOML types and runtime types.

**File size signals.** System >200 lines → split. Module >500 lines (excl. tests) → factor out.

**Debug binaries.** Reuse `lib/` types. If something is Bevy-entangled, extract it first.

## Current Debt

**`FrameStore` mirrors** — `FrameStore.frame_entities` and `FrameStore.displayed_frame` are kept as mirrors of `SeriesFrameStore` for backward compat with UI/spatial-index consumers. Once all consumers migrate to reading from `SeriesFrameStore` directly, the mirror fields can be removed. **Trigger:** when UI systems are updated to read per-series entity state.

**Debug binary duplication** — `debug_loader.rs` and `timeline_debug.rs` reimplement discovery, mesh parsing, GPU detection. `timeline_debug` partially fixed (uses library `LoadSection`/`distribute_sections`). **Trigger:** when discovery is extracted to a pure function.

### Resolved Debt

**`SequenceAssets` monolith** — ✅ Resolved by seaview-5zf. Split into `FrameStore` (paths, loading state), `MeshCache` (shared mesh handles), `SeriesFrameStore` (per-series entities), `LoaderConfig` (material, coord transform), `AssetIndex` (reverse lookups).

**`manage_frame_window` (loader.rs)** — ✅ Resolved by seaview-wme. Now uses `compute_spawn_windows()` across all sections with delta-based eviction/spawn.

**`playback_controls.rs`** — ✅ Resolved by seaview-wme. Split from 575-line monolith into `playback_controls/` module: `mod.rs` (orchestrator), `transport.rs`, `timeline_bar.rs`, `load_section_bar.rs`.

## Feature Map

| Issue | Touches | Risk | Status |
|-------|---------|------|--------|
| seaview-blz (light controls) | `lighting_section.rs`, `NightLightingConfig` | Low | ✅ Done |
| seaview-4sg (toasts) | New `UiState` fields, new UI system | Low | ✅ Done |
| seaview-9wx (multi-GPU) | `memory_budget.rs` new backends | Low | Blocked / punted |
| seaview-nvh (HUD overlays) | New UI files, reads existing resources | Low | Open |
| seaview-wme (multi-section) | `playback_controls/`, `manage_frame_window`, Timeline | Medium | ✅ Done |
| seaview-5zf (multi-series GPU) | `loader/` (mesh_cache, series_frame_store, frame_window, load_queue) | Medium | ✅ Done (Phases 1–3) |

## Multi-Section Architecture (seaview-wme)

**Timeline** owns `Vec<LoadSection>`. Methods: `add_section`, `remove_section`, `next_loaded_frame`, `prev_loaded_frame`, `is_frame_loaded`, `init_multi_section`.

**`compute_spawn_windows`** — multi-section generalisation of `compute_spawn_window`. Returns a `Vec<(lo, hi)>` for each section that overlaps the render window.

**`manage_frame_window`** — tracks `Local<Vec<(usize, usize)>>` (previous windows). Evicts/spawns the delta between old and new window sets.

**`advance_playback`** — skips gaps between sections. When the next frame is unloaded, jumps to the start of the next section. Looping wraps to the first loaded frame.

**UI interactions** (in `playback_controls/load_section_bar.rs`):
- Ctrl+Click on empty area → `timeline.add_section(frame, budget)`
- Right-click on section → `timeline.remove_section(idx, budget)`
- Drag section edges → per-section `set_start` / `set_end`
- Sections colour-coded with 6-colour rotating palette

**Persistence** — `settings.set_load_sections(&timeline.sections)` on save; `PendingSavedSections` resource applied once sequence loads.

## Multi-Series Mesh Architecture (seaview-5zf)

### Shared MeshCache

**`MeshCache`** (`loader/mesh_cache.rs`) — owns `Handle<Mesh>` keyed by canonical file `PathBuf`. One entry per file, shared across any number of series.

**`CacheEntry`** holds:
- `handles: Vec<Handle<Mesh>>` — one per mesh primitive (GLB may have multiple)
- `loaded: bool` — whether all handles have finished loading
- `refcount: usize` — number of consumers (series/frame-store slots) referencing this path
- `gpu_consumers: usize` — number of series with spawned entities using this mesh
- `estimated_bytes: u64` — for memory budget accounting

**GPU residency refcounting** — `promote_to_gpu` / `demote_from_gpu` on `CacheEntry` flip `asset_usage` flags. A mesh is only demoted from the render world when `gpu_consumers` drops to 0 (safe for shared handles across series).

### Per-Series Entity Stores

**`SeriesFrameStore`** (`loader/series_frame_store.rs`) — maps `SeriesId → SeriesEntityState`.

**`SeriesEntityState`** holds:
- `frame_entities: Vec<Option<Vec<Entity>>>` — spawned entities per frame index
- `displayed_frame: Option<usize>` — currently visible frame for this series
- `prev_windows: Vec<(usize, usize)>` — previous spawn windows for delta computation
- `total_frames: usize`

**`manage_frame_window`** reads/writes the active series' `SeriesEntityState`. `FrameStore.frame_entities` and `FrameStore.displayed_frame` are kept in sync as mirrors for backward compat with UI and spatial-index consumers.

### Union Loading

**`union_frames_to_load()`** — computes the sorted, deduplicated set of frame indices that any series needs loaded. Used by `sync_section_loads` when >1 series exists.

**`sync_section_loads`** unions all series' `timeline.sections` to determine which files `MeshCache` needs. If series A wants `[0..100]` and clone B wants `[80..200]`, MeshCache loads `[0..200]`.

### Data Flow (Multi-Series)

```
SeriesManager.iter() → union_frames_to_load() → sync_section_loads
  → LoadQueue → drain_load_queue → asset_server.load()
  → MeshCache (shared handles) + FrameStore (loading state)

PostUpdate:
  manage_frame_window (per active series):
    → compute_spawn_windows() per series timeline
    → compute_window_delta() against SeriesEntityState.prev_windows
    → spawn/despawn entities in SeriesEntityState
    → promote/demote GPU via MeshCache.gpu_consumers refcount
    → mirror into FrameStore.frame_entities for backward compat
```