# FrameStore Architecture: Current → Projected

## Overview

This document describes the current single-series FrameStore architecture and the
projected multi-series architecture needed for simultaneous rendering of cloned/multiple
mesh series (seaview-5zf).

---

## Current Architecture

One global `FrameStore` resource owns everything: mesh handles, spawned entities,
loading state, and visibility. Only one series is rendered at a time.

```mermaid
graph TD
    subgraph "SeriesManager (lib/)"
        SM[SeriesManager]
        SA[SeriesState A<br/>timeline, transform, label]
        SB[SeriesState B <i>clone</i><br/>timeline, transform, label]
        SM --> SA
        SM --> SB
    end

    subgraph "Global Resources (app/)"
        TL[Timeline<br/>playhead, sections]
        SEQ[SequenceManager<br/>current_sequence]
        FS[FrameStore<br/>frame_handles: Vec‹Handle‹Mesh››<br/>frame_entities: Vec‹Vec‹Entity››<br/>frame_paths: Vec‹PathBuf›<br/>loaded_frames: Vec‹bool›<br/>displayed_frame: Option‹usize›]
        LC[LoaderConfig<br/>shared_material<br/>coord_transform]
    end

    subgraph "Systems"
        SYNC[sync_series_manager<br/><i>active switch → reload</i>]
        FW[manage_frame_window<br/><i>spawn/despawn/visibility</i>]
        LQ[drain_load_queue<br/><i>asset loading</i>]
        LT[track_asset_loading<br/><i>completion tracking</i>]
        ST[sync_series_transform<br/><i>bulk transform update</i>]
    end

    subgraph "ECS World"
        E1[Entity: Frame 0<br/>Mesh3d + Transform + Visibility]
        E2[Entity: Frame 1<br/>Mesh3d + Transform + Visibility]
        E3[Entity: Frame 2<br/>...]
    end

    SM -->|active series| SYNC
    SYNC -->|push timeline| TL
    SYNC -->|LoadSequenceRequest| LQ
    LQ -->|load assets| FS
    LT -->|mark loaded| FS
    TL --> FW
    FS --> FW
    LC --> FW
    FW -->|spawn/despawn| E1
    FW -->|spawn/despawn| E2
    FW -->|spawn/despawn| E3
    ST -->|update transforms| E1
    ST -->|update transforms| E2
    ST -->|update transforms| E3

    style FS fill:#c44,stroke:#800,color:#fff
    style SA fill:#48a,stroke:#269,color:#fff
    style SB fill:#48a,stroke:#269,color:#fff
```

### Problems

1. **Switching series = full reload.** Even between clone and original (same files),
   `handle_load_requests` calls `frame_store.reset()` and re-queues every frame.

2. **Only one series visible.** `FrameStore.frame_entities` is a single flat vec —
   there's no concept of "these entities belong to series A, those to series B."

3. **No handle sharing.** Two series pointing at the same `.glb` files load them
   independently. Same bytes, two `Handle<Mesh>` instances.

4. **Load sections are per-series, loading is global.** Series A wants frames [0-100],
   clone B wants [80-200], but only the active series' sections drive loading.

### Data Flow (Series Switch)

```mermaid
sequenceDiagram
    participant UI as User clicks "Set Active"
    participant SM as SeriesManager
    participant SYNC as sync_series_manager
    participant FS as FrameStore
    participant LQ as LoadQueue
    participant FW as frame_window

    UI->>SM: set_active(clone_id)
    SYNC->>SYNC: detect active changed
    SYNC->>FS: LoadSequenceRequest (same paths!)
    FS->>FS: reset() — despawn ALL entities
    FS->>FS: re-allocate vecs for N frames
    LQ->>LQ: re-queue all frames for loading
    Note over LQ,FS: Full reload even though<br/>files are identical
    LQ->>FS: load frame assets (again)
    FW->>FW: spawn entities with new transform
```

---

## Projected Architecture

Split `FrameStore` into two layers:

- **MeshCache** — shared, path-keyed, owns `Handle<Mesh>`. Load once.
- **SeriesFrameStore** — per-series, owns spawned `Entity` lists. Spawn per transform.

