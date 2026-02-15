# 06 — 3D Manipulation System

> Camera animation, lighting rig interaction, and reusable spatial UI primitives.

## Motivation

Seaview currently has an FPS-style camera controller (WASD + mouse look) and a
lighting rig driven entirely by sliders in the egui environment panel. There's no
way to:

- Animate the camera along a path for presentation/export
- Click-drag a light in the 3D viewport to reposition it
- Move any invisible object (waypoint, probe, annotation anchor) in world space

All three needs share a common substrate: **a set of 3D manipulation building
blocks** — gizmo handles, axis constraints, mouse-ray picking, and multi-speed
keyboard nudging.

This document plans that substrate and the two features that ride on top of it.

---

## Table of Contents

1. [Design Principles](#design-principles)
2. [Feature 1 — 3D Manipulation Primitives](#feature-1--3d-manipulation-primitives)
3. [Feature 2 — Camera Animation](#feature-2--camera-animation)
4. [Feature 3 — Interactive Lighting Rig](#feature-3--interactive-lighting-rig)
5. [Architecture](#architecture)
6. [Interaction Design](#interaction-design)
7. [Keyboard Reference](#keyboard-reference)
8. [Implementation Phases](#implementation-phases)
9. [Open Questions](#open-questions)

---

## Design Principles

| Principle | Rationale |
|-----------|-----------|
| **Mouse-first** | Click to select, drag to move. Keyboard is the precision layer, not the primary input. |
| **Axis constraints feel like Blender** | Hold X/Y/Z during a drag to lock to that world axis. Intuitive for 3D users. |
| **`lib/` stays pure** | All math (ray-plane intersection, spline evaluation, constraint projection) lives in `lib/spatial/`. No Bevy queries, no `Entity`, no `Res<>`. |
| **`app/` is glue** | Thin systems read input → call `lib/` → write transforms. Bevy-dependent state (`ManipulationState`, `ActiveDrag` with `Entity`) lives in `app/`. |
| **Composable primitives** | A "manipulable" is just a `Manipulable` component + an optional `GizmoVisual`. Camera keyframes, light positions, and future probes all reuse the same machinery. |
| **Non-destructive** | Dragging shows a preview. Commit on mouse-up. Escape cancels. Undo stack (future). |

> **Architecture cross-ref:** see [`docs/architecture.md`](../architecture.md) for the
> `lib/` purity rules and resource ownership conventions this plan follows.

---

## Feature 1 — 3D Manipulation Primitives

The reusable building blocks that everything else consumes.

### 1.1 Manipulable Component (`app/`)

`Manipulable` is a Bevy `Component` — it lives in `app/`, not `lib/`.

```rust
/// Attach to any entity to make it manipulable in the 3D viewport.
/// Lives in app/systems/manipulation.rs (Bevy Component).
#[derive(Component)]
pub struct Manipulable {
    /// Whether this object is currently selected for manipulation.
    pub selected: bool,
    /// Which axes are enabled (bitflags: X=1, Y=2, Z=4, ALL=7).
    pub axes: u8,
    /// Snap increment (0.0 = free movement).
    pub snap: f32,
    /// Minimum / maximum bounds (optional clamping).
    pub bounds: Option<(Vec3, Vec3)>,
}
```

### 1.2 Gizmo Visuals

Optional visual handles rendered at the entity's position when selected:

| Handle | Appearance | Action |
|--------|-----------|--------|
| **Center sphere** | Small sphere at origin | Free-move (all axes) |
| **X arrow** | Red arrow/cone along +X | Constrain to X |
| **Y arrow** | Green arrow/cone along +Y | Constrain to Y |
| **Z arrow** | Blue arrow/cone along +Z | Constrain to Z |
| **XY plane** | Small red-green quad | Constrain to XY plane |
| **XZ plane** | Small red-blue quad | Constrain to XZ plane |
| **YZ plane** | Small green-blue quad | Constrain to YZ plane |

Gizmos scale with camera distance so they remain a constant screen size.
Rendered using Bevy's `Gizmos` API (line primitives) — no extra mesh assets needed.

### 1.3 Picking / Hit Testing

- **Mouse ray** — cast from camera through cursor position every frame.
- **Gizmo hit test** — sphere/cylinder intersection against gizmo handles.
  Priority: arrow > plane > center (closest hit wins).
- **Entity hit test** — optional AABB test for entities with `Manipulable` but
  no visible gizmo (e.g. invisible waypoints).
- All intersection math in `lib/spatial/picking.rs`. Pure functions:
  `ray_sphere`, `ray_cylinder`, `ray_plane`, `ray_aabb`.

### 1.4 Drag State Machine

```
Idle
  │ click on gizmo handle or manipulable entity
  ▼
DragActive { entity, constraint, start_world_pos, start_mouse_pos }
  │ mouse move → project onto constraint → preview position
  │ hold X/Y/Z → override constraint
  │ Escape → cancel (revert to start_world_pos) → Idle
  │ mouse up → commit → Idle
  ▼
Idle
```

**`AxisConstraint`** is a pure enum — lives in `lib/spatial/constraints.rs`:

```rust
/// Which axis or plane a drag is constrained to.
/// Pure type — no Bevy dependencies.  (lib/spatial/constraints.rs)
pub enum AxisConstraint {
    Free,
    AxisX,
    AxisY,
    AxisZ,
    PlaneXY,
    PlaneXZ,
    PlaneYZ,
}
```

**`ManipulationState`** and **`ActiveDrag`** hold `Entity` (a Bevy type), so
they live in `app/systems/manipulation.rs`, NOT in `lib/`:

```rust
/// Bevy resource tracking the current manipulation interaction.
/// Lives in app/systems/manipulation.rs.
#[derive(Resource, Default)]
pub struct ManipulationState {
    pub active_drag: Option<ActiveDrag>,
    pub hovered_handle: Option<HandleId>,
}

/// Active drag state.  Contains Entity — must stay in app/.
pub struct ActiveDrag {
    pub entity: Entity,
    pub constraint: AxisConstraint,
    pub start_world: Vec3,
    pub start_screen: Vec2,
    pub current_world: Vec3,
}
```

### 1.5 Constraint Projection (pure `lib/`)

Given a mouse ray and an axis constraint, compute the closest point on the
constrained line/plane to the ray:

```rust
/// Project a mouse ray onto a constrained axis through `origin`.
/// Returns the world-space point on the axis closest to the ray.
pub fn project_ray_to_axis(
    ray_origin: Vec3,
    ray_dir: Vec3,
    axis_origin: Vec3,
    axis_dir: Vec3,
) -> Vec3 { ... }

/// Project a mouse ray onto a constrained plane through `origin`.
pub fn project_ray_to_plane(
    ray_origin: Vec3,
    ray_dir: Vec3,
    plane_origin: Vec3,
    plane_normal: Vec3,
) -> Option<Vec3> { ... }
```

### 1.6 Keyboard Nudging

When an entity is selected (but no drag is active), arrow keys move it:

| Keys | Speed | Typical use |
|------|-------|-------------|
| Arrow keys | 1× base (0.1 m/press) | Normal positioning |
| **Ctrl** + Arrow | 10× (1.0 m/press) | Coarse movement |
| **Shift** + Arrow | 0.01× (0.001 m/press) | Fine-tuning |
| **Alt** + Arrow | Snap-to-grid | Snap to nearest `snap` increment |

Arrow key mapping in 3D:

| Key | Camera-relative direction |
|-----|--------------------------|
| ← / → | Camera right axis (X-ish) |
| ↑ / ↓ | Camera forward axis projected onto XZ (depth) |
| **PgUp / PgDn** | World Y axis (up/down) |

When an axis constraint key (X/Y/Z) is held, arrows always move along that
world axis regardless of camera orientation.

---

## Feature 2 — Camera Animation

Animate the camera along a user-defined path for flythroughs and video export.

### 2.1 Concepts

| Concept | Description |
|---------|-------------|
| **Keyframe** | A saved `(position, rotation, time)` tuple. Represented as an entity with `Manipulable` + `CameraKeyframe` components. |
| **Path** | Ordered list of keyframes. Catmull-Rom spline interpolation between them. |
| **Playback** | Separate from sequence playback. Camera animation has its own timeline (seconds, not frames). Can optionally sync to sequence playback. |
| **Preview** | While editing, the spline path is rendered as a Gizmo polyline. Keyframe positions shown as small camera icons. |

### 2.2 Data Model (`lib/camera_path.rs`)

```rust
pub struct CameraKeyframe {
    /// Position in world space
    pub position: Vec3,
    /// Rotation (look direction)
    pub rotation: Quat,
    /// Time in seconds from path start
    pub time: f32,
    /// Easing function for transition TO this keyframe
    pub easing: EasingFunction,
}

pub struct CameraPath {
    pub keyframes: Vec<CameraKeyframe>,
    pub loop_mode: LoopMode,
    pub total_duration: f32,  // derived from last keyframe time
}

pub enum LoopMode { Once, Loop, PingPong }
pub enum EasingFunction { Linear, EaseInOut, EaseIn, EaseOut }
```

### 2.3 Spline Evaluation (pure `lib/`)

```rust
/// Evaluate camera position and rotation at time `t` along the path.
/// Uses Catmull-Rom for position, Slerp for rotation.
pub fn evaluate(path: &CameraPath, t: f32) -> (Vec3, Quat) { ... }

/// Sample the path at N evenly-spaced points for preview rendering.
pub fn sample_path(path: &CameraPath, num_samples: usize) -> Vec<Vec3> { ... }
```

### 2.4 Workflow

1. **Enter camera animation mode** — toggle via toolbar or hotkey (`Ctrl+K`).
2. **Place keyframes** — press `K` to drop a keyframe at the current camera
   position/rotation. Or click in the viewport to place a keyframe at a picked
   point (camera will look toward the mesh centroid by default).
3. **Edit keyframes** — keyframes appear as manipulable entities. Drag to
   reposition. Hold `R` + drag to adjust look direction.
4. **Preview** — press `Space` to play the camera path. Sequence playback can
   optionally sync (1 second of camera time = N sequence frames).
5. **Export** — render each frame along the path to PNG (future feature).

### 2.5 Persistence (`seaview.toml`)

```toml
[[camera_paths]]
name = "flythrough-1"
loop_mode = "once"

[[camera_paths.keyframes]]
position = [-72.1, 8.3, 43.8]
rotation = [0.0, -0.53, 0.0, 0.85]  # quaternion xyzw
time = 0.0
easing = "ease_in_out"

[[camera_paths.keyframes]]
position = [10.0, 25.0, -30.0]
rotation = [-0.1, 0.2, 0.0, 0.97]
time = 5.0
easing = "linear"
```

---

## Feature 3 — Interactive Lighting Rig

Make the existing `NightLightingConfig` lights directly manipulable in the
viewport, replacing slider-only positioning.

### 3.1 Current State

- `NightLightingConfig` controls light count, height, placement algorithm, etc.
- `NightLightMarker` entities (small spheres) already render at light positions.
- Positions are computed algorithmically from `placement.rs` — no per-light
  manual override.

### 3.2 Changes

| Change | Description |
|--------|-------------|
| **Add `Manipulable` to `NightLightMarker`** | Light markers become draggable. |
| **Per-light position overrides** | `NightLightingConfig` gains `overrides: Vec<Option<Vec3>>`. `None` = algorithm-placed, `Some(pos)` = user-placed. |
| **Drag a light → sets override** | On drag commit, write the new position to `overrides[index]`. |
| **Reset** | Right-click a light marker → clear override (snap back to algorithm position). |
| **Height constraint** | By default, dragging a light constrains to the XZ plane at the current height. Hold `Y` to adjust height. |
| **Persistence** | Light position overrides saved in `seaview.toml`. |

### 3.3 Persistence (`seaview.toml`)

```toml
[environment.lighting]
num_lights = 9
height = 12.6
# ... existing fields ...

# Per-light overrides (index → position). Omitted indices use algorithm placement.
[[environment.lighting.overrides]]
index = 3
position = [10.5, 12.6, -22.0]

[[environment.lighting.overrides]]
index = 7
position = [-5.0, 8.0, 30.0]
```

---

## Architecture

Following the project's `lib/` = pure, `app/` = glue convention:

```
lib/                            (pure logic — NO Entity, Res<>, Query<>, Commands)
├── spatial/                    ← NEW: 3D manipulation math primitives
│   ├── mod.rs                  ← AxisConstraint enum, GizmoHandleId, re-exports
│   ├── picking.rs              ← ray_sphere, ray_cylinder, ray_plane, ray_aabb
│   ├── constraints.rs          ← project_ray_to_axis, project_ray_to_plane, snap_to_grid
│   └── gizmo.rs                ← Gizmo handle layout geometry, hit-test sizing
├── camera_path.rs              ← NEW: CameraPath, CameraKeyframe, evaluate(), sample_path()
└── lighting/
    └── mod.rs                  ← NightLightingConfig gains `overrides: Vec<Option<Vec3>>`

app/                            (Bevy systems + components — thin wrappers)
├── systems/
│   ├── manipulation.rs         ← NEW: Manipulable (Component), ManipulationState (Resource),
│   │                              ActiveDrag, pick_system, drag_system, nudge_system,
│   │                              gizmo_render_system
│   └── camera_animation.rs     ← NEW: camera_path_playback, keyframe_placement, path_preview
└── ui/
    └── systems/
        └── camera_path_panel.rs ← NEW: egui panel for keyframe list, timing, easing curves
```

> **Layer rule reminder:** `lib/spatial/` contains zero Bevy system signatures.
> All functions take plain args (`Vec3`, `f32`, `&[CameraKeyframe]`) and return
> plain results.  `Manipulable`, `ManipulationState`, `ActiveDrag` contain
> `Entity` / are `Component` / `Resource` — they live in `app/systems/manipulation.rs`.

### Resource Ownership (additions)

| Domain | Authority | Mirror |
|--------|-----------|--------|
| Manipulation state | `ManipulationState` | — |
| Camera paths | `CameraPathLibrary` (Vec of CameraPath) | — |
| Camera anim playback | `CameraAnimPlayer` (current path, time, playing) | — |
| Light overrides | `NightLightingConfig.overrides` | — |

### System Ordering

```
PreUpdate:
  update_mouse_ray          ← compute ray from camera + cursor each frame

Update:
  pick_system               ← hit-test gizmos and manipulables, set hovered
  drag_system               ← if drag active: project mouse ray → update preview
  nudge_system              ← keyboard arrow nudges for selected entity
  commit_or_cancel_drag     ← mouse-up commits, Escape cancels
  camera_path_playback      ← if camera anim playing: evaluate spline → set camera transform
  gizmo_render_system       ← draw gizmo handles for selected entities
  path_preview_system       ← draw spline polyline + keyframe markers

PostUpdate:
  (existing manage_frame_window, etc.)
```

---

## Interaction Design

### Selection

- **Click** on a manipulable entity or gizmo handle → select it.
- **Click** on empty space → deselect.
- Only **one** entity selected at a time (single-select for now).
- Selected entity gets a highlight outline (emissive boost or wireframe overlay).

### Drag

1. Click + hold on a gizmo handle → enters drag mode.
2. Mouse movement projects onto the constraint (axis or plane) of that handle.
3. While dragging, pressing **X**, **Y**, or **Z** overrides the constraint.
4. Pressing **Shift** while dragging enables fine mode (0.1× sensitivity).
5. Pressing **Ctrl** while dragging enables snap mode (round to `snap` grid).
6. **Escape** cancels the drag and reverts to the start position.
7. **Mouse-up** commits the new position.

### Visual Feedback

| State | Visual |
|-------|--------|
| Hovered handle | Handle brightens / thickens |
| Active drag | Dragged axis drawn as a dashed line through the scene; position readout near cursor |
| Constraint override | Only the active axis handle is drawn; others dim |
| Snap active | Grid lines flash on the constrained plane |
| Keyboard nudge | Brief positional delta shown as floating text |

### Conflict with FPS Camera

The FPS camera currently grabs the cursor on left-click. With manipulation mode:

- **Escape mode** (default on launch): cursor free. Clicking picks/selects entities.
- **Camera mode** (left-click when no entity hit): grabs cursor for FPS look.
- **Manipulation mode**: left-click on a gizmo starts a drag instead of camera.
- Simple priority: gizmo hit > entity hit > camera grab.

---

## Keyboard Reference

### Manipulation Mode

| Key | Action |
|-----|--------|
| **Click** | Select entity / start drag on gizmo |
| **Escape** | Cancel drag / deselect |
| **X** / **Y** / **Z** (hold) | Constrain to axis during drag |
| **Shift** (hold) | Fine mode (0.1× drag sensitivity, 0.01× nudge) |
| **Ctrl** (hold) | Coarse mode (10× nudge), snap mode during drag |
| **Alt** (hold) | Snap-to-grid nudge |
| **← → ↑ ↓** | Nudge selected entity (camera-relative) |
| **PgUp / PgDn** | Nudge selected entity along world Y |
| **Delete** | Delete selected keyframe (camera animation mode) |

### Camera Animation Mode

| Key | Action |
|-----|--------|
| **Ctrl+K** | Toggle camera animation mode |
| **K** | Place keyframe at current camera position |
| **Space** | Play / pause camera animation preview |
| **Ctrl+Shift+K** | Clear all keyframes |
| **[ / ]** | Select prev / next keyframe |
| **R** (hold) + drag | Adjust keyframe look direction |

---

## Implementation Phases

### Phase 0 — Spatial Math (`lib/spatial/`) ◆ No UI, no Bevy runtime

Pure library code, fully testable. Functions take `Vec3`/`f32`, return `Vec3`/`Option<f32>`.

- [ ] `picking.rs` — `ray_sphere`, `ray_cylinder`, `ray_plane`, `ray_aabb`
- [ ] `constraints.rs` — `AxisConstraint` enum, `project_ray_to_axis`, `project_ray_to_plane`, `snap_to_grid`
- [ ] `gizmo.rs` — gizmo handle geometry (positions/radii for hit testing)
- [ ] Unit tests for all intersection and projection functions (no Bevy App needed)

### Phase 1 — Mouse Ray + Picking (`app/systems/manipulation.rs`)

- [ ] `Manipulable` component, `ManipulationState` resource, `ActiveDrag` struct (all in `app/`)
- [ ] `MouseRay` resource — updated from camera + cursor each frame
- [ ] `pick_system` — cast ray (via `lib/spatial/picking`), set `ManipulationState.hovered`
- [ ] Visual hover feedback (gizmo handle color change)
- [ ] Wire into `app/systems/mod.rs` plugin

### Phase 2 — Drag + Nudge

- [ ] `drag_system` — drag state machine (start / update / commit / cancel)
- [ ] Axis constraint override (X/Y/Z keys during drag)
- [ ] Speed modifiers (Shift = fine, Ctrl = snap)
- [ ] `nudge_system` — arrow keys move selected entity
- [ ] Conflict resolution with FPS camera (priority: gizmo > entity > camera)

### Phase 3 — Interactive Lighting Rig

- [ ] Add `Manipulable` component to `NightLightMarker` entities
- [ ] `overrides: Vec<Option<Vec3>>` in `NightLightingConfig`
- [ ] On drag commit → write override
- [ ] Right-click → clear override
- [ ] XZ-plane default constraint, Y key to adjust height
- [ ] Save/restore overrides in `seaview.toml`
- [ ] Update `lighting_section.rs` UI to show override indicators

### Phase 4 — Camera Path Data Model

Pure `lib/` code, no systems.

- [ ] `lib/camera_path.rs` — `CameraKeyframe`, `CameraPath`, `LoopMode`, `EasingFunction`
- [ ] Catmull-Rom spline evaluation for position
- [ ] Slerp interpolation for rotation
- [ ] `sample_path()` for preview polyline
- [ ] Easing functions: linear, ease-in, ease-out, ease-in-out
- [ ] Unit tests for spline evaluation and edge cases
- [ ] `seaview.toml` persistence (serialize/deserialize)

### Phase 5 — Camera Animation Systems

- [ ] `CameraPathLibrary` + `CameraAnimPlayer` resources
- [ ] Keyframe placement system (K key → save current camera)
- [ ] Path preview rendering (gizmo polyline + keyframe markers)
- [ ] Camera path playback system (evaluate spline → override camera transform)
- [ ] Optional sync with sequence playback (seconds ↔ frames mapping)
- [ ] `camera_path_panel.rs` — egui panel: keyframe list, timing editor, easing selector

### Phase 6 — Polish

- [ ] Undo/redo for manipulation actions
- [ ] Multi-select (Shift+Click to add to selection)
- [ ] Rotation gizmo (ring handles for pitch/yaw/roll)
- [ ] Scale gizmo (if needed for future features)
- [ ] Export camera animation to video (frame-by-frame PNG dump)

---

## Open Questions

1. **Gizmo rendering approach** — Bevy's `Gizmos` API (immediate-mode lines) vs.
   spawned mesh entities? Gizmos are simpler and always-on-top, but don't support
   hit-testing natively. Likely: use Gizmos for rendering, separate ray math for
   picking.

2. **Camera animation sync** — should camera time be in seconds or sequence
   frames? Leaning toward seconds with an optional frames-per-second mapping so
   the camera path is independent of sequence length.

3. **Rotation manipulation** — Phase 6 scope. For camera keyframes, rotation is
   critical. For lights, less so (they point down). Should rotation gizmo be part
   of Phase 2 or deferred?

4. **Screen-space vs. world-space gizmo size** — constant screen size (like
   Blender) requires scaling by camera distance each frame. Adds a bit of
   complexity but vastly better UX.

5. **bevy_mod_picking** — should we use the community picking crate or roll our
   own? Our needs are narrow (gizmo handles + AABBs), so rolling our own in
   `lib/spatial/` keeps dependencies minimal and the math testable. Revisit if
   needs grow.

6. **`lib/` purity boundary** — `AxisConstraint`, `EasingFunction`, `LoopMode`,
   `CameraKeyframe`, `CameraPath` are all pure types (no `Entity`). They live in
   `lib/`. Anything referencing `Entity`, `Component`, `Resource`, or Bevy system
   params lives in `app/`. This is consistent with how `Timeline` (pure) vs
   `SequenceAssets` (Bevy resource) are split today.