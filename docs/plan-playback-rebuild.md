# Plan: Playback System Rebuild

**Branch:** `refactor/playback-rebuild`  
**Base:** `perf/visibility-swap-playback`  
**Date:** 2026-02-13  
**Related:** `docs/audit-visibility-swap-playback.md`

---

## Problem

The playback and frame-display systems have three independent writers to frame
state (`handle_playback_input`, `playback_update_system`,
`playback_controls_system`) with no ordering guarantees. A fourth system
(`sync_sequence_manager_with_events`) tries to reconcile them via events, while
`playback_controls_system` overwrites `ui_state.current_frame` from
`sequence_manager` every frame — reverting advances made in the same tick.

The result: flickering, half-speed playback, and a speed slider that snaps back
to 1.0×. See audit §4–§5 for the full analysis.

Patching individual symptoms won't fix the architecture. This plan guts the
playback wiring and rebuilds it with a single source of truth.

---

## Approach

Two commits:

1. **Gut** — remove all playback advancement, frame display, and speed sync
   systems. The app compiles and runs but playback is inert (frames load, UI
   panel renders, buttons do nothing, no frame displayed).

2. **Rebuild** — add clean systems back with unidirectional data flow.

---

## Commit 1: Gut

### Remove (systems / registrations)

| System | Location | Reason |
|--------|----------|--------|
| `handle_playback_input` | `playback.rs` → `SequencePlaybackPlugin` | Competing writer to `sequence_manager.current_frame` |
| `playback_update_system` | `playback_controls.rs` → `UiSystemsPlugin` | Competing writer via `ui_state.next_frame()` |
| `sync_sequence_manager_with_events` | `sequence/mod.rs` → `SequencePlugin` | Event relay that races with direct writes |
| `manage_frame_window` | `loader.rs` → `SequenceLoaderPlugin` | Frame display — will be rebuilt |

### Gut (modify in place)

| Item | Change |
|------|--------|
| `playback_controls_system` | Remove sync block (lines that overwrite `ui_state` from `sequence_manager`). Remove speed sync hack. Buttons and slider remain in UI but are **inert** — no events written, no state mutated. |
| `SequencePlaybackPlugin::build` | Empty body (or remove plugin registration entirely) |
| `SequenceManager.playback_fps` | Remove field. Speed is only `ui_state.playback.speed`. |

### Remove (dead code)

| Item | Location |
|------|----------|
| `PlaybackControl` enum | `playback.rs` |
| `PlaybackControlSender` component | `playback.rs` |
| `playback::PlaybackState` struct | `playback.rs` (not to be confused with `ui::state::PlaybackState`) |
| `PlaybackStats` resource | `playback.rs` |
| `KeyHoldTimer` resource | `playback.rs` |
| `UiState::next_frame` | `state.rs` |
| `UiState::previous_frame` | `state.rs` |
| `UiState::seek_to_frame` | `state.rs` |
| `UiState::toggle_playback` | `state.rs` |

### Keep (untouched)

| Item | Reason |
|------|--------|
| `handle_load_requests` | Asset loading — no playback involvement |
| `track_asset_loading` | Marks frames loaded, creates shared material |
| `SequenceAssets` | Stores frame handles, entities, material |
| `SequenceManager` (struct + methods) | Will become single source of truth |
| `SequenceEvent` enum | Will be used in rebuild |
| `ui::state::PlaybackState` | Display-only state — stays on `UiState` |
| `RenderConfig` | Frame window size config — used by rebuilt `manage_frame_window` |

### Expected state after commit 1

- `cargo build` succeeds
- App launches, loads meshes (assets load to completion)
- UI panel shows frame 0/N, speed slider, loop checkbox — all inert
- No mesh visible (no frame display system)
- No playback, no keyboard shortcuts
- No flickering (nothing to flicker)

---

## Commit 2: Rebuild

### Architecture

Single source of truth: **`SequenceManager`**

