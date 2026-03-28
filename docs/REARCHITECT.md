# Seaview Rearchitecture Plan

## Quick Fixes (DONE)
- [x] Remove dangling `src/ui.rs`
- [x] `default-run = "seaview"` already set — `cargo run` works from workspace root

---

## Systems Testability Analysis

**50+ systems analyzed.** The codebase already does well with pure function extraction
(Timeline, LoadSection, compute_spawn_windows, frames_to_load, discovery all have
extensive tests). The gap is that many systems still embed decision logic that should
be in testable structs/functions.

### Summary

| Category | Count | Description |
|----------|-------|-------------|
| GREEN    | 25    | Trivially thin wiring or already well-tested |
| YELLOW   | 20    | Has extractable logic that could be pure functions |
| RED      | 5     | Substantial business logic needing refactoring |

### RED — Needs Refactoring

1. **`update_action_state`** (`app/input.rs:81`)
   - Complex modifier suppression, consumed key tracking, egui focus detection
   - Should be data-driven mapping table with pure transition functions

2. **`sync_series_manager`** (`lib/sequence/mod.rs:890`)
   - Bidirectional series/timeline sync with edge cases
   - State machine transitions embedded in system
   - Extract pure transition logic, test all paths

3. **`sync_materials`** (`app/environment_objects.rs:458`)
   - Material override resolution with multiple branches
   - Original material snapshot management
   - Extract material computation to pure function

4. **`autosave_system`** (`app/autosave.rs:239`)
   - Gathers state from 12+ resources for snapshot
   - Snapshot hashing uses helpers (good), but gathering logic is embedded
   - Builder pattern or snapshot struct would help

5. **`handle_load_requests`** (`lib/sequence/loader/mod.rs`)
   - Multi-series load coordination, section mapping, format detection
   - Some pure helpers exist but orchestration is complex

### YELLOW — Extractable Logic (prioritized)

| System | File | What to Extract |
|--------|------|-----------------|
| `advance_playback` | `lib/sequence/mod.rs:845` | Frame advancement with gap skipping, loop wrapping |
| `camera_controller` | `app/systems/camera.rs:75` | Movement vector calculation |
| `handle_center_on_mesh` | `app/systems/camera.rs:33` | Camera distance/position calculation |
| `apply_material_config` | `app/ui/systems/material_system.rs:12` | MaterialConfig -> StandardMaterial mapping |
| `handle_keyboard_playback` | `lib/sequence/playback.rs:45` | Repeat-delay logic, speed adjustment |
| `update_rendering_stats` | `app/systems/diagnostics.rs:103` | Stats aggregation |
| `detect_performance_issues` | `app/systems/diagnostics.rs:166` | Threshold heuristics |
| `sync_lighting_rig` | `lib/lighting/rig_sync.rs:81` | Entity lifecycle management |
| `spawn_new_objects` | `app/environment_objects.rs:134` | Extension detection, path building |
| `poll_direct_mesh_assets` | `app/environment_objects.rs:244` | Mesh validation, AABB |
| `track_asset_loading` | `lib/sequence/loader/loading_tracker.rs` | Frame-size estimation, degenerate detection |
| `manage_frame_window` | `lib/sequence/loader/frame_window.rs` | Already uses pure helpers, but spawn logic is complex |
| `register_discovered_series` | `lib/sequence/discovery.rs:143` | Settings matching |
| `poll_for_new_frames` | `lib/sequence/watch.rs:148` | Already uses pure scan helper |
| `handle_incremental_load` | `lib/sequence/watch.rs:188` | Frame insertion, store growth |
| `update_loading_phase_on_load_complete` | `lib/sequence/watch.rs:236` | State transitions |
| `toast_events_system` | `app/ui/systems/toast_events.rs:30` | Transition detection |
| `restore_saved_sections` | `main.rs:468` | Section clamping/filtering |
| `handle_input_path` | `main.rs:528` | Path classification |
| `update_timeline_budget` | `lib/sequence/mod.rs:801` | Budget enforcement (uses pure helpers) |

### GREEN — Already Good (25 systems)

Thin wiring, UI-only rendering, trivial event bridges. No action needed.

---

## Refactoring Order

Work through RED items first, then YELLOW. Each section:
1. Plan the extraction
2. Write pure functions/structs with tests
3. Make the system a thin wrapper
4. Commit

### Phase 1: RED systems
- [ ] `sync_series_manager` — extract state machine
- [ ] `update_action_state` — data-driven input mapping
- [ ] `sync_materials` — pure material resolver
- [ ] `autosave_system` — snapshot builder
- [ ] `handle_load_requests` — further decomposition

### Phase 2: YELLOW systems (high-value)
- [ ] `advance_playback` — pure frame stepper
- [ ] `camera_controller` — movement math
- [ ] `handle_keyboard_playback` — repeat logic
- [ ] `track_asset_loading` — degenerate detection
- [ ] `manage_frame_window` — spawn decision logic

### Phase 3: Timeline rearchitecture
- [ ] Multi-series timeline UI
- [ ] Per-series sections (not proportional mapping)
- [ ] Independent playheads

---

## Testing Strategy

**Current state:** 529 unit tests, 0 system integration tests.

Pure helpers have excellent coverage. The gap is in system-level orchestration.

### Approach
1. **Extract first, test the extraction** — don't try to test Bevy systems directly
2. **System tests via Bevy's MinimalPlugins** — for critical multi-system flows
3. **Property-based tests** for Timeline section manipulation
4. **Snapshot tests** for autosave serialization
