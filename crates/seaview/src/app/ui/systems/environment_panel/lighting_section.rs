//! Lighting v2 section for the Environment Panel
//!
//! Renders a grouped accordion UI for the [`LightingRig`] resource:
//!
//! - Per-group collapsible header with name, visibility toggle, delete button
//! - Per-light collapsible row with position, height, intensity, range, cone
//!   angle, color picker, and visibility toggle
//! - "Generate Group" panel with placement config
//! - 2D light map visualization reading from `LightingRig`
//!
//! This module only reads/writes the data model. Bevy `SpotLight` entity
//! wiring is handled separately (seaview-352.5).

use bevy::prelude::*;
use bevy_egui::egui;
use uuid::Uuid;

use crate::lib::lighting::{
    Light, LightGroup, LightGroupGenConfig, LightingRig, PlacementAlgorithm,
};
use crate::lib::mesh_info::MeshDimensions;

// ---------------------------------------------------------------------------
// Generate-group UI state (lives in the panel, not in the domain model)
// ---------------------------------------------------------------------------

/// Transient state for the "Generate Group" inline form.
///
/// Stored externally (in [`GenerateGroupState`]) so it persists across frames
/// while the user is configuring.
pub struct GenerateGroupState {
    pub open: bool,
    pub config: LightGroupGenConfig,
}