```text
                    ┌────────────────────┐
  keyboard ────────►│  SequenceManager   │◄──────── UI buttons
  (Update)          │  .current_frame    │          (EguiPass)
                    │  .is_playing       │
  auto-advance ────►│  .loop_enabled     │
  (Update)          │  .speed            │◄──────── UI slider
                    └─────────┬──────────┘          (EguiPass)
                              │
                    ┌─────────▼──────────┐
                    │  sync_ui_display   │  (Update, after inputs)
                    │  SM → UiState      │  ONE-WAY, read-only
                    └─────────┬──────────┘
                              │
                    ┌─────────▼──────────┐
                    │ manage_frame_window │  (PostUpdate)
                    │ reads SM.current_frame
                    │ sole owner of spawn/despawn/visibility
                    └────────────────────┘
```

### Move `speed` and `loop_enabled` into `SequenceManager`

Currently `speed` and `loop_enabled` live on `UiState::PlaybackState`. Since
`SequenceManager` is the single source of truth, move them there. The UI reads
from `SequenceManager` via the one-way sync, and writes directly to
`SequenceManager` on slider/checkbox change.

This eliminates the speed sync problem entirely — one field, one location.

### New systems

#### 1. `advance_playback` (Update)

Timer-based auto-advance. Reads `sequence_manager.speed` and
`sequence_manager.loop_enabled`. Writes `sequence_manager.current_frame`.

```rust
fn advance_playback(
    mut sm: ResMut<SequenceManager>,
    time: Res<Time>,
    mut accumulator: Local<f32>,
) {
    if !sm.is_playing || sm.current_sequence.is_none() { return; }

    let frame_duration = 1.0 / (30.0 * sm.speed);
    *accumulator += time.delta_secs();

    if *accumulator >= frame_duration {
        *accumulator -= frame_duration;

        let total = sm.current_sequence().unwrap().frame_count();
        if sm.current_frame + 1 < total {
            sm.current_frame += 1;
        } else if sm.loop_enabled {
            sm.current_frame = 0;
        } else {
            sm.is_playing = false;
        }
    }
}
```

#### 2. `handle_keyboard_playback` (Update)

Keyboard input → `SequenceManager`. Replaces `handle_playback_input`.
Same key bindings (space, arrows, home/end, +/-, digits), but writes
directly to `sequence_manager.current_frame`, `sequence_manager.is_playing`,
`sequence_manager.speed`.

No events, no indirection.

#### 3. `sync_ui_from_sequence_manager` (Update, after 1 & 2)

One-way sync: `SequenceManager` → `UiState::PlaybackState` for display.

```rust
fn sync_ui_from_sequence_manager(
    sm: Res<SequenceManager>,
    mut ui_state: ResMut<UiState>,
) {
    if let Some(seq) = sm.current_sequence() {
        ui_state.playback.total_frames = seq.frame_count();
    }
    ui_state.playback.current_frame = sm.current_frame;
    ui_state.playback.is_playing = sm.is_playing;
    ui_state.playback.speed = sm.speed;
    ui_state.playback.loop_enabled = sm.loop_enabled;
}
```

#### 4. `playback_controls_system` (EguiPrimaryContextPass)

UI renders from `ui_state.playback` (read-only mirror). When buttons are
clicked or sliders change, write directly to `sequence_manager`:

```rust
// Play/Pause
if play_button.clicked() {
    sequence_manager.is_playing = !sequence_manager.is_playing;
}

// Previous frame
if prev_button.clicked() {
    sequence_manager.previous_frame();
}

// Speed slider — bind directly to sequence_manager.speed
ui.add(egui::Slider::new(&mut sequence_manager.speed, 0.1..=5.0));

// Timeline scrubber
let mut frame = sequence_manager.current_frame;
if ui.add(egui::Slider::new(&mut frame, 0..=total)).changed() {
    sequence_manager.jump_to_frame(frame);
}
```

