# Camera Rigs, Animation & Lighting — Research

Last updated: 2026-02-18

## What Seaview Does Today

Seaview is a Bevy-based 3D viewer for time-series fluid simulation meshes. It loads sequences of STL/OBJ/GLB frames, plays them back with memory-budget-aware frame windowing, and lets users configure lighting, materials, and imported environment objects.

### What works

| Area | Status | Details |
|------|--------|---------|
| Sequence playback | ✅ Solid | Timeline, multi-section load windows, budget enforcement, 94 tests |
| FPS camera | ✅ Basic | WASD + mouse look, scroll-wheel speed, Shift/Alt modifiers, Ctrl+C center-on-mesh |
| Lighting rig | ✅ Good | Named groups, spot lights, 5 placement algorithms, per-light editing, 47 tests |
| Environment objects | ✅ Working | STL/OBJ/PLY direct mesh + GLB scene import, transform, swizzle, material override |
| Persistence | ✅ Working | Camera, lighting, materials, env objects, load sections → `seaview.toml` |
| Memory budget | ✅ Working | NVML VRAM detection, frame promote/demote, budget-limited loading |

### What's missing

| Area | Status | Impact |
|------|--------|--------|
| Camera animation / flythrough | ❌ None | Can't record or replay camera paths for presentations |
| Camera rigs (orbit, turntable, dolly) | ❌ None | Only FPS mode — no orbit-around-object, no constrained motion |
| Multiple named cameras with rig types | 🟡 Partial | Named cameras persist position/rotation but not rig type |
| Lighting animation | ❌ None | Lights are static — no day/night cycle, no animated key lights |
| 3D gizmos for cameras | ❌ None | No visual representation of saved cameras in viewport |
| 3D gizmos for lights | 🟡 Partial | Marker spheres exist but no cone visualization, no drag handles |
| Curve editing (bezier etc.) | ❌ None | No spline editing UI or data model |

---

## Camera Rigs

### What is a camera rig?

A rig constrains camera motion to a specific pattern. Different rigs suit different tasks:

| Rig | Motion | Use case |
|-----|--------|----------|
| **FPS / Free** | 6DOF, WASD + mouse | Exploring unknown geometry, debugging |
| **Orbit** | Rotate around a focus point, zoom in/out | Inspecting a specific object from all angles |
| **Turntable** | Orbit locked to Y-axis rotation | Presentation turnarounds, consistent framing |
| **Dolly** | Translate along a rail (line or curve) | Cinematic tracking shots |
| **Crane** | Pivot around a boom arm | Overhead-to-ground sweeps |
| **Pan/Tilt** | Fixed position, rotate only | Surveillance-style overview |
| **Follow** | Track a moving target with offset | Following a particle or wavefront |

### Recommended for seaview

**Phase 1:** Orbit + FPS (two modes, toggle with hotkey)
**Phase 2:** Turntable (trivial extension of orbit with locked axis)
**Phase 3:** Dolly along curve (requires curve data model)

### Orbit rig design

```
struct OrbitRig {
    focus: Vec3,          // point to orbit around
    distance: f32,        // distance from focus
    yaw: f32,             // horizontal angle (radians)
    pitch: f32,           // vertical angle, clamped to avoid gimbal
    pitch_limits: (f32, f32), // typically (-85°, 85°)
}
```

Input mapping:
- Left drag → orbit (yaw/pitch)
- Right drag → pan (translate focus in screen plane)
- Scroll → zoom (adjust distance, optionally with min/max)
- Middle click → reset to default view
- Double-click mesh → set focus to hit point (uses existing `SpatialMesh::cast_ray`)

The orbit rig produces a `Transform` each frame:

```
let rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, 0.0);
let position = focus + rotation * Vec3::new(0.0, 0.0, distance);
Transform::from_translation(position).looking_at(focus, Vec3::Y)
```

### Rig switching

```
enum CameraRig {
    Fps(FpsCamera),
    Orbit(OrbitRig),
    Turntable { orbit: OrbitRig, auto_rotate_speed: f32 },
    Dolly { curve: CameraPath, t: f32, speed: f32 },
}
```

Each variant produces a `Transform`. One system reads the active rig, calls the appropriate update, and writes the camera transform. The rig type persists to `seaview.toml` alongside camera position.

---

## Camera Animation / Flythrough

### Data model

