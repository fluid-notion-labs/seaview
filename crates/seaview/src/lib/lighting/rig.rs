//! Lighting v2 data model
//!
//! Pure data types for the named-group lighting rig. No ECS queries, no system
//! signatures — everything here is testable without a running Bevy app.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

use super::placement::PlacementAlgorithm;

// ---------------------------------------------------------------------------
// Light
// ---------------------------------------------------------------------------

/// A single light source with fully editable parameters.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Light {
    /// Unique identifier.
    pub id: Uuid,

    /// Human-readable name (auto-generated, editable).
    pub name: String,

    /// Position on the XZ plane (metres).
    pub position: Vec2,

    /// Height above sea level (metres).
    pub height: f32,

    /// Luminous flux in **lumens** (Bevy's native unit for spot/point lights).
    pub intensity: f32,

    /// Maximum effective distance (metres).
    pub range: f32,

    /// Full cone angle in **degrees**. Halved internally for Bevy's
    /// `SpotLight::outer_angle`.
    pub cone_angle: f32,

    /// Light colour.
    pub color: Color,

    /// Per-light visibility toggle.
    pub visible: bool,
}

impl Light {
    /// Create a new light with a fresh UUID and default name.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            position: Vec2::ZERO,
            height: 50.0,
            intensity: 800_000.0,
            range: 500.0,
            cone_angle: 60.0,
            color: Color::srgb(1.0, 0.95, 0.9),
            visible: true,
        }
    }

    /// Convenience builder: set position and return self.
    pub fn with_position(mut self, position: Vec2) -> Self {
        self.position = position;
        self
    }
}

// ---------------------------------------------------------------------------
// LightGroup
// ---------------------------------------------------------------------------

/// A named collection of [`Light`]s with a group-level visibility toggle.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LightGroup {
    /// Unique identifier.
    pub id: Uuid,

    /// Human-readable name (defaults to the generation config description,
    /// e.g. "Uniform Grid #1").
    pub name: String,

    /// Group-level visibility toggle. When `false`, all lights in the group
    /// are hidden regardless of their individual `visible` flag.
    pub visible: bool,

    /// The lights belonging to this group.
    pub lights: Vec<Light>,
}

impl LightGroup {
    /// Create a new, empty group with a fresh UUID.
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: name.into(),
            visible: true,
            lights: Vec::new(),
        }
    }

    /// Add a light to this group.
    pub fn add_light(&mut self, light: Light) {
        self.lights.push(light);
    }

    /// Remove a light by id. Returns the removed light, or `None` if not
    /// found.
    pub fn remove_light(&mut self, light_id: Uuid) -> Option<Light> {
        if let Some(idx) = self.lights.iter().position(|l| l.id == light_id) {
            Some(self.lights.remove(idx))
        } else {
            None
        }
    }

    /// Toggle visibility for a single light inside this group.
    /// Returns `true` if the light was found.
    pub fn toggle_light_visibility(&mut self, light_id: Uuid) -> bool {
        if let Some(light) = self.lights.iter_mut().find(|l| l.id == light_id) {
            light.visible = !light.visible;
            true
        } else {
            false
        }
    }

    /// Look up a light by id.
    pub fn light(&self, light_id: Uuid) -> Option<&Light> {
        self.lights.iter().find(|l| l.id == light_id)
    }

    /// Look up a light by id (mutable).
    pub fn light_mut(&mut self, light_id: Uuid) -> Option<&mut Light> {
        self.lights.iter_mut().find(|l| l.id == light_id)
    }
}

// ---------------------------------------------------------------------------
// LightGroupGenConfig
// ---------------------------------------------------------------------------

/// Configuration used **only at generation time** to create a new
/// [`LightGroup`] with an initial set of lights. Not stored per-light after
/// creation.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LightGroupGenConfig {
    /// Algorithm used to distribute lights across the placement area.
    pub placement_algorithm: PlacementAlgorithm,

    /// Number of lights to generate.
    pub num_lights: usize,

    /// Coverage percentage of mesh XZ extent (100 = exactly covers the mesh
    /// footprint, 150 = extends 50 % beyond, etc.).
    pub coverage_pct: f32,

    // -- Default parameters applied to each generated light ----------------
    /// Default height (metres).
    pub default_height: f32,

    /// Default cone angle (full angle, degrees).
    pub default_cone_angle: f32,

    /// Default intensity (lumens).
    pub default_intensity: f32,

    /// Default range (metres).
    pub default_range: f32,

    /// Default colour.
    pub default_color: Color,
}