No events written. No `ui_state` mutation. The next tick's `sync_ui_from_sequence_manager`
will propagate the change for display.

#### 5. `manage_frame_window` (PostUpdate)

Same ring-buffer logic as before but with the O(N) despawn fix from audit §4
Issue 4: track `(prev_lo, prev_hi)` and only despawn/spawn the delta.

### System ordering

```rust
// In SequencePlugin::build
app.add_systems(Update, (
    advance_playback,
    handle_keyboard_playback,
).before(sync_ui_from_sequence_manager));

app.add_systems(Update, sync_ui_from_sequence_manager);

// In SequenceLoaderPlugin::build
app.add_systems(Update, (handle_load_requests, track_asset_loading).chain());
app.add_systems(PostUpdate, manage_frame_window);

// In UiSystemsPlugin::build (EguiPrimaryContextPass)
// playback_controls_system — reads ui_state, writes sequence_manager
```

### `SequenceManager` changes

```rust
pub struct SequenceManager {
    pub current_sequence: Option<Sequence>,
    pub current_frame: usize,
    pub is_playing: bool,
    pub speed: f32,         // was playback_fps, now a multiplier (1.0 = normal)
    pub loop_enabled: bool, // moved from UiState
}
```

### `UiState::PlaybackState` becomes read-only mirror

```rust
pub struct PlaybackState {
    pub is_playing: bool,      // mirror of SM
    pub current_frame: usize,  // mirror of SM
    pub total_frames: usize,   // mirror of SM
    pub speed: f32,            // mirror of SM
    pub loop_enabled: bool,    // mirror of SM
}
```

No methods on `PlaybackState` that mutate frame state. The struct exists
purely so egui systems can read it without borrowing `SequenceManager`.

### Settings save/load

`handle_save_view` currently reads `ui_state.playback.speed` and
`ui_state.playback.loop_enabled`. After rebuild, read from `sequence_manager`
instead (or from `ui_state` since it's a faithful mirror — either works).

`setup` currently writes to `ui_state.playback.speed`. Change to write to
`sequence_manager.speed` and `sequence_manager.loop_enabled`.

### Expected state after commit 2

- `cargo build` succeeds
- Playback works: space to play/pause, arrows to step, +/- to change speed
- Speed slider works and sticks
- Loop checkbox works
- No flickering — single source of truth, no competing writers
- Ring-buffer window with O(delta) despawn
- Settings save/load works with new fields

---

## Verification

1. `cargo build` — no errors or warnings
2. Load a sequence directory — frames load, first frame visible
3. Press space — playback starts, frames advance smoothly
4. Press space again — stops
5. Drag speed slider — sticks at new value, playback speed changes
6. Arrow keys — step forward/backward
7. Toggle loop — plays to end and either loops or stops
8. Save view — `seaview.toml` captures speed + loop
9. Restart — settings restored
10. No flickering at any point

---

## Files Modified

| File | Commit 1 (Gut) | Commit 2 (Rebuild) |
|------|-----------------|---------------------|
| `sequence/mod.rs` | Remove `sync_sequence_manager_with_events`, remove `playback_fps` | Add `speed`, `loop_enabled` to SM, add `advance_playback`, `sync_ui_from_sequence_manager` |
| `sequence/playback.rs` | Gut entire file (empty or remove) | Rebuild with `handle_keyboard_playback` only |
| `sequence/loader.rs` | Remove `manage_frame_window` registration | Rebuild `manage_frame_window` with delta tracking |
| `ui/systems/playback_controls.rs` | Gut sync block, make buttons inert, remove `playback_update_system` | Rebuild buttons to write directly to SM |
| `ui/systems/mod.rs` | Remove `playback_update_system` registration | No change |
| `ui/state.rs` | Remove frame advancement methods from `UiState` | No change (PlaybackState is now read-only mirror) |
| `main.rs` | No change | Update settings save/load to use SM |
| `docs/plan-playback-rebuild.md` | This file | — |