A camera path is a sequence of keyframes interpolated by a spline:

```
struct CameraKeyframe {
    position: Vec3,
    look_at: Vec3,       // or: rotation: Quat
    fov: Option<f32>,    // optional FOV animation
    time: f32,           // normalized [0, 1] or absolute seconds
    easing: EasingCurve, // per-segment easing
}

struct CameraPath {
    keyframes: Vec<CameraKeyframe>,
    curve_type: CurveType,
    duration_seconds: f32,
    loop_mode: LoopMode,
}

enum CurveType {
    CubicBezier,       // explicit control points
    CatmullRom,        // passes through all keyframes
    CubicHermite,      // keyframes + tangent vectors
    Linear,            // straight-line segments
}

enum LoopMode {
    Once,
    Loop,
    PingPong,
}
```

### Position vs. look-at interpolation

Position interpolation is straightforward (spline through Vec3 keyframes). Look-at is harder:

| Approach | Pros | Cons |
|----------|------|------|
| Interpolate look-at point on separate spline | Intuitive, easy to visualize | Look-at and position splines can diverge causing jerky rotation |
| Interpolate quaternion (Slerp) | Smooth rotation guaranteed | Hard to author — quaternions aren't human-friendly |
| Interpolate yaw/pitch/roll | Human-readable | Gimbal lock near poles |
| **Recommended: Dual splines** | Position spline + look-at spline | Most animation packages do this. Visualize both curves. |

### Syncing with simulation playback

Camera animation time can be:
1. **Independent** — camera loops on its own clock
2. **Locked to playhead** — camera `t` = `playhead / total_frames`
3. **Triggered** — camera path starts when playback starts, runs at its own speed

Option 2 is most useful for seaview: the camera follows a path synchronized to the simulation timeline.

```
// In advance_playback system:
let t = timeline.playhead as f32 / timeline.total_frames as f32;
let position = camera_path.position_curve.sample_clamped(t);
let look_at = camera_path.look_at_curve.sample_clamped(t);
camera_transform = Transform::from_translation(position).looking_at(look_at, Vec3::Y);
```

### Bevy 0.18 curve primitives (available now)

Bevy's `bevy_math::cubic_splines` provides:

| Type | Description | Passes through control points? |
|------|-------------|-------------------------------|
| `CubicBezier<Vec3>` | Classic bezier, 4 control points per segment | First and last only |
| `CubicHermite<Vec3>` | Position + tangent pairs | Yes |
| `CubicCardinalSpline<Vec3>` | Catmull-Rom family, tension parameter | Yes |
| `CubicBSpline<Vec3>` | Smooth approximating spline | No |
| `CubicNurbs<Vec3>` | Weighted, non-uniform knots | No |

All implement `Curve<Vec3>` with `.sample(t)`, `.sample_clamped(t)`, `.domain()`. No external crate needed.

**Recommendation:** Use `CubicCardinalSpline` (Catmull-Rom) for camera paths. It passes through every keyframe, which is what users expect when placing waypoints.

---

## Lighting Rig Animation

### Current state

Lights are static. `LightingRig` contains `Vec<LightGroup>`, each with `Vec<Light>`. Each `Light` has position (Vec2 XZ), height, intensity, color, cone angle. `sync_lighting_rig` creates/updates Bevy `SpotLight` entities.

### Animation model

```
struct LightKeyframe {
    time: f32,
    position: Option<Vec3>,     // animate position
    intensity: Option<f32>,     // animate brightness
    color: Option<Color>,       // animate color (e.g. day/night)
    cone_angle: Option<f32>,    // animate spread
}

struct LightAnimation {
    light_id: Uuid,
    keyframes: Vec<LightKeyframe>,
    curve_type: CurveType,
}

struct LightingAnimation {
    animations: Vec<LightAnimation>,
    duration: f32,
    sync_to_playhead: bool,
}
```

### Animation types for lighting

| Type | Description | Use case |
|------|-------------|----------|
| **Day/night cycle** | Animate directional light color + intensity over time | Outdoor simulations |
| **Tracking light** | Light follows a point on the mesh (e.g. wavefront) | Highlighting flow features |
| **Sweep** | Light moves along a path across the scene | Dramatic reveal |
| **Pulse** | Intensity oscillates | Drawing attention to a region |
| **Color ramp** | Color transitions through a gradient | Encoding time or temperature |