impl Default for LightGroupGenConfig {
    fn default() -> Self {
        Self {
            placement_algorithm: PlacementAlgorithm::UniformGrid,
            num_lights: 9,
            coverage_pct: 120.0,
            default_height: 50.0,
            default_cone_angle: 60.0,
            default_intensity: 800_000.0,
            default_range: 500.0,
            default_color: Color::srgb(1.0, 0.95, 0.9),
        }
    }
}

impl LightGroupGenConfig {
    /// Generate a [`LightGroup`] by placing lights according to this config.
    ///
    /// `bounds_min` / `bounds_max` describe the XZ area that has already been
    /// scaled by `coverage_pct` (the caller is responsible for computing
    /// placement bounds from mesh dimensions, just like the v1 system).
    pub fn generate(
        &self,
        group_name: impl Into<String>,
        bounds_min: Vec2,
        bounds_max: Vec2,
    ) -> LightGroup {
        let positions =
            self.placement_algorithm
                .calculate_positions(self.num_lights, bounds_min, bounds_max);

        let lights: Vec<Light> = positions
            .into_iter()
            .enumerate()
            .map(|(i, pos)| Light {
                id: Uuid::new_v4(),
                name: format!("Light {}", i + 1),
                position: pos,
                height: self.default_height,
                intensity: self.default_intensity,
                range: self.default_range,
                cone_angle: self.default_cone_angle,
                color: self.default_color,
                visible: true,
            })
            .collect();

        let mut group = LightGroup::new(group_name);
        group.lights = lights;
        group
    }
}

// ---------------------------------------------------------------------------
// LightingRig
// ---------------------------------------------------------------------------

/// Top-level resource holding every lighting group.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize, Resource)]
pub struct LightingRig {
    /// All lighting groups in the rig.
    pub groups: Vec<LightGroup>,

    /// Which group's accordion panel is currently expanded in the UI
    /// (`None` = all collapsed).
    pub active_group: Option<Uuid>,
}

impl Default for LightingRig {
    fn default() -> Self {
        Self {
            groups: Vec::new(),
            active_group: None,
        }
    }
}

impl LightingRig {
    /// Create an empty rig.
    pub fn new() -> Self {
        Self::default()
    }

    // -- Group operations --------------------------------------------------

    /// Add a group to the rig. Returns the group's id.
    pub fn add_group(&mut self, group: LightGroup) -> Uuid {
        let id = group.id;
        self.groups.push(group);
        id
    }

    /// Remove a group by id. Returns the removed group, or `None` if not
    /// found. If the removed group was the `active_group`, the active
    /// selection is cleared.
    pub fn remove_group(&mut self, group_id: Uuid) -> Option<LightGroup> {
        if let Some(idx) = self.groups.iter().position(|g| g.id == group_id) {
            if self.active_group == Some(group_id) {
                self.active_group = None;
            }
            Some(self.groups.remove(idx))
        } else {
            None
        }
    }

    /// Toggle the visibility flag of a group.
    /// Returns `true` if the group was found.
    pub fn toggle_group_visibility(&mut self, group_id: Uuid) -> bool {
        if let Some(group) = self.group_mut(group_id) {
            group.visible = !group.visible;
            true
        } else {
            false
        }
    }

    /// Look up a group by id.
    pub fn group(&self, group_id: Uuid) -> Option<&LightGroup> {
        self.groups.iter().find(|g| g.id == group_id)
    }

    /// Look up a group by id (mutable).
    pub fn group_mut(&mut self, group_id: Uuid) -> Option<&mut LightGroup> {
        self.groups.iter_mut().find(|g| g.id == group_id)
    }

    // -- Light operations (convenience delegation) -------------------------

    /// Add a light to the group identified by `group_id`.
    /// Returns `true` if the group was found and the light was added.
    pub fn add_light(&mut self, group_id: Uuid, light: Light) -> bool {
        if let Some(group) = self.group_mut(group_id) {
            group.add_light(light);
            true
        } else {
            false
        }
    }