```mermaid
graph TD
    subgraph "SeriesManager (lib/)"
        SM[SeriesManager]
        SA[SeriesState A<br/>timeline, transform]
        SB[SeriesState B <i>clone</i><br/>timeline, transform]
        SM --> SA
        SM --> SB
    end

    subgraph "Shared Resources (app/)"
        MC[MeshCache<br/>path → Handle‹Mesh›<br/>refcount per path<br/>loaded_set: HashSet‹PathBuf›]
        LC[LoaderConfig<br/>shared_material<br/>coord_transform]
        AI[AssetIndex<br/>AssetId → path reverse map]
    end

    subgraph "Per-Series Resources"
        FSA[SeriesFrameStore A<br/>frame_entities: Vec‹Vec‹Entity››<br/>displayed_frame<br/>spawn_windows]
        FSB[SeriesFrameStore B<br/>frame_entities: Vec‹Vec‹Entity››<br/>displayed_frame<br/>spawn_windows]
    end

    subgraph "Systems"
        UL[union_load_sections<br/><i>merge all series sections</i>]
        LQ[drain_load_queue<br/><i>load into MeshCache</i>]
        LT[track_asset_loading<br/><i>mark paths loaded in cache</i>]
        FWA[frame_window A<br/><i>spawn with transform A</i>]
        FWB[frame_window B<br/><i>spawn with transform B</i>]
    end

    subgraph "ECS World"
        EA1[Entity: A Frame 0<br/>Transform A]
        EA2[Entity: A Frame 1<br/>Transform A]
        EB1[Entity: B Frame 0<br/>Transform B]
        EB2[Entity: B Frame 1<br/>Transform B]
    end

    SA --> UL
    SB --> UL
    UL -->|union of all sections| LQ
    LQ -->|load by path| MC
    LT -->|mark loaded| MC

    MC -->|Handle‹Mesh›| FWA
    MC -->|Handle‹Mesh›| FWB
    LC --> FWA
    LC --> FWB
    SA --> FWA
    SB --> FWB
    FWA -->|spawn| EA1
    FWA -->|spawn| EA2
    FWB -->|spawn| EB1
    FWB -->|spawn| EB2

    style MC fill:#2a7,stroke:#185,color:#fff
    style FSA fill:#48a,stroke:#269,color:#fff
    style FSB fill:#48a,stroke:#269,color:#fff
```

### Key Changes

| Aspect | Current | Projected |
|--------|---------|-----------|
| Mesh ownership | `FrameStore.frame_handles` (global, flat vec) | `MeshCache` (path-keyed HashMap) |
| Entity ownership | `FrameStore.frame_entities` (global, flat vec) | `SeriesFrameStore.frame_entities` (per-series) |
| Loading trigger | `LoadSequenceRequest` per series switch | `EnsureCached { paths }` — additive, no reset |
| Handle sharing | None — reload same file per switch | Shared by path — load once, clone handle |
| Visible series | One at a time | All simultaneously |
| Transform application | Global compose at spawn | Per-series compose at spawn |
| Load sections | Active series only | Union across all series |
| Series switch cost | Full reload (seconds) | Instant (spawn entities from cached handles) |

### Data Flow (Clone Visibility)

```mermaid
sequenceDiagram
    participant User
    participant SM as SeriesManager
    participant UL as union_load_sections
    participant MC as MeshCache
    participant FWA as frame_window (A)
    participant FWB as frame_window (B)
    participant World as ECS World

    User->>SM: clone_series(A) → B
    Note over SM: B has same paths,<br/>different transform

    UL->>UL: union A.sections ∪ B.sections
    UL->>MC: EnsureCached([0..200])
    MC->>MC: frames 0-100 already loaded ✓
    MC->>MC: load frames 101-200 (new)

    par Series A spawns
        FWA->>MC: get_handle(frame_42.glb)
        MC-->>FWA: Handle<Mesh> (shared)
        FWA->>World: spawn Entity(Mesh3d, Transform_A, Visible)
    and Series B spawns
        FWB->>MC: get_handle(frame_42.glb)
        MC-->>FWB: Handle<Mesh> (same handle!)
        FWB->>World: spawn Entity(Mesh3d, Transform_B, Visible)
    end

    Note over World: Both visible simultaneously,<br/>same mesh data, different transforms
```