### Integration with existing rig

The `LightingRig` is already the authority. Animation would work as a layer on top:

1. `LightingAnimation` resource holds animated properties
2. New system `animate_lighting_rig` runs before `sync_lighting_rig`
3. Each frame: sample curves at current time → write to `LightingRig` fields
4. `sync_lighting_rig` picks up changes via Bevy change detection (already works)

No architectural changes needed — just a new resource and one system.

---

## 3D Gizmos

### Available in Bevy 0.18

Bevy's `bevy_gizmos` provides immediate-mode drawing:

| Gizmo | API | Notes |
|-------|-----|-------|
| Line | `gizmos.line(a, b, color)` | |
| Arrow | `gizmos.arrow(start, end, color)` | 4-tip for 3D visibility |
| Sphere | `gizmos.sphere(isometry, radius, color)` | Wireframe sphere |
| Cuboid | `gizmos.cuboid(transform, color)` | Wireframe box |
| Circle | `gizmos.circle(isometry, radius, color)` | |
| Arc | `gizmos.arc_3d(...)` | |
| Axes | `gizmos.axes(transform, length)` | RGB XYZ axes |
| Curve | `gizmos.curve_3d(curve, times, color)` | Any `Curve<Vec3>` |
| AABB | `gizmos.aabb_3d(aabb, transform, color)` | |
| Retained | `Gizmo { handle, line_config, ... }` | Persistent, not per-frame |

Also: `GizmoConfigGroup` for per-category enable/disable, depth bias, line width.

### Gizmos for cameras

| Gizmo | What it shows | Implementation |
|-------|---------------|----------------|
| **Frustum** | Wireframe frustum at saved camera positions | 8 lines for frustum edges, or `gizmos.cuboid` with perspective transform |
| **Camera icon** | Small camera-shaped wireframe at position | Custom line art (body + lens cone) |
| **Path curve** | The camera flythrough path | `gizmos.curve_3d(path.position_curve, 0..100, color)` |
| **Keyframe markers** | Spheres at each keyframe position | `gizmos.sphere(pos, 0.5, color)` per keyframe |
| **Look-at lines** | Dashed lines from camera position to look-at target | `gizmos.line(cam_pos, look_at, color)` |
| **Up vector** | Small arrow showing camera up direction | `gizmos.arrow(pos, pos + up * scale, color)` |

Minimal implementation: frustum wireframe + path curve. Everything else is polish.

#### Frustum gizmo

```
fn draw_camera_frustum(gizmos: &mut Gizmos, transform: &Transform, fov: f32, aspect: f32, near: f32, far: f32) {
    let half_v = (fov / 2.0).tan();
    let half_h = half_v * aspect;

    // Near plane corners (camera-local)
    let corners_near = [
        Vec3::new(-half_h * near,  half_v * near, -near),
        Vec3::new( half_h * near,  half_v * near, -near),
        Vec3::new( half_h * near, -half_v * near, -near),
        Vec3::new(-half_h * near, -half_v * near, -near),
    ];
    // Far plane corners (same but scaled to far)
    let corners_far = [
        Vec3::new(-half_h * far,  half_v * far, -far),
        Vec3::new( half_h * far,  half_v * far, -far),
        Vec3::new( half_h * far, -half_v * far, -far),
        Vec3::new(-half_h * far, -half_v * far, -far),
    ];

    // Transform to world space and draw
    for i in 0..4 {
        let j = (i + 1) % 4;
        let n_i = transform.transform_point(corners_near[i]);
        let n_j = transform.transform_point(corners_near[j]);
        let f_i = transform.transform_point(corners_far[i]);
        let f_j = transform.transform_point(corners_far[j]);

        gizmos.line(n_i, n_j, CYAN);  // near rect
        gizmos.line(f_i, f_j, CYAN);  // far rect
        gizmos.line(n_i, f_i, CYAN);  // connecting edges
    }
}
```

### Gizmos for lights

Current state: marker spheres (mesh entities). Could be replaced or augmented with gizmos:

| Gizmo | What it shows | Implementation |
|-------|---------------|----------------|
| **Cone wireframe** | Spot light cone shape | `gizmos.circle` at base + lines from apex to circle perimeter |
| **Range sphere** | Light effective range | `gizmos.sphere(pos, range, dim_color)` |
| **Direction arrow** | Which way the light points | `gizmos.arrow(pos, pos + dir * length, color)` |
| **Intensity halo** | Brighter gizmo = stronger light | Vary gizmo color alpha by intensity |

