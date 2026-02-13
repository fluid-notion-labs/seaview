# HUD Overlays — Research & Ideas

> **Context**: Seaview is a simulation output viewer. Overlays should give
> at-a-glance info without cluttering the 3D viewport — think game HUD, not
> sidebar panels.

---

## Core Overlays

### Domain Info (top-left corner)
- Bounding box dimensions: W × D × H (metres)
- Domain min/max coordinates
- Cell/vertex count (when available)
- Coordinate system indicator (Y-up, Z-up, etc.)

### Playback Info (already in bottom bar, but could augment)
- Current timestep / physical time (not just frame index)
- Simulation time per frame (dt)
- Wall-clock playback rate vs real-time ratio

### Camera Info (top-right corner, subtle)
- Position `(x, y, z)`
- Active camera preset name
- Distance to mesh centre

### Performance (bottom-right corner, toggleable)
- FPS / frame time
- GPU memory estimate
- Spawned entity count / frame window

---

## ParaView Features Worth Stealing

ParaView is the gold standard for scientific vis. These are the overlay/annotation
features most relevant to a simulation playback viewer:

| ParaView Feature | What It Does | Seaview Equivalent |
|---|---|---|
| **Orientation Axes** | Small 3D axis widget in corner showing X/Y/Z orientation | Gizmo overlay — Bevy has `GizmoConfig` |
| **Color Legend / Scalar Bar** | Maps colour to data range (e.g. velocity magnitude) | Future: when we support scalar fields |
| **Time Annotation** | Shows current simulation time on viewport | Overlay: physical time + timestep |
| **Cube Axes / Bounding Box** | 3D wireframe box with axis labels and tick marks around the domain | Wireframe AABB with dimension labels |
| **Text Annotation** | Arbitrary text pinned to viewport position | Generic overlay text system |
| **Scale Bar** | Physical-units ruler in the viewport | Useful for screenshots — "10m" bar |
| **Center of Rotation Marker** | Shows the orbit pivot point | Dot at look-at target |
| **Data Range Display** | Min/max of active scalar field | Future: scalar field stats |
| **Camera Link / Sync** | Multiple viewports with linked cameras | Out of scope for now |

### High-value, low-effort picks

1. **Orientation axes gizmo** — Bevy supports this natively, just needs wiring up.
2. **Wireframe bounding box** — We already compute `MeshDimensions`; draw it as a gizmo with corner labels.
3. **Scale bar** — Simple line + text in screen space. Essential for screenshots.
4. **Time annotation** — We have frame index; just need to map to physical time if dt is known.

---

## Implementation Sketch

```text
OverlayPlugin
├── OverlayConfig (Resource)
│   ├── show_domain_info: bool
│   ├── show_camera_info: bool
│   ├── show_performance: bool
│   ├── show_orientation_gizmo: bool
│   ├── show_bounding_box: bool
│   └── show_scale_bar: bool
│
├── Systems (Update)
│   ├── render_domain_overlay      → egui, top-left
│   ├── render_camera_overlay      → egui, top-right
│   ├── render_perf_overlay        → egui, bottom-right
│   ├── render_orientation_gizmo   → Bevy gizmos
│   ├── render_bounding_box        → Bevy gizmos (3D wireframe)
│   └── render_scale_bar           → egui or gizmos
│
└── View menu toggles for each
```

Overlays should be:
- **Semi-transparent** background so they don't obscure the scene
- **Small, fixed-position** — anchored to viewport corners
- **Togglable** from the View menu (and eventually hotkeys)
- **Not in the sidebar** — overlays live on top of the 3D viewport

---

## Open Questions

- **Physical time**: Do we have `dt` from the simulation metadata? If not, frame index is all we can show.
- **Scalar fields**: No scalar data yet (just geometry). Colour legends etc. are future work.
- **Screenshot mode**: Should overlays be included in screenshots, or should there be a "clean" mode?
- **Multiple viewports**: Not planned, but the overlay system should be viewport-aware if we ever go there.