# Audit: `perf/visibility-swap-playback` branch

**Base:** `perf-and-logging`  
**Commits reviewed:** `6eb595f..HEAD` (including uncommitted working tree)  
**Date:** 2026-02-13

---

## 1. Summary of Changes

The branch evolved through several iterations to solve GPU alloc/free churn
during sequence playback:

| Commit | Approach | Outcome |
|--------|----------|---------|
| `aa12d91` | Spawn ALL entities, toggle `Visibility` | GPU alloc/free gone, but Bevy CPU systems iterate 388 entities/frame → 10 FPS |
| `808c491` | Ring-buffer: only N entities spawned | CPU overhead solved, but flickering |
| `118c9d1` | Add `ApplyDeferred` + direct visibility mutations | Still flickering |
| WIP (uncommitted) | Move `manage_frame_window` to `PostUpdate`, remove `update_mesh_display`/`handle_frame_changes` | Correct fix for ordering, but introduces new issues (see §4) |

---

## 2. Architecture (Current State)

### Frame Advancement — Three Writers, Unordered

There are **three independent systems** that advance the current frame, all
registered in `Update` with **no mutual ordering**:

| System | Schedule | Writes to | Mechanism |
|--------|----------|-----------|-----------|
| `handle_playback_input` | `Update` (playback plugin) | `sequence_manager.current_frame` directly | Keyboard arrows, space |
| `playback_update_system` | `Update` (UI systems plugin) | `ui_state.playback.current_frame` then `SequenceEvent::FrameChanged` | Auto-play timer |
| `playback_controls_system` | `EguiPrimaryContextPass` | `ui_state.playback.current_frame` then `SequenceEvent::FrameChanged` | UI buttons, slider |

A fourth system, `sync_sequence_manager_with_events`, reads `SequenceEvent::FrameChanged`
and copies into `sequence_manager.current_frame`. It also runs in `Update`, unordered.

A fifth system, `playback_controls_system`, **overwrites** `ui_state.playback.current_frame`
with `sequence_manager.current_frame` every frame (the "sync" block at lines 31–34).

### Frame Display — Single Owner

`manage_frame_window` (now in `PostUpdate`) reads `sequence_manager.current_frame` and
is the sole owner of entity spawn/despawn and `Visibility` toggling.

### Data Flow Diagram

```text
┌─────────────────────────────────┐
│         Update Schedule          │
│                                  │
│  handle_playback_input ──────┐   │
│    (keyboard → sm.current_frame) │
│                              │   │
│  playback_update_system ─────┤   │
│    (timer → ui_state.current_frame)
│    (writes SequenceEvent)    │   │
│                              │   │
│  playback_controls_system ───┤   │
│    (UI → ui_state, writes event) │
│    (ALSO overwrites ui_state     │
│     from sm.current_frame!)  │   │
│                              │   │
│  sync_sequence_manager ──────┘   │
│    (reads events → sm.current_frame)
└──────────────────────────────────┘
                 │
                 ▼
┌──────────────────────────────────┐
│       PostUpdate Schedule         │
│                                   │
│  manage_frame_window              │
│    reads sm.current_frame         │
│    spawn/despawn ring buffer      │
│    set Visibility directly        │
└───────────────────────────────────┘
                 │
                 ▼
┌───────────────────────────────────┐
│         Render Schedule            │
│   (extracts visible entities)      │
└────────────────────────────────────┘
```

---

## 3. What's Working

✅ **Ring-buffer window** — `manage_frame_window` correctly spawns/despawns
entities within a configurable window around the current frame. Only N entities
exist at a time, eliminating the Bevy CPU overhead from `calculate_bounds`,
`reset_view_visibility`, and `par_for_each`.

✅ **UI rendering section** — `RenderConfig` resource with slider in sidebar.
Shows spawned count, displayed frame, and window range.

✅ **Asset loading** — `track_asset_loading` correctly marks frames as loaded
and stores handles without spawning entities. Material and transform are cached.

✅ **Cleanup on reload** — `handle_load_requests` despawns old entities before
loading a new sequence.

✅ **Mesh info** — `compute_bounds_on_first_load` and `handle_recompute_request`
correctly filter for the visible entity only.

✅ **Diagnostics** — `update_rendering_stats` already filters `Visibility::Hidden`.

✅ **Material system** — `apply_material_config` applies to all `SequenceMeshDisplay`
entities. Since they share a material handle, changes propagate to all (including
newly-spawned window entities).

---

## 4. Issues Found

### 🚨 ISSUE 1: `update_mesh_display` and `handle_frame_changes` are dead code (committed) but still registered

In the **committed** state (`118c9d1`), the system registration is:

```rust
(
    (handle_load_requests, track_asset_loading).chain(),
    manage_frame_window,
    ApplyDeferred,
    (update_mesh_display, handle_frame_changes),
)
    .chain()
```

Both `update_mesh_display` and `handle_frame_changes` try to toggle visibility
on the same entities that `manage_frame_window` already handled. They fight:

