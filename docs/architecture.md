# Seaview Architecture

Seaview is a Bevy-based 3D mesh viewer for time-series simulation data. It loads sequences of mesh files (STL, glTF/GLB) and plays them back as animations with memory-budget-aware frame management.

~13,400 lines of Rust · 101 tests · Two debug binaries

## Layer Boundaries

```
src/
├── main.rs                 # Bevy app setup, CLI handling, asset source registration
├── lib.rs                  # Public API re-exports
├── lib/                    # Core library — pure logic, testable without Bevy
│   ├── sequence/           # Timeline, LoadSection, discovery, loading, playback
│   ├── memory_budget.rs    # VRAM/RAM detection, budget computation
│   ├── settings.rs         # Per-directory seaview.toml persistence
│   ├── global_settings.rs  # ~/.config/seaview/ global config
│   ├── lighting/           # Lighting config, placement algorithms
│   ├── coordinates.rs      # Source coordinate system transforms
│   ├── mesh_info.rs        # Mesh bounds tracking
│   ├── asset_loaders/      # Custom Bevy asset loaders (STL)
│   └── session/            # Session management
├── app/                    # Bevy integration — thin system wrappers + UI
│   ├── systems/            # Camera, window, diagnostics
│   ├── ui/
│   │   ├── state.rs        # UiState + RenderConfig + MaterialConfig
│   │   ├── systems/        # egui panels: menu, environment, playback
│   │   └── widgets/        # (future) reusable UI components
│   └── cli/                # CLI argument parsing
└── bin/                    # Standalone debug tools (no Bevy runtime)
    ├── debug_loader.rs     # Headless sequence analysis
    └── timeline_debug.rs   # Multi-window timeline visualisation
```

### The Rule

| Layer | Allowed | Forbidden |
|-------|---------|-----------|
| `lib/` | Pure types, plain methods, `derive(Resource)` | System signatures, `Res<>`, `Query<>`, `Commands` |
| `app/` | Bevy systems that read resources → call `lib/` → write back | Business logic beyond glue |
| `bin/` | Import from crate public API (`seaview::`) | Import from `app/`, depend on Bevy runtime |

Structs in `lib/` may derive Bevy traits (`Resource`, `Component`) for convenience, but their **methods must take plain arguments and return plain results**.

## Resource Ownership

One authoritative resource per domain. `UiState` fields are read-only mirrors, synced once per tick.

| Domain | Authority | Key fields | Mirror |
|--------|-----------|------------|--------|
| Timeline | `Timeline` | `total_frames`, `sections: Vec<LoadSection>`, `playhead` | `UiState.playback.{current_frame, total_frames}` |
| Playback | `SequenceManager` | `is_playing`, `speed`, `loop_enabled`, `Option<Sequence>` | `UiState.playback.{is_playing, speed, loop_enabled}` |
| Asset loading | `SequenceAssets` | `frame_handles`, `loaded_frames`, `frame_entities`, `loading` | `UiState.loading` |
| Memory budget | `MemoryBudget` | `total_vram`, `max_frames`, `frame_size_estimate` | — |
| Appearance | `MaterialConfig` + `NightLightingConfig` | colors, roughness, light positions | — |
| Render window | `RenderConfig` | `frame_window_size` | — |
| Persistence | `SettingsResource` | `Settings` + directory path | — |
| UI interaction | `UiState` | `load_section_dragging`, `collapsible`, `temp_state`, panel visibility | — |

## Data Flow