    /// Remove a light from the group identified by `group_id`.
    /// Returns the removed [`Light`], or `None` if group or light was not
    /// found.
    pub fn remove_light(&mut self, group_id: Uuid, light_id: Uuid) -> Option<Light> {
        self.group_mut(group_id)
            .and_then(|g| g.remove_light(light_id))
    }

    /// Toggle visibility for a single light.
    /// Returns `true` if the group and light were both found.
    pub fn toggle_light_visibility(&mut self, group_id: Uuid, light_id: Uuid) -> bool {
        self.group_mut(group_id)
            .map_or(false, |g| g.toggle_light_visibility(light_id))
    }

    // -- Iteration helpers -------------------------------------------------

    /// Iterate over all lights across all groups, yielding `(&LightGroup,
    /// &Light)` pairs. Useful for the rendering layer.
    pub fn all_lights(&self) -> impl Iterator<Item = (&LightGroup, &Light)> {
        self.groups
            .iter()
            .flat_map(|g| g.lights.iter().map(move |l| (g, l)))
    }

    /// Iterate over all **visible** lights (group visible AND light visible).
    pub fn visible_lights(&self) -> impl Iterator<Item = (&LightGroup, &Light)> {
        self.all_lights().filter(|(g, l)| g.visible && l.visible)
    }

    /// Total number of lights across all groups.
    pub fn light_count(&self) -> usize {
        self.groups.iter().map(|g| g.lights.len()).sum()
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- Helpers -----------------------------------------------------------

    fn make_light(name: &str) -> Light {
        Light::new(name)
    }

    fn make_group(name: &str, n_lights: usize) -> LightGroup {
        let mut g = LightGroup::new(name);
        for i in 0..n_lights {
            g.add_light(make_light(&format!("L{}", i + 1)));
        }
        g
    }

    // -- Light tests -------------------------------------------------------

    #[test]
    fn light_new_has_unique_id() {
        let a = Light::new("A");
        let b = Light::new("B");
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn light_defaults_are_sane() {
        let l = Light::new("Test");
        assert_eq!(l.name, "Test");
        assert!(l.visible);
        assert!(l.intensity > 0.0);
        assert!(l.range > 0.0);
        assert!(l.height > 0.0);
    }

    #[test]
    fn light_with_position() {
        let l = Light::new("A").with_position(Vec2::new(10.0, 20.0));
        assert_eq!(l.position, Vec2::new(10.0, 20.0));
    }

    // -- LightGroup tests --------------------------------------------------

    #[test]
    fn group_add_and_remove_light() {
        let mut g = LightGroup::new("G");
        let l = make_light("L1");
        let lid = l.id;

        g.add_light(l);
        assert_eq!(g.lights.len(), 1);

        let removed = g.remove_light(lid);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().id, lid);
        assert!(g.lights.is_empty());
    }

    #[test]
    fn group_remove_nonexistent_light_returns_none() {
        let mut g = LightGroup::new("G");
        assert!(g.remove_light(Uuid::new_v4()).is_none());
    }

    #[test]
    fn group_toggle_light_visibility() {
        let mut g = LightGroup::new("G");
        let l = make_light("L1");
        let lid = l.id;
        g.add_light(l);

        assert!(g.lights[0].visible);
        assert!(g.toggle_light_visibility(lid));
        assert!(!g.lights[0].visible);
        assert!(g.toggle_light_visibility(lid));
        assert!(g.lights[0].visible);
    }

    #[test]
    fn group_toggle_missing_light_returns_false() {
        let mut g = LightGroup::new("G");
        assert!(!g.toggle_light_visibility(Uuid::new_v4()));
    }

    #[test]
    fn group_light_lookup() {
        let mut g = LightGroup::new("G");
        let l = make_light("L1");
        let lid = l.id;
        g.add_light(l);

        assert!(g.light(lid).is_some());
        assert_eq!(g.light(lid).unwrap().name, "L1");
        assert!(g.light(Uuid::new_v4()).is_none());
    }

    // -- LightGroupGenConfig tests -----------------------------------------

    #[test]
    fn gen_config_default_is_sane() {
        let cfg = LightGroupGenConfig::default();
        assert!(cfg.num_lights > 0);
        assert!(cfg.coverage_pct > 0.0);
        assert!(cfg.default_intensity > 0.0);
    }

    #[test]
    fn gen_config_generates_correct_count() {
        let cfg = LightGroupGenConfig {
            num_lights: 4,
            ..Default::default()
        };
        let group = cfg.generate("Test", Vec2::ZERO, Vec2::new(100.0, 100.0));
        assert_eq!(group.lights.len(), 4);
        assert_eq!(group.name, "Test");
        assert!(group.visible);
    }

    #[test]
    fn gen_config_applies_defaults_to_lights() {
        let cfg = LightGroupGenConfig {
            num_lights: 2,
            default_height: 77.0,
            default_intensity: 1234.0,
            default_range: 42.0,
            default_cone_angle: 90.0,
            ..Default::default()
        };
        let group = cfg.generate("Defaults", Vec2::ZERO, Vec2::new(50.0, 50.0));
        for light in &group.lights {
            assert_eq!(light.height, 77.0);
            assert_eq!(light.intensity, 1234.0);
            assert_eq!(light.range, 42.0);
            assert_eq!(light.cone_angle, 90.0);
            assert!(light.visible);
        }
    }

    #[test]
    fn gen_config_zero_lights() {
        let cfg = LightGroupGenConfig {
            num_lights: 0,
            ..Default::default()
        };
        let group = cfg.generate("Empty", Vec2::ZERO, Vec2::new(100.0, 100.0));
        assert!(group.lights.is_empty());
    }

    #[test]
    fn gen_config_lights_have_unique_ids() {
        let cfg = LightGroupGenConfig {
            num_lights: 5,
            ..Default::default()
        };
        let group = cfg.generate("Unique", Vec2::ZERO, Vec2::new(100.0, 100.0));
        let ids: Vec<Uuid> = group.lights.iter().map(|l| l.id).collect();
        for (i, id) in ids.iter().enumerate() {
            assert!(!ids[i + 1..].contains(id), "duplicate light id found");
        }
    }

    #[test]
    fn gen_config_light_names_are_sequential() {
        let cfg = LightGroupGenConfig {
            num_lights: 3,
            ..Default::default()
        };
        let group = cfg.generate("Names", Vec2::ZERO, Vec2::new(100.0, 100.0));
        assert_eq!(group.lights[0].name, "Light 1");
        assert_eq!(group.lights[1].name, "Light 2");
        assert_eq!(group.lights[2].name, "Light 3");
    }

    // -- LightingRig tests -------------------------------------------------

    #[test]
    fn rig_new_is_empty() {
        let rig = LightingRig::new();
        assert!(rig.groups.is_empty());
        assert!(rig.active_group.is_none());
        assert_eq!(rig.light_count(), 0);
    }

    #[test]
    fn rig_add_group() {
        let mut rig = LightingRig::new();
        let g = make_group("G1", 3);
        let gid = g.id;
        let returned_id = rig.add_group(g);

        assert_eq!(returned_id, gid);
        assert_eq!(rig.groups.len(), 1);
        assert_eq!(rig.light_count(), 3);
    }

    #[test]
    fn rig_remove_group() {
        let mut rig = LightingRig::new();
        let g = make_group("G1", 2);
        let gid = g.id;
        rig.add_group(g);

        let removed = rig.remove_group(gid);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().name, "G1");
        assert!(rig.groups.is_empty());
    }

