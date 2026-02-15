# Seaview UI Element Reference

Visual map of every named UI element in the application, organised by region.

---

## Layout Overview

```
┌──────────────────────────────────────────────────────────┐
│  Menu Bar  (top)                                         │
├────────────┬─────────────────────────────────────────────┤
│            │                                             │
│ Environment│            3D Viewport                      │
│   Panel    │                                             │
│  (left     │                                             │
│  sidebar)  │                                    Toasts ──┤
│            │                                  (top-right)│
│            │                                             │
├────────────┴─────────────────────────────────────────────┤
│  Playback Controls  (bottom panel)                       │
│  ┌─────────────────────────────────────────────────────┐ │
│  │ Transport Row                                       │ │
│  │ Timeline Bar                                        │ │
│  │ Load Section Bar                                    │ │
│  └─────────────────────────────────────────────────────┘ │
├──────────────────────────────────────────────────────────┤
│  Status Bar  (very bottom)                               │
└──────────────────────────────────────────────────────────┘
```

---

## 1. Menu Bar

**egui panel id:** `"menu_bar"`
**Type:** `TopBottomPanel::top`
**Source:** `app/ui/systems/menu_bar.rs`

| Element | Type | Description |
|---------|------|-------------|
| **File** menu | `menu_button` | Top-level menu |
| ├ Open File… | `button` | File picker (stub) |
| ├ Open Directory… | `button` | Directory picker (stub) |
| └ Exit | `button` | Quit the application |
| **View** menu | `menu_button` | Toggle panel visibility |
| ├ Environment | `checkbox` | Toggle environment panel |
| ├ Playback Controls | `checkbox` | Toggle playback panel |
| └ Reset Layout | `button` | Reset all panel sizes & visibility |
| **Help** menu | `menu_button` | Info & shortcuts |
| ├ Controls | `button` | Shows camera control reference |
| └ About Seaview | `button` | Version info |

**Keyboard shortcuts (global, egui-level):**
- `Ctrl+S` — Save view
- `Ctrl+Shift+W` — Toggle wireframe overlay

---

## 2. Environment Panel (Left Sidebar)

**egui panel id:** `"environment_panel"`
**Type:** `SidePanel::left`
**Source:** `app/ui/systems/environment_panel/mod.rs`
**Width:** 200–500 px (resizable, default 300)

Contains a vertical `ScrollArea` with collapsible accordion sections:

### 2.1 Environment Light

**Header:** `"Environment Light"`
**Source:** `environment_panel/environment_light_section.rs`
**State:** `UiState.collapsible.environment_light_open`

Controls for global scene lighting (directional + ambient). Independent of the spot-light rig.

### 2.2 Lights (Lighting Rig v2)

**Header:** `"Lights"`
**Source:** `environment_panel/lighting_section.rs`
**State:** `UiState.collapsible.lighting_rig_open`

| Element | Description |
|---------|-------------|
| Per-group collapsible header | Group name, visibility toggle, delete button |
| Per-light collapsible row | Position, height, intensity, range, cone angle, color picker, visibility toggle |
| Generate Group panel | Placement algorithm config |
| 2D Light Map | Interactive visualisation — drag to reposition, Ctrl+click visibility toggle, Shift snap-to-grid (5 m) |

### 2.3 Material

**Header:** `"Material"`
**Source:** `environment_panel/material_section.rs`
**State:** `UiState.collapsible.material_open`

Controls for mesh material properties: base color, opacity, roughness, metallic, reflectance, emissive color/intensity, double-sided toggle.

### 2.4 Rendering

**Header:** `"Rendering"`
**Source:** `environment_panel/rendering_section.rs`
**State:** `UiState.collapsible.rendering_open`

Wireframe overlay toggle, frame window size, spawned entity statistics.

### 2.5 Performance

**Header:** `"Performance"`
**Source:** `environment_panel/perf_section.rs`
**State:** `UiState.collapsible.perf_open`

Real-time FPS, frame time statistics, rendering stats (vertices, triangles, mesh counts), entity counts.

---

## 3. Playback Controls (Bottom Panel)

**egui panel id:** `"playback_panel"`
**Type:** `TopBottomPanel::bottom`
**Source:** `app/ui/systems/playback_controls/mod.rs`
**Height:** 100 px (fixed)

Three stacked rows, top to bottom:

### 3.1 Transport Row

**Source:** `playback_controls/transport.rs`