- `manage_frame_window` sets `displayed_frame = Some(current)` unconditionally
- `update_mesh_display` checks `displayed_frame == Some(current)` → early returns
- `handle_frame_changes` reads `SequenceEvent::FrameChanged` and toggles again

In practice they're mostly no-ops (manage_frame_window already did the work),
but they add system overhead and make the code confusing. They also hold mutable
borrows on `SequenceAssets` and `Query<&mut Visibility>` which prevents Bevy
from parallelizing other systems.

**Status:** Fixed in WIP (removed both systems).

### 🚨 ISSUE 2: Playback speed sync is fragile (WIP change)

The WIP change in `playback_controls_system` does:

```rust
let sm_speed = sequence_manager.playback_fps / 30.0;
if (sm_speed - ui_state.playback.speed).abs() > 0.001 {
    ui_state.playback.speed = sm_speed;
} else {
    sequence_manager.playback_fps = ui_state.playback.speed * 30.0;
}
```

**Problems:**

1. **Race on first frame:** `sequence_manager.playback_fps` defaults to `30.0`,
   and `ui_state.playback.speed` defaults to `1.0`. So `sm_speed = 30/30 = 1.0`
   which matches `ui_state.playback.speed`. The `else` branch fires and sets
   `sequence_manager.playback_fps = 1.0 * 30.0 = 30.0`. This happens to be
   correct by coincidence, but only because the defaults align.

2. **Hysteresis:** If the user moves the slider from 1.0 to 1.001 (barely),
   the 0.001 threshold might swallow it. The slider is continuous so this is
   unlikely but fragile.

3. **Two sources of truth:** The root problem is that `sequence_manager.playback_fps`
   (in FPS) and `ui_state.playback.speed` (a multiplier) represent the same
   concept in different units. One should be authoritative and the other derived.

**Recommendation:** Make `ui_state.playback.speed` the single source of truth.
Remove `sequence_manager.playback_fps` from the speed control path entirely,
or have keyboard `+/-` write directly to `ui_state.playback.speed` instead of
`sequence_manager.playback_fps`.

### 🚨 ISSUE 3: `playback_controls_system` overwrites `ui_state.current_frame` every frame

Lines 31–34 of `playback_controls_system`:

```rust
ui_state.playback.current_frame = sequence_manager.current_frame;
```

This runs in `EguiPrimaryContextPass`, which executes during `Update`. If
`playback_update_system` advances `ui_state.playback.current_frame` and writes
a `SequenceEvent::FrameChanged` in the same tick, but `sync_sequence_manager_with_events`
hasn't run yet, then `sequence_manager.current_frame` still holds the OLD value.
The overwrite reverts the advancement.

**Impact:** Auto-playback can stall or stutter depending on system ordering.
This is a pre-existing bug but the branch makes it more visible because
`manage_frame_window` now runs in `PostUpdate` and reads the potentially-stale
`sequence_manager.current_frame`.

**Recommendation:** `playback_controls_system` should NOT overwrite
`ui_state.playback.current_frame` from `sequence_manager`. Instead, it should
only READ from `ui_state` for display. The frame advancement should flow
one way: `playback_update_system` → `SequenceEvent` → `sync_sequence_manager`
→ `manage_frame_window`. The UI sync should go the other direction for display
only (sequence_manager → ui_state) in a dedicated system that runs AFTER
sync_sequence_manager.

### ⚠️ ISSUE 4: `manage_frame_window` despawn loop is O(total_frames) every frame

```rust
for idx in 0..sequence_assets.total_frames {
    if idx >= win_lo && idx < win_hi { continue; }
    if let Some(entity) = sequence_assets.frame_entities[idx].take() { ... }
}
```

With 388 frames this iterates 388 entries every tick, most of which are `None`.
Not a performance problem at 388, but it's needlessly O(N).

**Recommendation:** Track which indices are currently spawned in a small
`HashSet<usize>` or `BTreeSet<usize>` and only iterate that. Or track the
previous window `(prev_lo, prev_hi)` and only despawn the delta.

### ⚠️ ISSUE 5: `SequenceEvent::FrameChanged` is consumed but never read

With `handle_frame_changes` removed (WIP), nobody reads `SequenceEvent::FrameChanged`
except `sync_sequence_manager_with_events`. The events still serve a purpose
(syncing `sequence_manager`), but the name suggests they should trigger display
changes. This is fine functionally but may confuse future readers.

### ⚠️ ISSUE 6: `handle_playback_input` made `pub(crate)` unnecessarily

The WIP undoes the `.after(handle_playback_input)` ordering (by moving to
`PostUpdate`), but `handle_playback_input` and `KeyHoldTimer` are still
`pub(crate)`. These should be reverted to `fn` / `struct` (private) since
nothing references them externally any more.

### ℹ️ ISSUE 7: `SequenceEvent` import removed but events still used