#### Spot light cone gizmo

```
fn draw_spot_cone(gizmos: &mut Gizmos, position: Vec3, direction: Vec3, range: f32, outer_angle: f32, color: Color) {
    let base_radius = range * outer_angle.tan();
    let base_center = position + direction * range;

    // Cone base circle
    let rotation = Quat::from_rotation_arc(Vec3::Y, direction);
    gizmos.circle(Isometry3d::new(base_center, rotation), base_radius, color);

    // Apex-to-base lines (8 lines around the circle)
    for i in 0..8 {
        let angle = i as f32 * std::f32::consts::TAU / 8.0;
        let offset = rotation * Vec3::new(base_radius * angle.cos(), 0.0, base_radius * angle.sin());
        gizmos.line(position, base_center + offset, color);
    }
}
```

### Interactive 3D manipulation gizmos

Bevy 0.18 does **not** ship a transform gizmo (translate/rotate/scale handles). Options:

| Option | Maturity | Notes |
|--------|----------|-------|
| `bevy_transform_gizmo` (community) | Moderate | Translate/rotate/scale handles, picking-based. May need porting to 0.18. |
| `bevy_mod_picking` + custom | Solid | Picking is mature; build custom handles on top |
| Roll our own | Full control | Lines + spheres + raycasting. seaview already has `SpatialMesh::cast_ray`. |
| egui overlay | Quick | 2D manipulators overlaid on 3D viewport. Simpler but less immersive. |

**Recommendation:** Start with egui-based numeric inputs (already exist for env objects). Add viewport gizmos for camera waypoints and light positions in Phase 2, using `bevy_gizmos` lines + spheres with `SpatialMesh::cast_ray` for hit testing.

---

## Bezier Curves & Splines

### Which curve type for what?

| Curve | Interpolating? | Smoothness | Best for |
|-------|---------------|------------|----------|
| **Catmull-Rom** (Cardinal, tension=0.5) | Yes — passes through all points | C¹ | Camera paths, light paths — user places waypoints, curve passes through them |
| **Cubic Bezier** | Only endpoints | C² | Easing curves, UI animation — explicit control over shape via handles |
| **Cubic Hermite** | Yes | C¹ | When you know both position AND desired velocity at each point |
| **B-Spline** | No — approximates | C² | Smooth approximation where exact interpolation isn't needed |
| **NURBS** | No — approximates | C² | When you need rational curves (circles, conics) |
| **Linear** | Yes | C⁰ | Quick previews, debug, "connect the dots" |

### Bevy's Curve trait

All the above implement `Curve<T>` where `T: VectorSpace`:

```
trait Curve<T> {
    fn domain(&self) -> Interval;
    fn sample_unchecked(&self, t: f32) -> T;
    fn sample(&self, t: f32) -> Option<T>;
    fn sample_clamped(&self, t: f32) -> T;
}
```

Also `SampleDerivative<T>` for velocity and `SampleTwoDerivatives<T>` for acceleration — useful for:
- Camera velocity → motion blur intensity
- Camera acceleration → smoothness analysis
- Path tangent → orient camera along path direction

### Easing for keyframe interpolation

Bevy provides `CubicSegment::new_bezier_easing(p1, p2)` — a 2D bezier easing function (like CSS `cubic-bezier()`). Maps `[0,1] → [0,1]` with control points.

Common presets:

| Name | Control points | Character |
|------|---------------|-----------|
| Linear | `(0,0), (1,1)` | Constant speed |
| Ease-in | `(0.42, 0), (1, 1)` | Slow start |
| Ease-out | `(0, 0), (0.58, 1)` | Slow end |
| Ease-in-out | `(0.42, 0), (0.58, 1)` | Slow start and end |
| Smooth step | `(0.25, 0.1), (0.25, 1)` | Very gentle |

Easing can be per-segment: different feel between each keyframe pair.

### Curve editing UI

Two levels of complexity:

**Level 1 — Keyframe list (egui panel):**
- Table of keyframes: time, position, look-at
- Add/remove keyframes
- Drag to reorder
- Dropdown for curve type
- Preview button that plays the camera path