### Struct Sketches

```rust
/// Shared mesh handle cache — one Handle<Mesh> per file path.
/// Multiple series can reference the same handle.
#[derive(Resource)]
pub struct MeshCache {
    /// Canonical path → loaded mesh handle(s).
    /// Multi-mesh files (GLB) may have multiple handles per path.
    entries: HashMap<PathBuf, CacheEntry>,
}

struct CacheEntry {
    handles: Vec<Handle<Mesh>>,
    loaded: bool,
    /// Number of SeriesFrameStores referencing this path.
    refcount: usize,
    /// Estimated GPU bytes (for budget accounting).
    estimated_bytes: u64,
}

/// Per-series entity and visibility state.
/// Each series that should be renderable gets one of these.
#[derive(Debug)]
pub struct SeriesFrameStore {
    pub series_id: SeriesId,
    /// Spawned entities per frame index (None = not spawned).
    pub frame_entities: Vec<Option<Vec<Entity>>>,
    /// Currently displayed frame for this series.
    pub displayed_frame: Option<usize>,
    /// Frame paths (shared with MeshCache for lookups).
    pub frame_paths: Vec<PathBuf>,
    /// Total frames in this series.
    pub total_frames: usize,
}

/// Bevy resource holding all per-series frame stores.
#[derive(Resource, Default)]
pub struct SeriesFrameStores {
    stores: HashMap<SeriesId, SeriesFrameStore>,
}
```

---

## Migration Plan

### Phase 1: Introduce MeshCache (no behaviour change)

- Add `MeshCache` resource alongside existing `FrameStore`
- `drain_load_queue` populates both `FrameStore` (legacy) and `MeshCache`
- `track_asset_loading` marks loaded in both
- No per-series stores yet — `frame_window` still reads `FrameStore`
- **Result:** MeshCache exists and is populated, but not consumed. Safe, no breakage.

### Phase 2: Per-series entity stores

- Add `SeriesFrameStores` resource
- `manage_frame_window` iterates all series, spawns per-series entities from `MeshCache`
- Each series gets its own spawn window, transform, visibility
- Series switch = just adjust which timeline the UI controls (no reload)
- Remove entity fields from `FrameStore` (MeshCache owns handles, SeriesFrameStore owns entities)
- **Result:** Clones render simultaneously. Switching is instant.

### Phase 3: Union loading + budget

- `union_load_sections` system collects sections from all series, produces unified load set
- `drain_load_queue` loads the union into `MeshCache`
- Memory budget tracks `MeshCache` (shared) + per-series entity overhead
- Eviction: when a path's refcount drops to 0, it's eligible for cache eviction
- **Result:** Budget-aware multi-series rendering.

---

## Files Involved

| File | Current Role | Phase 1 | Phase 2 | Phase 3 |
|------|-------------|---------|---------|---------|
| `loader/assets.rs` | `FrameStore`, `LoaderConfig`, `AssetIndex` | + `MeshCache` | - entity fields from FrameStore | - |
| `loader/frame_window.rs` | Global spawn/despawn/visibility | unchanged | Per-series spawn loop | unchanged |
| `loader/load_queue.rs` | Queue frames → asset server | + populate MeshCache | reads union sections | union_load_sections |
| `loader/loading_tracker.rs` | Track asset completion | + mark MeshCache | unchanged | unchanged |
| `loader/mod.rs` | `handle_load_requests` | + init MeshCache | `EnsureCached` replaces full reset | budget integration |
| `sequence/mod.rs` | `sync_series_manager` | unchanged | no reload on switch | unchanged |
| `series.rs` | `SeriesManager`, `SeriesState` | unchanged | unchanged | unchanged |

---

## Related Issues

- **seaview-5zf** — This work (P1)
- **seaview-d33** — Clone series ✅ (data model done, rendering blocked on 5zf)
- **seaview-w5t** — Memory budget UI (Phase 3)
- **seaview-6pd** — Multi-series architecture ✅ (foundation)