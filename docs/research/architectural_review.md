# Architectural Review

Last updated: 2026-02-18

## Rating Legend

- 🟢 **Good** — clean, tested, follows architecture rules
- 🟡 **OK** — works, but has structural issues worth addressing
- 🟠 **Needs review** — accumulated debt, unclear boundaries, or missing tests
- 🔴 **Ancient/grubby** — known mess, blocks progress, or violates architecture

---

## Verdict by File

### `lib/` — Pure Logic Layer

| File | Lines (code) | Tests | Rating | Notes |
|------|-------------|-------|--------|-------|
| `sequence/mod.rs` | 1159 | 94 | 🟢 | Best code in the project. `Timeline`, `LoadSection`, pure functions, 94 tests. Slightly over 500-line guideline but tests account for ~1000 lines. |
| `lighting/rig.rs` | 440 | 47 | 🟢 | Clean pure data model. `Light`, `LightGroup`, `LightingRig`. Well tested. |
| `lighting/presets.rs` | 432 | 11 | 🟢 | Static data + builder helpers. Fine. |
| `lighting/placement.rs` | 258 | 3 | 🟡 | Only 3 tests for placement algorithms. Could use more edge-case coverage. |
| `lighting/environment_map.rs` | 236 | 5 | 🟢 | Small, focused, tested. |
| `lighting/rig_sync.rs` | 225 | 0 | 🟠 | Bevy system code living in `lib/`. Reads `Res<LightingRig>`, spawns entities via `Commands`. **Violates lib/ purity rule.** Should be in `app/`. |
| `lighting/systems.rs` | 47 | 0 | 🟠 | Same — system code in `lib/`. |
| `environment_objects.rs` | 627 | 22 | 🟢 | Clean pure data model. `EnvironmentObject`, `EnvironmentObjectStore`, `AxisSwizzle`, `MaterialOverride`. Good tests. |
| `spatial.rs` | 414 | 24 | 🟢 | `SpatialMesh` wrapper around parry3d. Well-structured, tested. |
| `memory_budget.rs` | 538 | 6 | 🟡 | Trait-based detection (`RamDetector`, `VramDetector`) is good. Only 6 tests — detection logic is hard to unit-test but the budget math should have more. |
| `coordinates.rs` | 164 | 3 | 🟢 | Small, focused. `SourceOrientation` enum. |
| `toast.rs` | 227 | 12 | 🟢 | Clean toast queue. Well tested. |
| `mesh_info.rs` | 219 | 0 | 🟠 | Has Bevy system code (`compute_bounds_on_first_load` queries `With<SequenceMeshDisplay>`). **Violates lib/ purity rule.** The pure AABB math should stay, but systems should move to `app/`. |
| `settings.rs` | 1424 | 51 | 🟠 | Largest file in the project. 51 tests is strong, but 1424 lines of code (excl. tests) is nearly 3× the 500-line guideline. Mixes TOML serde types, `From`/`Into` impls, helper functions, and migration logic. |
| `global_settings.rs` | 240 | 4 | 🟡 | Global `~/.config/seaview/` settings. Small, fine. |
| `session/types.rs` | 219 | 0 | 🟠 | `Session`, `SessionSource`, `FrameStorage` — zero tests. |
| `session/manager.rs` | 222 | 0 | 🟠 | `SessionManager` — zero tests. Entire session system appears unused/dormant. |
| `session/mod.rs` | 177 | 0 | 🟠 | Plugin registration for session system. Zero tests. |
| `network/protocol.rs` | 167 | 0 | 🟠 | `MessageHeader`, `MeshData` — zero tests. |
| `network/receiver.rs` | 233 | 1 | 🟠 | `NonBlockingMeshReceiver` — 1 test. Network subsystem appears dormant. |
| `sequence/loader/assets.rs` | 267 | 10 | 🟢 | `FrameStore`, `LoaderConfig`, `AssetIndex`. Recently refactored. |
| `sequence/loader/frame_window.rs` | 360 | 8 | 🟢 | `manage_frame_window`. Clean delta-based spawn/despawn. |
| `sequence/loader/load_queue.rs` | 305 | 6 | 🟢 | `LoadQueue`, `drain_load_queue`, `sync_section_loads`. |
| `sequence/loader/loading_tracker.rs` | 240 | 2 | 🟡 | Tracks asset loading completion. Only 2 tests. |
| `sequence/loader/spatial_index.rs` | 464 | 16 | 🟢 | Per-frame spatial index management. Good tests. |
| `sequence/loader/mod.rs` | 403 | 0 | 🟡 | Orchestrator — re-exports, plugin registration, constants. No logic to test. |
| `sequence/discovery.rs` | 384 | 4 | 🟡 | Filesystem sequence detection. Only 4 tests for non-trivial glob logic. |
| `sequence/watch.rs` | 467 | 0 | 🟠 | Watch mode. All Bevy systems, zero tests. Has complex polling logic that should be extracted into testable pure functions. |
| `sequence/playback.rs` | 176 | 0 | 🟡 | Keyboard playback controls. Thin system code, low risk. |