**Level 2 — Visual curve editor (viewport):**
- Keyframe positions as draggable spheres in 3D
- Bezier tangent handles as smaller draggable spheres
- Curve preview drawn with `gizmos.curve_3d()`
- Click on curve to insert keyframe
- Right-click keyframe to delete

Level 1 is sufficient for v1. Level 2 is polish.

---

## Proposed Data Model

```
// lib/camera/mod.rs — pure, no Bevy systems

pub enum CameraRigKind {
    Fps,
    Orbit { focus: Vec3, distance: f32, yaw: f32, pitch: f32 },
    Turntable { focus: Vec3, distance: f32, yaw: f32, pitch: f32, auto_speed: f32 },
    Flythrough { path: CameraPath, t: f32 },
}

pub struct CameraPath {
    pub keyframes: Vec<CameraKeyframe>,
    pub curve_type: CurveType,
    pub duration_seconds: f32,
    pub loop_mode: LoopMode,
    pub sync_to_playhead: bool,
}

pub struct CameraKeyframe {
    pub time: f32,         // normalized [0, 1]
    pub position: Vec3,
    pub look_at: Vec3,
    pub easing: EasingPreset,
}

pub enum CurveType { CatmullRom, CubicBezier, CubicHermite, Linear }
pub enum LoopMode { Once, Loop, PingPong }
pub enum EasingPreset { Linear, EaseIn, EaseOut, EaseInOut, Custom([f32; 4]) }

// CameraPath builds Bevy curves on demand:
impl CameraPath {
    pub fn position_curve(&self) -> CubicCardinalSpline<Vec3> { ... }
    pub fn look_at_curve(&self) -> CubicCardinalSpline<Vec3> { ... }
    pub fn sample(&self, t: f32) -> (Vec3, Vec3) { /* (position, look_at) */ }
}
```

This is pure `lib/` code — no systems, no ECS. A thin `app/` system reads the active rig, calls `sample()`, and writes the camera `Transform`.

---

## Implementation Phases

### Phase 1 — Orbit rig + camera rig switching
- `CameraRigKind::Fps` (existing) and `CameraRigKind::Orbit`
- Tab key or UI button to switch modes
- Orbit focus set by Ctrl+Click on mesh (raycast)
- Persist rig type + parameters to `seaview.toml`
- **Effort:** Small. **Impact:** High — most-requested feature for inspection.

### Phase 2 — Camera paths
- `CameraPath` data model with keyframes
- Catmull-Rom interpolation via `CubicCardinalSpline`
- `gizmos.curve_3d()` to visualize path
- `gizmos.sphere()` at each keyframe
- Sync-to-playhead mode
- Egui panel for keyframe editing
- **Effort:** Medium. **Impact:** High — enables presentation-quality output.

### Phase 3 — Light animation
- `LightAnimation` with keyframed intensity/color/position
- Sample curves in `animate_lighting_rig` system
- Day/night cycle preset
- **Effort:** Small (builds on Phase 2 curve infrastructure). **Impact:** Medium.

### Phase 4 — 3D gizmos
- Frustum gizmo for saved cameras
- Cone gizmo for spot lights
- Draggable waypoints for camera paths
- Hit testing via `SpatialMesh::cast_ray`
- **Effort:** Medium. **Impact:** Medium — visual polish.

### Phase 5 — Visual curve editor
- Bezier tangent handles in viewport
- Click-to-insert on curve
- Easing curve editor (2D bezier widget in egui)
- **Effort:** Large. **Impact:** Low — power-user feature.

---

## External References

- Bevy `cubic_splines` source: `vendor/bevy/crates/bevy_math/src/cubic_splines/mod.rs`
- Bevy `Curve` trait: `vendor/bevy/crates/bevy_math/src/curve/mod.rs`
- Bevy gizmos: `vendor/bevy/crates/bevy_gizmos/src/` (lines, arrows, spheres, curves, arcs)
- Bevy retained gizmos: `vendor/bevy/crates/bevy_gizmos/src/retained.rs`
- Existing camera: `crates/seaview/src/app/systems/camera.rs` (FPS only)
- Existing lighting: `crates/seaview/src/lib/lighting/rig.rs` (static)
- Existing spatial queries: `crates/seaview/src/lib/spatial.rs` (ray casting for focus-point picking)
- `bevy_transform_gizmo` (community): translate/rotate/scale handles
- `bevy_mod_picking`: mature ray-based picking for interactive gizmos