```
CLI path
  → handle_input_path()
    ├─ Directory → DiscoverSequenceRequest (component)
    │                → discovery system scans dir, matches numbered patterns
    │                → SequenceManager.load_sequence() + LoadSequenceRequest
    └─ Single file → SequenceManager.load_sequence() + LoadSequenceRequest

LoadSequenceRequest
  → handle_load_requests()
    → AssetServer loads all frames (async)
    → Timeline.init_sequence(total, max_budget)
    → SequenceAssets tracks handles + loaded state

AssetEvent<Mesh>
  → track_asset_loading()
    → marks frames loaded, estimates GPU bytes, creates shared material

Every tick (Update):
  → advance_playback(): SequenceManager.is_playing → Timeline.playhead++
  → update_timeline_budget(): MemoryBudget → Timeline.enforce_budget()
  → sync_ui_from_sequence_manager(): Timeline + SequenceManager → UiState

Every tick (PostUpdate):
  → manage_frame_window(): compute_spawn_window() → spawn/despawn entities

Watch mode (optional):
  → poll_for_new_frames() → IncrementalLoadRequest → extends SequenceAssets
```

## Plugin Registration Order

```rust
app.add_plugins(DefaultPlugins)        // Bevy core + bevy_gltf
   .add_plugins(AssetLoadersPlugin)    // Registers StlLoader for .stl
   .add_plugins(SequencePlugin)        // Discovery + loader + playback + timeline
   .add_plugins(MemoryBudgetPlugin)    // RAM/VRAM detection
   .add_plugins(SessionPlugin)
   .add_plugins(SeaviewUiPlugin)       // egui panels + UiState + MaterialConfig
   .add_plugins(NightLightingPlugin)
   .add_plugins(MeshInfoPlugin)
```

## Key Types

### Timeline (lib/sequence/mod.rs)

Pure temporal model. Owns the frame range, load sections, and playhead.

```rust
pub struct Timeline {
    pub total_frames: usize,
    pub sections: Vec<LoadSection>,
    pub playhead: usize,
}
```

Methods: `next_frame()`, `previous_frame()`, `jump_to_frame()`, `enforce_budget()`, `init_sequence()`, `section()`, `section_mut()`, `section_containing()`, `total_loaded()`.

### LoadSection (lib/sequence/mod.rs)

Pure `[start, end)` range. No UI state.

Methods: `frame_count()`, `contains()`, `clamp()`, `set_start()`, `set_end()`, `center_on()`.

### Pure Functions (lib/sequence/mod.rs)

- `compute_spawn_window(playhead, window_size, section_start, section_end, total_frames) → (lo, hi)` — entity window range
- `distribute_sections(total_frames, max_frames, num_sections) → Vec<LoadSection>` — multi-section layout

### MemoryBudget (lib/memory_budget.rs)

Trait-based GPU/RAM detection with pluggable backends.

- `VramDetector` trait → `NvmlBackend` (NVIDIA), future: AMD, Intel
- `RamDetector` trait → `SysInfoBackend`
- `MemoryBudget::detect()` probes hardware, computes `max_frames`

### Settings (lib/settings.rs)

TOML-serializable config. Supports single `[load_section]` (legacy) and `[[load_sections]]` (multi-section). `Settings::get_load_sections()` / `set_load_sections()` handle conversion.

## Standards

### 1. Pure Function Extraction

If a Bevy system has logic worth testing (>20 lines), extract it:

```rust
// In lib/ — pure, testable
pub fn enforce_budget(sections: &mut [LoadSection], total_frames: usize, max: usize) { ... }

// In app/ — thin wrapper
fn update_timeline_budget(mut timeline: ResMut<Timeline>, budget: Res<MemoryBudget>) {
    timeline.enforce_budget(budget.max_frames);
}
```

### 2. No UI State in Domain Structs

Interaction state (dragging, hover, selection) lives in `UiState`, not in the domain type it operates on. Example: `LoadSectionEdge` (drag handle) is in `app/ui/state.rs`, not in `LoadSection`.

### 3. Settings ↔ Runtime Conversions

Use `From`/`Into` implementations between TOML-serializable settings types and runtime types. Single direction: `LoadSectionSettings → LoadSection` and back.

### 4. File Size Signals

| Type | Threshold | Action |
|------|-----------|--------|
| System function | >200 lines | Split into sub-systems or extract logic |
| Module (excl. tests) | >500 lines | Factor into submodules |
| Module (incl. tests) | >1000 lines | Strong signal to split |