### `app/` — Bevy Integration Layer

| File | Lines (code) | Tests | Rating | Notes |
|------|-------------|-------|--------|-------|
| `environment_objects.rs` | 858 | 0 | 🟠 | Largest `app/` file. 7 systems + helpers. `poll_direct_mesh_assets` had the `RenderAssetUsages` bug. `sync_materials` is 90 lines of logic. No tests — should extract material sync logic to `lib/`. |
| `systems/camera.rs` | 279 | 0 | 🟡 | FPS camera controller. Hardcoded constants mixed with logic. No tests but mostly input handling (hard to unit-test). |
| `systems/diagnostics.rs` | 341 | 0 | 🟡 | `RenderingStats` gathering. Fine for `app/` layer. |
| `systems/network/mod.rs` | 165 | 0 | 🟡 | Network mesh polling. Dormant feature. |
| `systems/window.rs` | 29 | 0 | 🟢 | Tiny. Window title system. |
| `spatial_accelerator.rs` | 52 | 0 | 🟢 | Plugin stub behind feature gate. Clean. |
| `ui/state.rs` | 410 | 0 | 🟡 | `UiState`, `PlaybackState`, `LoadingState`, `CollapsibleState`, `MaterialConfig`, `RenderConfig`. Large grab-bag of UI state. No domain logic to test, but `MaterialConfig` arguably belongs in `lib/`. |
| `ui/systems/environment_panel/mod.rs` | 364 | 0 | 🟡 | Panel orchestrator. Draws egui side panel, delegates to sections. |
| `ui/systems/environment_panel/lighting_section.rs` | 772 | 0 | 🟠 | Largest UI file. Individual light editing inline. Should factor out reusable light editor widget. |
| `ui/systems/environment_panel/objects_section.rs` | 576 | 0 | 🟡 | Per-object controls (transform, material, mesh visibility). Long but mostly UI layout. |
| `ui/systems/playback_controls/load_section_bar.rs` | 536 | 0 | 🟡 | Multi-section timeline drawing + drag logic. Complex but recently refactored. |
| `ui/systems/playback_controls/transport.rs` | 284 | 0 | 🟡 | Play/pause/step buttons + speed slider. |
| `ui/systems/playback_controls/timeline_bar.rs` | 133 | 0 | 🟢 | Simple playhead scrubber. |
| `ui/systems/environment_panel/perf_section.rs` | 319 | 0 | 🟡 | Performance stats display. Read-only UI. |
| `ui/systems/environment_panel/spatial_section.rs` | 298 | 0 | 🟡 | BVH gizmo controls. |
| `ui/systems/environment_panel/material_section.rs` | 135 | 0 | 🟢 | Small, focused. |
| `ui/systems/environment_panel/rendering_section.rs` | 92 | 0 | 🟢 | Small, focused. |
| `ui/systems/environment_panel/environment_light_section.rs` | 152 | 0 | 🟢 | Small, focused. |
| `ui/widgets/section_header.rs` | 164 | 0 | 🟢 | Reusable widget. Clean. |
| `ui/systems/toast_overlay.rs` | 163 | 0 | 🟢 | Toast rendering. |
| `ui/systems/toast_events.rs` | 135 | 0 | 🟢 | Toast event bridge. |
| `ui/systems/status_bar.rs` | 193 | 0 | 🟡 | Bottom status bar. |
| `ui/systems/menu_bar.rs` | 176 | 0 | 🟡 | Top menu bar. |
| `ui/systems/material_system.rs` | 79 | 0 | 🟢 | Reactive material application. Small. |
| `ui/systems/event_handlers.rs` | 49 | 0 | 🟢 | Tiny. |