    #[test]
    fn rig_remove_nonexistent_group_returns_none() {
        let mut rig = LightingRig::new();
        assert!(rig.remove_group(Uuid::new_v4()).is_none());
    }

    #[test]
    fn rig_remove_active_group_clears_active() {
        let mut rig = LightingRig::new();
        let g = make_group("G1", 1);
        let gid = g.id;
        rig.add_group(g);
        rig.active_group = Some(gid);

        rig.remove_group(gid);
        assert!(rig.active_group.is_none());
    }

    #[test]
    fn rig_toggle_group_visibility() {
        let mut rig = LightingRig::new();
        let g = make_group("G1", 1);
        let gid = g.id;
        rig.add_group(g);

        assert!(rig.group(gid).unwrap().visible);
        assert!(rig.toggle_group_visibility(gid));
        assert!(!rig.group(gid).unwrap().visible);
        assert!(rig.toggle_group_visibility(gid));
        assert!(rig.group(gid).unwrap().visible);
    }

    #[test]
    fn rig_toggle_missing_group_returns_false() {
        let mut rig = LightingRig::new();
        assert!(!rig.toggle_group_visibility(Uuid::new_v4()));
    }

    #[test]
    fn rig_add_light() {
        let mut rig = LightingRig::new();
        let g = LightGroup::new("G1");
        let gid = g.id;
        rig.add_group(g);

        assert!(rig.add_light(gid, make_light("L1")));
        assert_eq!(rig.light_count(), 1);
    }

