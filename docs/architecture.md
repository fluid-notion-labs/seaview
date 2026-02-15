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
| `lib/sequence/mod.rs` | `Timeline`, `LoadSection`, `compute_spawn_window()`, `compute_spawn_windows()`, `distribute_sections()` |
| `lib/sequence/loader.rs` | `SequenceAssets`, `manage_frame_window` (multi-section), asset loading systems |
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
| Asset loading | `SequenceAssets` | `UiState.loading` |
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

## Standards

**Pure function extraction.** >20 lines of logic in a system → extract to testable `lib/` function. System becomes read → call → write.

**No UI state in domain structs.** Interaction state (drag, hover) lives in `UiState`.

**Settings ↔ Runtime.** `From`/`Into` impls between TOML types and runtime types.

**File size signals.** System >200 lines → split. Module >500 lines (excl. tests) → factor out.

**Debug binaries.** Reuse `lib/` types. If something is Bevy-entangled, extract it first.

## Current Debt

**`SequenceAssets` (loader.rs)** — holds 7 concerns: asset handles, entity tracking, metadata, memory estimation, material cache, coord transform, window computation. Every consumer takes `ResMut<SequenceAssets>` blocking parallel scheduling. **Trigger:** next major loader refactor.

**Debug binary duplication** — `debug_loader.rs` and `timeline_debug.rs` reimplement discovery, mesh parsing, GPU detection. `timeline_debug` partially fixed (uses library `LoadSection`/`distribute_sections`). **Trigger:** when discovery is extracted to a pure function.

### Resolved Debt

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