### `bin/` — Standalone Tools

| File | Lines | Tests | Rating | Notes |
|------|-------|-------|--------|-------|
| `debug_loader.rs` | 768 | 0 | 🔴 | Reimplements discovery, parsing, GPU detection, memory budget. Massive duplication of `lib/` logic. Architecture doc already flags this. |
| `timeline_debug.rs` | 695 | 0 | 🔴 | Same — reimplements discovery + parsing. Partially fixed (uses `LoadSection`/`distribute_sections`) but still duplicates mesh parsing + memory estimation. |

### `main.rs`

| File | Lines | Rating | Notes |
|------|-------|--------|-------|
| `main.rs` | 638 | 🟠 | God function. `setup()` is 200+ lines: spawns camera, lights, restores settings, applies materials, env objects. `handle_save_view` is 50+ lines of settings gathering. `handle_input_path` mixes discovery and single-file loading. Should be broken into focused startup systems. |

### `vendor/loadfu/`

| Area | Rating | Notes |
|------|--------|-------|
| Core parsers | 🟢 | STL, OBJ, GLB, PLY parsers are clean, tested. |
| `MeshData` / `AssetCapabilities` | 🟢 | Good foundation. |
| `into_mesh.rs` | 🟡 | `RenderAssetUsages::MAIN_WORLD` only — causes invisible meshes for consumers that don't manually promote. See seaview-znf. |
| `plugin.rs` | 🟡 | Registers `GlbLoader` which shadows Bevy's built-in glTF loader. Works but fragile — env objects rely on Bevy's loader for scene support while sequences use loadfu's. Two loaders for the same extension. |
| Material/texture pipeline | 🔴 | Doesn't exist. `MaterialData` is defined but never flows to Bevy. GLB materials, textures, scene hierarchy all stripped. See seaview-znf. |

---

## Top Structural Issues

### 1. 🔴 `settings.rs` is a 1424-line monolith

**Problem:** Mixes 15+ settings structs, 30+ `From`/`Into` impls, serde helpers, migration logic, and file I/O in one file. Every new feature (env objects, lighting groups, load sections) adds another 100+ lines here.

**Fix:** Split into modules:
- `settings/camera.rs` — `CameraSettings`, `SavedCameras`
- `settings/environment.rs` — `EnvironmentSettings`, `EnvironmentObjectSettings`, `LightingSettings`
- `settings/sequence.rs` — `SequenceSettings`, `PlaybackSettings`, `LoadSectionSettings`
- `settings/material.rs` — `MaterialSettings`
- `settings/persistence.rs` — `Settings` (top-level), `load_from_dir`, `save_to_dir`, migration
- `settings/mod.rs` — re-exports

### 2. 🔴 `main.rs` setup is a god function

**Problem:** `setup()` (200+ lines) handles camera, lights, environment restore, material config, mesh bounds, playback settings, load sections, env objects. `handle_save_view` mirrors this in reverse. Adding features means touching both.

**Fix:** Break into focused startup systems:
- `setup_camera` — camera from settings
- `setup_default_lights` — directional + point lights
- `restore_environment` — env light, lighting rig, materials, env objects
- `restore_playback` — speed, loop, sections
- Parallel: `handle_save_view` becomes a system that iterates sub-savers

### 3. 🔴 `bin/` tools duplicate `lib/` logic

**Problem:** `debug_loader.rs` (768 lines) and `timeline_debug.rs` (695 lines) reimplement discovery, mesh parsing, memory estimation, GPU detection. Drift is guaranteed.