| Element | Icon / Label | Description |
|---------|-------------|-------------|
| Previous Frame | `⏮` | Step back one frame |
| Play / Pause | `▶` / `⏸` | Toggle playback |
| Next Frame | `⏭` | Step forward one frame |
| Center Camera | `🎯` | Center camera on mesh |
| Save View | `💾` | Save camera + environment to `seaview.toml` |
| Camera Selector | `📷` dropdown | Switch between named camera presets (`"camera_selector"`) |
| Save Camera As | `📷+` button + popup | Save current camera under a new name |
| Frame Counter | `Frame: N / M` | Current / total frame display |
| Speed Slider | `Speed: 0.1x – 5.0x` | Playback speed multiplier |
| Loop | checkbox | Toggle loop-at-end |
| Watch | checkbox (`👁 Watch`) | Watch directory for new frames |
| FPS | right-aligned label | Frames per second |
| Time | right-aligned label | `Time: Xs / Ys` (at 30 fps) |

### 3.2 Timeline Bar

**Source:** `playback_controls/timeline_bar.rs`

| Element | Description |
|---------|-------------|
| Background track | Dark grey bar with 10% frame markers |
| Progress fill | Blue fill from left edge to current frame |
| Scrubber handle | Orange/yellow draggable knob with grip lines. Click or drag to scrub. |

### 3.3 Load Section Bar

**Source:** `playback_controls/load_section_bar.rs`

| Element | Description |
|---------|-------------|
| Section regions | Colour-coded highlighted ranges (6-colour cycling palette: green, blue, orange, purple, olive, teal) |
| Edge handles | Draggable start/end edges per section |
| Body drag | Drag section body to move without resizing |
| Ctrl+Drag (empty) | Paint a new section |
| Right-click (section) | Remove a section |
| Section count label | Small count at right edge |

---

## 4. Status Bar

**egui panel id:** `"status_bar"`
**Type:** `TopBottomPanel::bottom`
**Source:** `app/ui/systems/status_bar.rs`
**Height:** 24 px (fixed)

| Element | Description |
|---------|-------------|
| Loading indicator | Progress bar + `"Loading <name>: N/M (P%)"` during load |
| Loaded status | `"✓ N frames loaded"` when complete |
| Ready label | `"Ready"` when idle |
| History button | `📋` (right-aligned) — toggles Message History window |

### 4.1 Message History Window

**egui window:** `"Message History"`
**Anchor:** bottom-right corner

| Element | Description |
|---------|-------------|
| Message count | `"N messages"` label |
| Clear button | Clears all history |
| Scrollable list | Newest-first, each entry has level icon + message text |

---

## 5. Toast Overlay

**Source:** `app/ui/systems/toast_overlay.rs`
**Position:** Top-right corner, stacked vertically
**egui id:** `"toast_overlay"` (per-toast indexed)

| Element | Description |
|---------|-------------|
| Toast card | Rounded frame with level-coloured accent border |
| Level icon | ℹ️ / ✅ / ⚠️ / ❌ per level |
| Message text | Body content |
| Dismiss button | `×` on right side; clicking anywhere also dismisses |

**Toast levels & colours:**

| Level | Background | Accent |
|-------|-----------|--------|
| Info | Dark blue | `rgb(80, 160, 255)` |
| Success | Dark green | `rgb(80, 200, 120)` |
| Warning | Dark amber | `rgb(240, 180, 50)` |
| Error | Dark red | `rgb(240, 80, 80)` |

Toasts auto-dismiss after their duration. They fade out during the last 20% of their lifetime.

---

## 6. Modal Dialogs

**Source:** `app/ui/systems/menu_bar.rs` (`message_display_system`)

| Dialog | egui window | Trigger |
|--------|------------|---------|
| Error dialog | `"Error"` | `UiState.temp_state.error_message` |
| Information dialog | `"Information"` | `UiState.temp_state.info_message` |

Both are non-collapsible, non-resizable, anchored to centre-top with an OK button.

---

## 7. 3D Viewport

The central area — everything not claimed by the egui panels. Contains:

- Bevy `Camera3d` with `FpsCamera` controller
- Mesh entities (frame window)
- `SpotLight` entities synced from `LightingRig`
- Light marker meshes (debug dots with `RigLightMarker`)

No egui panels are drawn here; this is pure Bevy rendering.

---

## State Cross-Reference

| UiState field | Controls |
|---------------|----------|
| `show_environment_panel` | Environment panel visibility |
| `show_playback_controls` | Playback panel visibility |
| `show_network_panel` | (reserved, not currently rendered) |
| `show_perf_panel` | (reserved, not currently rendered) |
| `playback.*` | Mirror of `Timeline` + `SequenceManager` |
| `loading.*` | Mirror of `SequenceAssets` |
| `panel_sizes.*` | Persisted widths/heights |
| `collapsible.*` | Accordion open/closed per section |
| `load_section_dragging` | Active drag edge in load section bar |
| `toasts` | Toast queue (active + history) |
| `toast_clock` | Monotonic time for toast expiry |
| `temp_state.*` | Modal dialog state |