The WIP removes `use super::SequenceEvent` from `loader.rs`, which is correct
since `handle_frame_changes` was removed. But the import of `SequenceEvent`
is still needed elsewhere — just confirming it's not broken (it isn't, the
import only existed in `loader.rs`).

---

## 5. Remaining Flickering Root Cause

The flickering is caused by **ISSUE 3** above. The frame advancement round-trip is:

```text
Tick N:
  playback_update_system: ui_state.current_frame = 10, writes FrameChanged(10)
  playback_controls_system: ui_state.current_frame = sequence_manager.current_frame (still 9!) → REVERTS TO 9
  sync_sequence_manager: reads FrameChanged(10), sets sequence_manager.current_frame = 10

Tick N+1:
  playback_update_system: ui_state.current_frame is 9 (was reverted!), advances to 10 again
  playback_controls_system: ui_state.current_frame = sequence_manager.current_frame (now 10)
  sync_sequence_manager: reads FrameChanged(10), no-op (already 10)

PostUpdate Tick N:
  manage_frame_window: sees current_frame = 10 (sync ran), shows frame 10

PostUpdate Tick N+1:
  manage_frame_window: sees current_frame = 10 (no advancement because tick N+1
  re-advanced to 10 instead of 11)
```

Net effect: **frames advance at half speed** or irregularly, and if the ordering
varies between ticks, the displayed frame oscillates → **flicker**.

---

## 6. Recommended Fix

### Step 1: Unify frame advancement (fixes flickering AND speed)

Make `sequence_manager.current_frame` the single source of truth. All frame
advancement writes directly to it:

```rust
// In playback_update_system, change:
//   ui_state.next_frame();
// To:
    sequence_manager.next_frame();
    sequence_events.write(SequenceEvent::FrameChanged(sequence_manager.current_frame));
```

And in `playback_controls_system`, the UI reads FROM `sequence_manager` for
display but never writes back:

```rust
// Display sync (read-only):
ui_state.playback.current_frame = sequence_manager.current_frame;
ui_state.playback.is_playing = sequence_manager.is_playing;
```

For UI button clicks (prev/next/slider), write directly to `sequence_manager`:

```rust
if ui.button("⏮").clicked() {
    sequence_manager.previous_frame();
    sequence_events.write(SequenceEvent::FrameChanged(sequence_manager.current_frame));
}
```

### Step 2: Unify playback speed

Make `ui_state.playback.speed` authoritative. `playback_update_system` already
reads it. Have keyboard `+/-` in `handle_playback_input` write to a new
`PlaybackSpeedChanged` event or directly to a shared resource, instead of
`sequence_manager.playback_fps`. Or keep `sequence_manager.playback_fps` but
always derive it: `sequence_manager.playback_fps = ui_state.playback.speed * 30.0`.

### Step 3: Clean up dead systems

Remove `update_mesh_display` and `handle_frame_changes` from the codebase entirely
(currently removed from registration but code still exists in committed state).

### Step 4: Revert unnecessary pub(crate)

Revert `handle_playback_input` and `KeyHoldTimer` to private since the
`.after()` ordering is no longer used.

### Step 5: Optimize despawn loop (optional)

Track previous window bounds and only iterate the delta.

---

## 7. System Schedule (Recommended Final State)

```text
Update:
  handle_playback_input          — keyboard → sequence_manager.current_frame
  playback_update_system         — timer → sequence_manager.current_frame
  sync_sequence_manager_with_events — SequenceEvent → sequence_manager (for UI events)
  handle_load_requests → track_asset_loading (chained)

EguiPrimaryContextPass:
  playback_controls_system       — reads sequence_manager, writes SequenceEvent for UI clicks
  session_panel_system           — reads RenderConfig, SequenceAssets

PostUpdate:
  manage_frame_window            — reads sequence_manager.current_frame (finalized)
                                    sole owner of spawn/despawn/visibility
```

This guarantees `manage_frame_window` always sees the final frame index for
the tick, and no system fights over visibility.

---

## 8. Files Changed (Full Inventory)

| File | Change | Status |
|------|--------|--------|
| `loader.rs` | Ring-buffer window, removed handle-swap, removed redundant systems | Committed + WIP |
| `playback.rs` | `pub(crate)` on `handle_playback_input` and `KeyHoldTimer` | WIP (should revert) |
| `playback_controls.rs` | Speed sync block added | WIP |
| `state.rs` | `RenderConfig` resource, `rendering_open` collapsible | Committed |
| `session_panel.rs` | Rendering section with slider | Committed |
| `mesh_info.rs` | Query filters for visible entity | Committed |
| `main.rs` | Removed `LogDiagnosticsPlugin` | Committed |
| `docs/plan-*.md` | Plan document | Committed |
| `docs/impl-*.md` | Implementation summary (partially stale) | Committed |
| `docs/profiling-guide.md` | Profiling instructions | Committed |
| `scripts/analyze_trace.py` | Trace analysis tool | Committed |
| `.gitignore` | Exclude trace JSON files | Committed |