**Fix:** Extract shared logic to `lib/` public API:
- `lib/sequence/discovery.rs` already has `discover_sequences` — ensure it's usable without Bevy runtime
- `lib/memory_budget.rs` — expose `detect()` as a standalone function (no `Res<>`)
- `lib/sequence/loader/` — expose frame size estimation as a pure function

### 4. 🟠 System code in `lib/`

**Problem:** `lib/lighting/rig_sync.rs`, `lib/lighting/systems.rs`, `lib/mesh_info.rs`, and `lib/sequence/watch.rs` contain Bevy system signatures (`Res<>`, `Query<>`, `Commands`). This violates the core architecture rule.

**Files to move to `app/`:**
- `rig_sync.rs` → `app/systems/lighting_sync.rs`
- `lighting/systems.rs` → merge into above
- `mesh_info.rs` systems → `app/systems/mesh_info.rs` (keep AABB math in `lib/`)
- `watch.rs` systems → `app/systems/watch.rs` (keep directory scanning logic in `lib/`)

### 5. 🟠 Environment objects has no test coverage in `app/`

**Problem:** `app/environment_objects.rs` (858 lines) has 7 systems, material sync logic, mesh spawning — zero tests. The `RenderAssetUsages` bug lived here undetected.

**Fix:** Extract testable logic:
- `sync_materials` material resolution logic → `lib/environment_objects.rs`
- `bevy_transform_from` → already pure, add tests
- `poll_direct_mesh_assets` mesh setup → extract mesh preparation to `lib/`

### 6. 🟠 Session + network subsystems are dormant

**Problem:** `lib/session/` (618 lines), `lib/network/` (438 lines), `app/systems/network/` (165 lines) — total ~1200 lines with 1 test. Appears to be scaffolding for a future multi-session + network mesh receiving feature that was never completed.

**Decision needed:** Ship it or strip it. Dead code is maintenance burden.

### 7. 🟡 Dual GLB loader architecture

**Problem:** loadfu registers `GlbLoader` (produces `Handle<Mesh>`), Bevy's `DefaultPlugins` registers `GltfPlugin` (produces `Handle<Gltf>`). Both handle `.glb`/`.gltf`. Sequences use loadfu's loader; environment objects use Bevy's. This works by accident of Bevy's type-directed asset loading, but is fragile and confusing.

**Fix:** seaview-znf (loadfu GLB parity) will unify this.

### 8. 🟡 `UiState` is a grab-bag

**Problem:** `UiState` has 12 fields spanning panel visibility, playback mirrors, loading state, toast queue, collapsible accordion state, drag state. It's the "junk drawer" resource.

**Fix:** Split into focused resources:
- `PanelVisibility` — which panels are open
- `CollapsibleState` — accordion state (or derive from panel interaction)
- Keep `UiState` for truly transient interaction state (dragging, temp dialogs)
- `PlaybackState` and `LoadingState` are already separate structs — could become independent resources

---

## Key Structs & Resources

### Domain Model (`lib/`)