### 5. Debug Binary Rule

Binaries import library types via the crate's public API (`seaview::LoadSection`, `seaview::distribute_sections`). If a binary needs something entangled with Bevy, extract it into `lib/` first.

## Known Tech Debt

### SequenceAssets God Object

`SequenceAssets` holds 7 responsibilities in one struct: asset tracking, entity management, sequence metadata, memory estimation, material cache, coord transform cache, and window computation. Every system that needs any one field takes `ResMut<SequenceAssets>`, blocking Bevy's parallel scheduling.

**When to fix:** When seaview-wme (multi-section) forces changes to `manage_frame_window`. Not before — no planned feature is blocked by this today.

**Decomposition plan:**
- `FrameAssets` — `frame_handles`, `loaded_frames`, `loaded_count`, `loading`, `format`
- `FrameEntities` — `frame_entities`, `displayed_frame` (or move entity tracking to ECS components)
- Move `total_frames` authority fully to `Timeline` (already partially done)
- Move `estimated_frame_bytes` to `MemoryBudget`
- Move `coord_transform` to `Sequence` or a dedicated resource
- Move `shared_material` to `MaterialConfig` or a loader-local resource

### Debug Binary Duplication

Both `debug_loader.rs` (~1000 lines) and `timeline_debug.rs` (~890 lines) duplicate discovery, mesh parsing, and GPU detection from `lib/`. `timeline_debug` was partially fixed (now uses `LoadSection` and `distribute_sections`).

**When to fix:** When discovery is extracted to a pure function (no Bevy types in signature). Filed separately.

**Decomposition plan:**
- Extract `discover_sequences()` in `lib/sequence/discovery.rs` into a pure function returning plain structs
- Extract mesh stats parsing into `lib/` (vertex count, GPU size estimation)
- Binaries call library functions instead of reimplementing

### manage_frame_window Monolith

140-line system mixing window computation (extracted to `compute_spawn_window`), entity spawn/despawn, visibility management, and memory accounting.

**When to fix:** When seaview-wme needs multi-section spawn logic.

**Decomposition plan:**
- `compute_spawn_window()` — already extracted ✓
- `compute_evictions(old_window, new_window) → (Vec<usize>, Vec<usize>)` — pure function
- `spawn_frame()` / `despawn_frame()` — helper functions
- System becomes: compute window → compute evictions → despawn → spawn → set visibility

### playback_controls.rs (575 lines)

One giant egui system function rendering the entire bottom panel: loading progress, playback buttons, speed slider, timeline scrubber, load section bar, camera presets.

**When to fix:** When any section needs significant rework (e.g. seaview-wme multi-section UI).

**Decomposition plan:** Split into sub-functions per section (progress bar, transport controls, timeline scrubber, load section bar, camera picker).

### RenderConfig Placement

`RenderConfig` (frame window size) lives in `app/ui/state.rs` but is not UI state — it's a rendering concern read by `manage_frame_window`.

**When to fix:** Low priority. Move to `lib/sequence/` or a dedicated rendering config when state.rs is next refactored.

## Feature Map

Where each planned feature touches the codebase.

| Issue | Layer | Files touched | Churn risk |
|-------|-------|---------------|------------|
| **seaview-blz** (light controls) | `app/` + `lib/` | `lighting_section.rs`, `NightLightingConfig`, `settings.rs` | Low — extends existing pattern |
| **seaview-4sg** (toasts) | `app/` | New `UiState` fields, new UI system | Low — additive |
| **seaview-9wx** (multi-GPU) | `lib/` | `memory_budget.rs` (new backends), `debug_loader.rs` | Low — trait extension |
| **seaview-nvh** (HUD overlays) | `app/` | New UI system files, reads existing resources | Low — additive |
| **seaview-wme** (multi-section) | All | `playback_controls.rs`, `manage_frame_window`, `Timeline` | Medium — triggers SequenceAssets decomposition |