impl Default for GenerateGroupState {
    fn default() -> Self {
        Self {
            open: false,
            config: LightGroupGenConfig::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// Light-map interaction state
// ---------------------------------------------------------------------------

/// Persistent state for the 2D light map drag/hover interaction.
///
/// Stored via `egui` data so it survives across frames without a `Local<>`.
#[derive(Clone, Default)]
struct LightMapInteraction {
    /// Light currently being dragged: `(group_id, light_id)`.
    dragged: Option<(Uuid, Uuid)>,
    /// Light currently hovered (closest within hit radius).
    hovered: Option<(Uuid, Uuid)>,
}

/// Snap `value` to the nearest multiple of `grid` when Shift is held.
fn snap(value: f32, grid: f32) -> f32 {
    (value / grid).round() * grid
}

const HIT_RADIUS_PX: f32 = 10.0;
const DOT_RADIUS: f32 = 4.0;
const DOT_RADIUS_HOVER: f32 = 6.0;
const SNAP_GRID: f32 = 5.0;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Renders the full Lighting v2 section inside an existing egui `Ui`.
///
/// Call this from the environment panel system, passing the mutable
/// `LightingRig` and the current mesh dimensions.
pub fn render_lighting_controls(
    ui: &mut egui::Ui,
    rig: &mut LightingRig,
    mesh_dims: &MeshDimensions,
    gen_state: &mut GenerateGroupState,
) {
    // ── Light map (top) ───────────────────────────────────────────────
    render_light_map(ui, rig, mesh_dims, ui.input(|i| i.modifiers));
    ui.add_space(5.0);

    // Summary line
    let total_lights: usize = rig.groups.iter().map(|g| g.lights.len()).sum();
    let visible_count = rig.visible_lights().count();
    ui.label(
        egui::RichText::new(format!(
            "{} group{}, {} light{} ({} visible)",
            rig.groups.len(),
            if rig.groups.len() == 1 { "" } else { "s" },
            total_lights,
            if total_lights == 1 { "" } else { "s" },
            visible_count,
        ))
        .small()
        .color(egui::Color32::from_gray(160)),
    );

    ui.add_space(5.0);

    // ── Per-group accordions ──────────────────────────────────────────
    let mut group_to_remove: Option<uuid::Uuid> = None;

    // We need to iterate by index so we can mutably borrow individual groups
    // without conflicting with the rig borrow for active_group.
    let group_count = rig.groups.len();
    for idx in 0..group_count {
        let group_id = rig.groups[idx].id;
        let is_active = rig.active_group == Some(group_id);

        // Build the header label
        let header_label = format!(
            "{} ({} lights)",
            rig.groups[idx].name,
            rig.groups[idx].lights.len()
        );

        ui.push_id(group_id, |ui| {
            // Horizontal header row: collapsible + eye + delete
            let _header_response =
                egui::CollapsingHeader::new(egui::RichText::new(&header_label).strong())
                    .default_open(is_active)
                    .show(ui, |ui| {
                        render_group_body(ui, &mut rig.groups[idx], mesh_dims);
                    });

            // Controls on the same line as the header (right-aligned)
            // We render them below the header for simplicity in egui
            ui.horizontal(|ui| {
                // Visibility eye toggle
                let vis_label = if rig.groups[idx].visible {
                    "👁"
                } else {
                    "🚫"
                };
                if ui
                    .selectable_label(rig.groups[idx].visible, vis_label)
                    .on_hover_text("Toggle group visibility")
                    .clicked()
                {
                    rig.groups[idx].visible = !rig.groups[idx].visible;
                }

                // Active/expand toggle
                if ui
                    .selectable_label(is_active, "📌")
                    .on_hover_text("Pin this group open")
                    .clicked()
                {
                    rig.active_group = if is_active { None } else { Some(group_id) };
                }

                // Delete group
                if ui.small_button("🗑").on_hover_text("Remove group").clicked() {
                    group_to_remove = Some(group_id);
                }
            });

            ui.add_space(3.0);
            ui.separator();
        });
    }

    // Process deferred removal
    if let Some(id) = group_to_remove {
        rig.remove_group(id);
    }

    ui.add_space(5.0);

    // ── Generate Group panel ──────────────────────────────────────────
    render_generate_panel(ui, rig, mesh_dims, gen_state);
}

// ---------------------------------------------------------------------------
// Group body: per-light editing
// ---------------------------------------------------------------------------

fn render_group_body(ui: &mut egui::Ui, group: &mut LightGroup, mesh_dims: &MeshDimensions) {
    let mut light_to_remove: Option<uuid::Uuid> = None;

    for light_idx in 0..group.lights.len() {
        let light_id = group.lights[light_idx].id;

        ui.push_id(light_id, |ui| {
            let light_header = format!("{}", group.lights[light_idx].name);

            egui::CollapsingHeader::new(&light_header)
                .default_open(false)
                .show(ui, |ui| {
                    render_light_editor(ui, &mut group.lights[light_idx], mesh_dims);
                });

            // Light controls row
            ui.horizontal(|ui| {
                // Visibility toggle
                let vis_label = if group.lights[light_idx].visible {
                    "👁"
                } else {
                    "🚫"
                };
                if ui
                    .selectable_label(group.lights[light_idx].visible, vis_label)
                    .on_hover_text("Toggle light visibility")
                    .clicked()
                {
                    group.lights[light_idx].visible = !group.lights[light_idx].visible;
                }

                // Delete light
                if ui.small_button("🗑").on_hover_text("Remove light").clicked() {
                    light_to_remove = Some(light_id);
                }
            });
        });
    }

    // Process deferred removal
    if let Some(id) = light_to_remove {
        group.remove_light(id);
    }

    // Add single light button
    ui.add_space(3.0);
    if ui.small_button("+ Add Light").clicked() {
        let next_num = group.lights.len() + 1;
        let light = Light::new(format!("Light {}", next_num));
        group.add_light(light);
    }
}

// ---------------------------------------------------------------------------
// Per-light editor
// ---------------------------------------------------------------------------

fn render_light_editor(ui: &mut egui::Ui, light: &mut Light, mesh_dims: &MeshDimensions) {
    // Name (editable)
    ui.horizontal(|ui| {
        ui.label("Name:");
        ui.text_edit_singleline(&mut light.name);
    });

    ui.add_space(2.0);

    // Position (XZ)
    ui.horizontal(|ui| {
        ui.label("Position:");
        ui.add(
            egui::DragValue::new(&mut light.position.x)
                .prefix("X: ")
                .speed(0.5),
        );
        ui.add(
            egui::DragValue::new(&mut light.position.y)
                .prefix("Z: ")
                .speed(0.5),
        );
    });

    ui.add_space(2.0);

    // Height
    let max_height = mesh_dims
        .dimensions
        .map(|d| d.y * 5.0)
        .unwrap_or(500.0)
        .max(10.0);
    ui.add(
        egui::Slider::new(&mut light.height, 1.0..=max_height)
            .text("Height (m)")
            .logarithmic(true),
    );

    // Intensity
    ui.add(
        egui::Slider::new(&mut light.intensity, 1000.0..=10_000_000.0)
            .text("Intensity (lm)")
            .logarithmic(true),
    );

    // Range
    ui.add(
        egui::Slider::new(&mut light.range, 10.0..=5000.0)
            .text("Range (m)")
            .logarithmic(true),
    );

    // Cone angle
    ui.add(egui::Slider::new(&mut light.cone_angle, 5.0..=180.0).text("Cone (°)"));

    // Color
    ui.horizontal(|ui| {
        ui.label("Color:");
        let srgba = light.color.to_srgba();
        let mut color_arr = [srgba.red, srgba.green, srgba.blue];
        if ui.color_edit_button_rgb(&mut color_arr).changed() {
            light.color = Color::srgb(color_arr[0], color_arr[1], color_arr[2]);
        }
    });
}

// ---------------------------------------------------------------------------
// Generate Group panel
// ---------------------------------------------------------------------------

fn render_generate_panel(
    ui: &mut egui::Ui,
    rig: &mut LightingRig,
    mesh_dims: &MeshDimensions,
    state: &mut GenerateGroupState,
) {
    // Toggle button
    let btn_text = if state.open {
        "▾ Generate Group"
    } else {
        "▸ Generate Group"
    };
    if ui.button(btn_text).clicked() {
        state.open = !state.open;
    }

    if !state.open {
        return;
    }

    ui.add_space(3.0);
    egui::Frame::group(ui.style()).show(ui, |ui| {
        ui.label(egui::RichText::new("New Group Configuration").strong());
        ui.add_space(4.0);

        // Placement algorithm
        let current_name = state.config.placement_algorithm.name();
        egui::ComboBox::from_label("Placement")
            .selected_text(current_name)
            .show_ui(ui, |ui| {
                for alg in PlacementAlgorithm::all() {
                    ui.selectable_value(&mut state.config.placement_algorithm, *alg, alg.name());
                }
            });

        // Light count
        ui.add(
            egui::Slider::new(&mut state.config.num_lights, 1..=64)
                .text("Lights")
                .logarithmic(true),
        );

        // Coverage
        ui.add(egui::Slider::new(&mut state.config.coverage_pct, 10.0..=300.0).text("Coverage %"));

        ui.add_space(4.0);

        // Default params (collapsed)
        egui::CollapsingHeader::new("Default Light Params")
            .default_open(false)
            .show(ui, |ui| {
                ui.add(
                    egui::Slider::new(&mut state.config.default_height, 1.0..=500.0)
                        .text("Height (m)")
                        .logarithmic(true),
                );
                ui.add(
                    egui::Slider::new(&mut state.config.default_intensity, 1000.0..=10_000_000.0)
                        .text("Intensity (lm)")
                        .logarithmic(true),
                );
                ui.add(
                    egui::Slider::new(&mut state.config.default_range, 10.0..=5000.0)
                        .text("Range (m)")
                        .logarithmic(true),
                );
                ui.add(
                    egui::Slider::new(&mut state.config.default_cone_angle, 5.0..=180.0)
                        .text("Cone (°)"),
                );
                ui.horizontal(|ui| {
                    ui.label("Color:");
                    let srgba = state.config.default_color.to_srgba();
                    let mut color_arr = [srgba.red, srgba.green, srgba.blue];
                    if ui.color_edit_button_rgb(&mut color_arr).changed() {
                        state.config.default_color =
                            Color::srgb(color_arr[0], color_arr[1], color_arr[2]);
                    }
                });
            });

        ui.add_space(6.0);

        // Generate button
        if ui
            .button(
                egui::RichText::new(format!(
                    "✨ Generate {} lights ({})",
                    state.config.num_lights,
                    state.config.placement_algorithm.name()
                ))
                .strong(),
            )
            .clicked()
        {
            let group =
                state
                    .config
                    .generate_for_mesh_auto_named(rig, mesh_dims.min, mesh_dims.max);
            rig.add_group(group);
            state.open = false;
        }
    });
}

// ── Light Map Visualization ────────────────────────────────────────────────

/// Renders a top-down light map showing the domain AABB, coverage boundaries,
/// cone footprints, and light positions — reading from the v2 [`LightingRig`].
fn render_light_map(
    ui: &mut egui::Ui,
    rig: &mut LightingRig,
    mesh_dims: &MeshDimensions,
    modifiers: egui::Modifiers,
) {
    // ------------------------------------------------------------------
    // 1. Compute view bounds (before any mutation this frame)
    // ------------------------------------------------------------------
    let (domain_min, domain_max) = if let (Some(mn), Some(mx)) = (mesh_dims.min, mesh_dims.max) {
        (
            bevy::math::Vec2::new(mn.x, mn.z),
            bevy::math::Vec2::new(mx.x, mx.z),
        )
    } else {
        (
            bevy::math::Vec2::new(-50.0, -50.0),
            bevy::math::Vec2::new(50.0, 50.0),
        )
    };

    let mut view_min = domain_min;
    let mut view_max = domain_max;
    for (_group, light) in rig.all_lights() {
        view_min = bevy::math::Vec2::new(
            view_min.x.min(light.position.x),
            view_min.y.min(light.position.y),
        );
        view_max = bevy::math::Vec2::new(
            view_max.x.max(light.position.x),
            view_max.y.max(light.position.y),
        );
    }

    let view_extent = view_max - view_min;
    let padding_frac = 0.12;
    let pad = bevy::math::Vec2::new(
        view_extent.x * padding_frac + 5.0,
        view_extent.y * padding_frac + 5.0,
    );
    let view_min = view_min - pad;
    let view_max = view_max + pad;
    let view_w = (view_max.x - view_min.x).max(1.0);
    let view_h = (view_max.y - view_min.y).max(1.0);

    // ------------------------------------------------------------------
    // 2. Allocate interactive rect
    // ------------------------------------------------------------------
    let map_size = ui.available_width().min(280.0);
    let (rect, response) =
        ui.allocate_exact_size(egui::Vec2::splat(map_size), egui::Sense::click_and_drag());

    // Coordinate transforms
    let to_px = |wx: f32, wz: f32| -> egui::Pos2 {
        egui::pos2(
            rect.min.x + ((wx - view_min.x) / view_w) * map_size,
            rect.min.y + ((wz - view_min.y) / view_h) * map_size,
        )
    };
    let from_px = |px: f32, py: f32| -> bevy::math::Vec2 {
        bevy::math::Vec2::new(
            view_min.x + ((px - rect.min.x) / map_size) * view_w,
            view_min.y + ((py - rect.min.y) / map_size) * view_h,
        )
    };

    // ------------------------------------------------------------------
    // 3. Load / initialise interaction state from egui memory
    // ------------------------------------------------------------------
    let map_id = response.id;
    let mut istate: LightMapInteraction = ui
        .data(|d| d.get_temp::<LightMapInteraction>(map_id))
        .unwrap_or_default();

    // ------------------------------------------------------------------
    // 4. Hit-test: find closest light to pointer
    // ------------------------------------------------------------------
    // Build a flat list of (group_id, light_id, screen_pos) for hit testing.
    struct LightHit {
        group_id: Uuid,
        light_id: Uuid,
        screen: egui::Pos2,
    }
    let mut hits: Vec<LightHit> = Vec::new();
    for group in rig.groups.iter() {
        for light in &group.lights {
            let sp = to_px(light.position.x, light.position.y);
            hits.push(LightHit {
                group_id: group.id,
                light_id: light.id,
                screen: sp,
            });
        }
    }

    // Find the light nearest to the current pointer, within HIT_RADIUS_PX.
    let pointer_pos = response.hover_pos();
    let nearest: Option<(Uuid, Uuid, f32)> = pointer_pos.and_then(|pp| {
        hits.iter()
            .map(|h| {
                let d = h.screen.distance(pp);
                (h.group_id, h.light_id, d)
            })
            .filter(|(_, _, d)| *d <= HIT_RADIUS_PX)
            .min_by(|a, b| a.2.partial_cmp(&b.2).unwrap())
    });

    // Update hover
    istate.hovered = nearest.map(|(gid, lid, _)| (gid, lid));

    // ------------------------------------------------------------------
    // 5. Interaction: ctrl-click toggles visibility, drag repositions
    // ------------------------------------------------------------------

    // Ctrl-click → toggle per-light visibility
    if response.clicked() && modifiers.command {
        if let Some((gid, lid)) = istate.hovered {
            rig.toggle_light_visibility(gid, lid);
        }
    }

    // Drag start → latch the hovered light
    if response.drag_started() && !modifiers.command {
        istate.dragged = istate.hovered;
    }

    // Dragging → move the light
    if response.dragged() {
        if let Some((gid, lid)) = istate.dragged {
            if let Some(pp) = pointer_pos {
                let mut world = from_px(pp.x, pp.y);
                if modifiers.shift {
                    world.x = snap(world.x, SNAP_GRID);
                    world.y = snap(world.y, SNAP_GRID);
                }
                if let Some(group) = rig.group_mut(gid) {
                    if let Some(light) = group.light_mut(lid) {
                        light.position = world;
                    }
                }
            }
        }
    }

    // Drag end → release
    if response.drag_stopped() {
        istate.dragged = None;
    }

    // ------------------------------------------------------------------
    // 6. Cursor feedback
    // ------------------------------------------------------------------
    if istate.dragged.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grabbing);
    } else if istate.hovered.is_some() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::Grab);
    }

    // ------------------------------------------------------------------
    // 7. Store interaction state back into egui memory
    // ------------------------------------------------------------------
    ui.data_mut(|d| d.insert_temp(map_id, istate.clone()));

    // ------------------------------------------------------------------
    // 8. Paint
    // ------------------------------------------------------------------
    if !ui.is_rect_visible(rect) {
        return;
    }

    let painter = ui.painter_at(rect);

    // Background
    painter.rect_filled(rect, 4.0, egui::Color32::from_gray(20));

    // --- Domain AABB ---
    let dom_tl = to_px(domain_min.x, domain_min.y);
    let dom_br = to_px(domain_max.x, domain_max.y);
    let dom_rect = egui::Rect::from_min_max(dom_tl, dom_br);
    painter.rect_filled(
        dom_rect,
        1.0,
        egui::Color32::from_rgba_unmultiplied(180, 180, 180, 15),
    );
    painter.rect_stroke(
        dom_rect,
        1.0,
        egui::Stroke::new(
            1.5,
            egui::Color32::from_rgba_unmultiplied(200, 200, 200, 200),
        ),
        egui::StrokeKind::Outside,
    );
    if let Some(dims) = mesh_dims.dimensions {
        painter.text(
            egui::pos2(dom_br.x - 3.0, dom_br.y - 2.0),
            egui::Align2::RIGHT_BOTTOM,
            format!("Domain {:.0}×{:.0}", dims.x, dims.z),
            egui::FontId::proportional(7.5),
            egui::Color32::from_rgba_unmultiplied(200, 200, 200, 160),
        );
    }

    // --- Light dots and cone projections ---
    let px_per_unit = map_size / ((view_w + view_h) * 0.5);

    let group_palette: &[egui::Color32] = &[
        egui::Color32::from_rgb(255, 200, 80),
        egui::Color32::from_rgb(100, 180, 255),
        egui::Color32::from_rgb(180, 120, 255),
        egui::Color32::from_rgb(120, 255, 160),
        egui::Color32::from_rgb(255, 120, 120),
        egui::Color32::from_rgb(255, 160, 220),
    ];

    for (group_idx, group) in rig.groups.iter().enumerate() {
        let palette_color = group_palette[group_idx % group_palette.len()];

        for light in &group.lights {
            let is_effective_visible = group.visible && light.visible;
            let alpha_mult: f32 = if is_effective_visible { 1.0 } else { 0.25 };
            let is_hovered = istate.hovered == Some((group.id, light.id));
            let is_dragged = istate.dragged == Some((group.id, light.id));

            let center = to_px(light.position.x, light.position.y);

            // Cone ground radius
            let cone_world_radius = light.height * (light.cone_angle / 2.0).to_radians().tan();
            let cone_px = (cone_world_radius * px_per_unit).min(map_size * 0.4);

            // Glow circle
            let glow_alpha = (25.0 * alpha_mult) as u8;
            painter.circle_filled(
                center,
                cone_px,
                egui::Color32::from_rgba_unmultiplied(
                    palette_color.r(),
                    palette_color.g(),
                    palette_color.b(),
                    glow_alpha,
                ),
            );

            // Cone edge ring
            painter.circle_stroke(
                center,
                cone_px,
                egui::Stroke::new(
                    0.5,
                    egui::Color32::from_rgba_unmultiplied(
                        palette_color.r(),
                        palette_color.g(),
                        palette_color.b(),
                        (50.0 * alpha_mult) as u8,
                    ),
                ),
            );

            // Light dot — size and style vary with hover/drag/visibility
            let dot_radius = if is_hovered || is_dragged {
                DOT_RADIUS_HOVER
            } else {
                DOT_RADIUS
            };
            let dot_alpha = (255.0 * alpha_mult) as u8;
            let dot_color = egui::Color32::from_rgba_unmultiplied(
                palette_color.r(),
                palette_color.g(),
                palette_color.b(),
                dot_alpha,
            );

            if is_effective_visible {
                // Solid dot for visible lights
                painter.circle_filled(center, dot_radius, dot_color);
            } else {
                // Hollow ring for hidden lights
                painter.circle_stroke(center, dot_radius, egui::Stroke::new(1.5, dot_color));
            }

            // Hover highlight ring
            if is_hovered || is_dragged {
                painter.circle_stroke(
                    center,
                    dot_radius + 3.0,
                    egui::Stroke::new(
                        1.0,
                        egui::Color32::from_rgba_unmultiplied(255, 255, 255, 140),
                    ),
                );
            }

            // Light name label on hover
            if is_hovered && !is_dragged {
                painter.text(
                    egui::pos2(center.x, center.y - dot_radius - 4.0),
                    egui::Align2::CENTER_BOTTOM,
                    &light.name,
                    egui::FontId::proportional(8.0),
                    egui::Color32::from_rgba_unmultiplied(255, 255, 255, 200),
                );
            }
        }
    }

    // Border
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, egui::Color32::from_gray(50)),
        egui::StrokeKind::Outside,
    );

    // Interaction hint below the map
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new("Drag to move · Ctrl-click to toggle · Shift snaps to grid")
            .small()
            .color(egui::Color32::from_gray(100)),
    );
}