| Struct | File | Role |
|--------|------|------|
| `Timeline` | `sequence/mod.rs` | Playhead, sections, total_frames. All temporal logic. |
| `LoadSection` | `sequence/mod.rs` | `[start, end)` frame range with active flag. |
| `SequenceManager` | `sequence/mod.rs` | Playback controls + loaded `Sequence` metadata. |
| `Sequence` | `sequence/mod.rs` | Frame paths, format, source orientation. |
| `FrameStore` | `sequence/loader/assets.rs` | Frame handles, entities, loading state. Public, read-heavy. |
| `LoaderConfig` | `sequence/loader/assets.rs` | Material handle, coord transform. Set once on first frame. |
| `AssetIndex` | `sequence/loader/assets.rs` | Reverse `AssetId → frame` map. Private to loader. |
| `LoadQueue` | `sequence/loader/load_queue.rs` | Frames waiting to be sent to `AssetServer`. |
| `SpatialIndex` | `sequence/loader/spatial_index.rs` | Per-frame `SpatialMesh` cache. |
| `MemoryBudget` | `memory_budget.rs` | RAM/VRAM detection, frame budget, usage tracking. |
| `LightingRig` | `lighting/rig.rs` | Named groups of spot/point lights. |
| `Light` | `lighting/rig.rs` | Single light with position, intensity, color, cone. |
| `LightGroup` | `lighting/rig.rs` | Named group of lights with placement algorithm. |
| `EnvironmentLightConfig` | `lighting/environment_map.rs` | Ambient + directional + environment map. |
| `EnvironmentObjectStore` | `environment_objects.rs` | Ordered list of imported scene objects. |
| `EnvironmentObject` | `environment_objects.rs` | Single imported object: path, transform, swizzle, material override, mesh entries. |
| `SpatialMesh` | `spatial.rs` | parry3d `TriMesh` wrapper for ray casting + point queries. |
| `Settings` | `settings.rs` | Top-level TOML persistence struct. |
| `GlobalSettings` | `global_settings.rs` | `~/.config/seaview/` cross-project defaults. |
| `SessionManager` | `session/manager.rs` | Multi-session management. **Dormant.** |
| `ToastQueue` | `toast.rs` | Notification queue with levels, durations, fade. |

### UI & App State (`app/`)

| Struct | File | Role |
|--------|------|------|
| `UiState` | `ui/state.rs` | Panel visibility, playback mirror, loading mirror, drag state, toasts. |
| `MaterialConfig` | `ui/state.rs` | PBR material properties for sequence mesh. |
| `RenderConfig` | `ui/state.rs` | Frame window size, wireframe toggle. |
| `RenderingStats` | `systems/diagnostics.rs` | Live vertex/triangle/FPS stats. |
| `SpawnedEnvironmentObjects` | `app/environment_objects.rs` | ECS tracking: root entities, mesh children, discovery state. |
| `FpsCamera` | `systems/camera.rs` | Camera controller state (speed, sensitivity). |
| `WatchConfig` | `sequence/watch.rs` | Directory watch mode config. |
| `LoadingPhase` | `sequence/watch.rs` | State machine: discovery → loading → watching. |

### loadfu (`vendor/loadfu/`)

| Struct | File | Role |
|--------|------|------|
| `MeshData` | `mesh_data.rs` | Format-agnostic mesh: positions, normals, UVs, colors, indices, topology. |
| `AssetData` | `capabilities.rs` | Multi-mesh output: `Vec<MeshData>` + `Vec<MaterialData>`. |
| `AssetCapabilities` | `capabilities.rs` | Pre/post-parse metadata: topology, normals, UVs, cost, memory. |
| `MaterialData` | `capabilities.rs` | Minimal PBR description. **Not yet wired to Bevy.** |
| `StlLoader` / `ObjLoader` / `GlbLoader` / `PlyLoader` | `bevy_integration/asset_loaders.rs` | Bevy `AssetLoader` impls. All produce `Handle<Mesh>`. |

---

## Priority Actions

| # | What | Effort | Impact | Blocked by |
|---|------|--------|--------|------------|
| 1 | Split `settings.rs` into modules | Medium | High — every feature touches this | Nothing |
| 2 | Break `main.rs` setup into focused systems | Medium | High — unblocks parallel dev | Nothing |
| 3 | Move system code out of `lib/` (`rig_sync`, `mesh_info`, `watch`) | Low | Medium — restores architecture invariant | Nothing |
| 4 | Extract testable logic from `app/environment_objects.rs` | Medium | Medium — prevents bugs like RenderAssetUsages | Nothing |
| 5 | Fix loadfu `RenderAssetUsages` default | Low | Medium — removes footgun | Nothing |
| 6 | Decide on session/network subsystem: ship or strip | Low | Low — removes dead code maintenance | Product decision |
| 7 | Deduplicate `bin/` tools against `lib/` | Medium | Low — reduces drift | #3 (pure `lib/` functions) |
| 8 | Factor `lighting_section.rs` (772 lines) | Low | Low — UI readability | Nothing |