    #[test]
    fn rig_add_light_to_missing_group_returns_false() {
        let mut rig = LightingRig::new();
        assert!(!rig.add_light(Uuid::new_v4(), make_light("L1")));
    }

    #[test]
    fn rig_remove_light() {
        let mut rig = LightingRig::new();
        let mut g = LightGroup::new("G1");
        let gid = g.id;
        let l = make_light("L1");
        let lid = l.id;
        g.add_light(l);
        rig.add_group(g);

        let removed = rig.remove_light(gid, lid);
        assert!(removed.is_some());
        assert_eq!(rig.light_count(), 0);
    }

    #[test]
    fn rig_remove_light_missing_returns_none() {
        let mut rig = LightingRig::new();
        let g = LightGroup::new("G1");
        let gid = g.id;
        rig.add_group(g);

        assert!(rig.remove_light(gid, Uuid::new_v4()).is_none());
        assert!(rig.remove_light(Uuid::new_v4(), Uuid::new_v4()).is_none());
    }

    #[test]
    fn rig_toggle_light_visibility() {
        let mut rig = LightingRig::new();
        let mut g = LightGroup::new("G1");
        let gid = g.id;
        let l = make_light("L1");
        let lid = l.id;
        g.add_light(l);
        rig.add_group(g);

        assert!(rig.toggle_light_visibility(gid, lid));
        assert!(!rig.group(gid).unwrap().light(lid).unwrap().visible);
    }

    #[test]
    fn rig_toggle_light_visibility_missing() {
        let mut rig = LightingRig::new();
        assert!(!rig.toggle_light_visibility(Uuid::new_v4(), Uuid::new_v4()));
    }

    #[test]
    fn rig_all_lights_iterates_every_light() {
        let mut rig = LightingRig::new();
        rig.add_group(make_group("G1", 3));
        rig.add_group(make_group("G2", 2));

        let count = rig.all_lights().count();
        assert_eq!(count, 5);
        assert_eq!(rig.light_count(), 5);
    }

    #[test]
    fn rig_visible_lights_respects_group_visibility() {
        let mut rig = LightingRig::new();
        let g = make_group("G1", 3);
        let gid = g.id;
        rig.add_group(g);

        assert_eq!(rig.visible_lights().count(), 3);

        rig.toggle_group_visibility(gid);
        assert_eq!(rig.visible_lights().count(), 0);
    }

    #[test]
    fn rig_visible_lights_respects_light_visibility() {
        let mut rig = LightingRig::new();
        let mut g = LightGroup::new("G1");
        let gid = g.id;
        let l = make_light("L1");
        let lid = l.id;
        g.add_light(l);
        g.add_light(make_light("L2"));
        rig.add_group(g);

        assert_eq!(rig.visible_lights().count(), 2);

        rig.toggle_light_visibility(gid, lid);
        assert_eq!(rig.visible_lights().count(), 1);
    }

    #[test]
    fn rig_group_lookup() {
        let mut rig = LightingRig::new();
        let g = make_group("G1", 1);
        let gid = g.id;
        rig.add_group(g);

        assert!(rig.group(gid).is_some());
        assert_eq!(rig.group(gid).unwrap().name, "G1");
        assert!(rig.group(Uuid::new_v4()).is_none());

        // mutable lookup
        rig.group_mut(gid).unwrap().name = "Renamed".into();
        assert_eq!(rig.group(gid).unwrap().name, "Renamed");
    }

    #[test]
    fn rig_multiple_groups_independent() {
        let mut rig = LightingRig::new();
        let g1 = make_group("G1", 2);
        let g2 = make_group("G2", 3);
        let g1id = g1.id;
        let g2id = g2.id;
        rig.add_group(g1);
        rig.add_group(g2);

        // Toggle G1 off — only G2 lights visible
        rig.toggle_group_visibility(g1id);
        assert_eq!(rig.visible_lights().count(), 3);

        // Remove G2
        rig.remove_group(g2id);
        assert_eq!(rig.light_count(), 2);
        assert_eq!(rig.visible_lights().count(), 0); // G1 still hidden
    }
}
