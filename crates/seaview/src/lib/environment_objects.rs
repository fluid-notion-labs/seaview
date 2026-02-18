//! Environment objects — pure data model
//!
//! Static scene props (GLB/glTF files) imported alongside the fluid simulation.
//! This module is **pure logic**: no Bevy systems, no ECS queries, no asset
//! loading.  The Bevy integration lives in `app/environment_objects.rs`.
//!
//! [`MaterialOverride`] controls per-object material appearance.  `Default`
//! keeps the GLB's own materials; presets and `Custom` replace them with a
//! flat `StandardMaterial` built from the stored colour / roughness / metallic.
//!
//! # Design
//!
//! [`EnvironmentObjectStore`] is the single authoritative resource for
//! environment object state.  The app layer reads it to spawn/despawn scenes
//! and the UI layer reads/writes it for controls.
//!
//! GLB scene loading is intentionally absent here.  When `loadfu` gains
//! multi-object scene support, only the app-layer spawning code needs to
//! change — this module stays the same.

use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::lib::coordinates::WindingOrder;

// ---------------------------------------------------------------------------
// Identifiers
// ---------------------------------------------------------------------------

/// Strongly-typed identifier for an environment object.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct EnvironmentObjectId(pub Uuid);

impl EnvironmentObjectId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for EnvironmentObjectId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for EnvironmentObjectId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// Transform (lib-side, no Bevy Transform)
// ---------------------------------------------------------------------------

/// Position / rotation / scale for an environment object.
///
/// Uses plain arrays so the type is Bevy-free.  The app layer converts to/from
/// `bevy::prelude::Transform` via `From` / `Into` impls.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Transform3D {
    /// World-space position `[x, y, z]`.
    pub position: [f32; 3],
    /// Euler rotation in **degrees** `[pitch, yaw, roll]` (XYZ order).
    pub rotation_degrees: [f32; 3],
    /// Non-uniform scale `[x, y, z]`.
    pub scale: [f32; 3],
}

impl Default for Transform3D {
    fn default() -> Self {
        Self {
            position: [0.0, 0.0, 0.0],
            rotation_degrees: [0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
        }
    }
}

impl Transform3D {
    pub fn with_position(mut self, x: f32, y: f32, z: f32) -> Self {
        self.position = [x, y, z];
        self
    }

    pub fn with_rotation_degrees(mut self, pitch: f32, yaw: f32, roll: f32) -> Self {
        self.rotation_degrees = [pitch, yaw, roll];
        self
    }

    pub fn with_scale(mut self, x: f32, y: f32, z: f32) -> Self {
        self.scale = [x, y, z];
        self
    }

    pub fn with_uniform_scale(mut self, s: f32) -> Self {
        self.scale = [s, s, s];
        self
    }
}

// ---------------------------------------------------------------------------
// Coordinate preset
// ---------------------------------------------------------------------------

/// Predefined coordinate system conventions for environment objects.
///
/// Selecting a preset auto-configures the [`AxisSwizzle`] to match the
/// convention.  Users can still manually override individual axis flips
/// on top.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub enum CoordinatePreset {
    /// Y-up, right-handed (glTF default, Bevy native). Identity swizzle.
    #[default]
    YUp,
    /// Z-up, right-handed (common in CAD, GIS, simulation tools).
    /// Swaps Y↔Z.
    ZUp,
    /// Custom — axis flips are set manually, preset doesn't override them.
    Custom,
}

impl CoordinatePreset {
    /// All presets in display order.
    pub const ALL: &'static [CoordinatePreset] = &[Self::YUp, Self::ZUp, Self::Custom];

    /// Human-readable label for UI.
    pub fn label(self) -> &'static str {
        match self {
            Self::YUp => "Y-up (glTF default)",
            Self::ZUp => "Z-up (CAD / sim)",
            Self::Custom => "Custom",
        }
    }

    /// Returns the [`AxisSwizzle`] that this preset implies.
    /// Returns `None` for `Custom` (user controls the swizzle directly).
    pub fn to_swizzle(self) -> Option<AxisSwizzle> {
        match self {
            Self::YUp => Some(AxisSwizzle::default()),
            Self::ZUp => Some(AxisSwizzle {
                x: AxisMapping::pos(SourceAxis::X),
                y: AxisMapping::pos(SourceAxis::Z),
                z: AxisMapping::neg(SourceAxis::Y),
            }),
            Self::Custom => None,
        }
    }
}

impl std::fmt::Display for CoordinatePreset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

// ---------------------------------------------------------------------------
// Axis swizzle (coordinate remapping)
// ---------------------------------------------------------------------------

/// Which source axis to read from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum SourceAxis {
    X,
    Y,
    Z,
}

impl std::fmt::Display for SourceAxis {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::X => write!(f, "x"),
            Self::Y => write!(f, "y"),
            Self::Z => write!(f, "z"),
        }
    }
}

/// Sign applied to the source axis value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum AxisSign {
    Positive,
    Negative,
}

impl AxisSign {
    /// Returns the multiplier: `+1.0` or `-1.0`.
    pub fn multiplier(self) -> f32 {
        match self {
            Self::Positive => 1.0,
            Self::Negative => -1.0,
        }
    }
}

impl std::fmt::Display for AxisSign {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Positive => write!(f, "+"),
            Self::Negative => write!(f, "-"),
        }
    }
}

/// One axis mapping: which source axis (with sign) feeds this output axis.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AxisMapping {
    pub sign: AxisSign,
    pub source: SourceAxis,
}

impl AxisMapping {
    pub fn new(sign: AxisSign, source: SourceAxis) -> Self {
        Self { sign, source }
    }

    /// Shorthand for a positive mapping.
    pub fn pos(source: SourceAxis) -> Self {
        Self::new(AxisSign::Positive, source)
    }

    /// Shorthand for a negative mapping.
    pub fn neg(source: SourceAxis) -> Self {
        Self::new(AxisSign::Negative, source)
    }
}

impl std::fmt::Display for AxisMapping {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}{}", self.sign, self.source)
    }
}

/// Per-object axis swizzle: remaps source XYZ to output XYZ.
///
/// Default is the identity mapping: X→+x, Y→+y, Z→+z.
/// The app layer converts this to a 3×3 matrix composed with the object's
/// Transform3D before applying to the Bevy `Transform`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AxisSwizzle {
    /// What feeds output X.
    pub x: AxisMapping,
    /// What feeds output Y.
    pub y: AxisMapping,
    /// What feeds output Z.
    pub z: AxisMapping,
}

impl Default for AxisSwizzle {
    fn default() -> Self {
        Self {
            x: AxisMapping::pos(SourceAxis::X),
            y: AxisMapping::pos(SourceAxis::Y),
            z: AxisMapping::pos(SourceAxis::Z),
        }
    }
}

impl AxisSwizzle {
    /// Returns `true` if this is the identity mapping (no remapping).
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }

    /// Build the 3×3 swizzle matrix as column-major `[[f32; 3]; 3]`.
    ///
    /// Each output axis picks one source axis with a sign, so each column
    /// of the matrix has exactly one non-zero entry.
    pub fn to_matrix(&self) -> [[f32; 3]; 3] {
        fn column(mapping: &AxisMapping) -> [f32; 3] {
            let m = mapping.sign.multiplier();
            match mapping.source {
                SourceAxis::X => [m, 0.0, 0.0],
                SourceAxis::Y => [0.0, m, 0.0],
                SourceAxis::Z => [0.0, 0.0, m],
            }
        }
        [column(&self.x), column(&self.y), column(&self.z)]
    }
}

// ---------------------------------------------------------------------------
// Material override
// ---------------------------------------------------------------------------

/// RGBA colour stored as `[r, g, b, a]` in linear sRGB, each 0.0–1.0.
pub type LinearRgba = [f32; 4];

/// Predefined material presets for environment objects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum MaterialPreset {
    /// Earthy green/brown, moderate roughness.
    Terrain,
    /// Warm beige, high roughness.
    Sand,
    /// Grey, high roughness.
    Rock,
    /// Blue, low roughness, moderate metallic.
    Water,
    /// Dark grey, very high roughness.
    Asphalt,
    /// White, low roughness.
    Snow,
}

impl MaterialPreset {
    /// All available presets in display order.
    pub const ALL: &'static [MaterialPreset] = &[
        Self::Terrain,
        Self::Sand,
        Self::Rock,
        Self::Water,
        Self::Asphalt,
        Self::Snow,
    ];

    /// Human-readable name for UI display.
    pub fn label(self) -> &'static str {
        match self {
            Self::Terrain => "Terrain",
            Self::Sand => "Sand",
            Self::Rock => "Rock",
            Self::Water => "Water",
            Self::Asphalt => "Asphalt",
            Self::Snow => "Snow",
        }
    }

    /// Returns `(base_color_linear_rgba, roughness, metallic)`.
    pub fn values(self) -> (LinearRgba, f32, f32) {
        match self {
            Self::Terrain => ([0.13, 0.18, 0.08, 1.0], 0.85, 0.0),
            Self::Sand => ([0.76, 0.70, 0.50, 1.0], 0.95, 0.0),
            Self::Rock => ([0.35, 0.33, 0.30, 1.0], 0.90, 0.0),
            Self::Water => ([0.10, 0.25, 0.50, 0.85], 0.05, 0.3),
            Self::Asphalt => ([0.15, 0.15, 0.15, 1.0], 0.95, 0.0),
            Self::Snow => ([0.90, 0.90, 0.92, 1.0], 0.30, 0.0),
        }
    }
}

impl std::fmt::Display for MaterialPreset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.label())
    }
}

/// User-specified material properties (colour + PBR knobs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomMaterial {
    /// Base colour in linear sRGB `[r, g, b, a]`.
    pub base_color: LinearRgba,
    /// Perceptual roughness 0.0 (mirror) – 1.0 (fully rough).
    pub roughness: f32,
    /// Metallic factor 0.0 (dielectric) – 1.0 (metal).
    pub metallic: f32,
}

impl Default for CustomMaterial {
    fn default() -> Self {
        Self {
            base_color: [0.5, 0.5, 0.5, 1.0],
            roughness: 0.5,
            metallic: 0.0,
        }
    }
}

/// Per-object material override.
///
/// `Default` keeps whatever materials the GLB shipped with.  Any other
/// variant replaces **all** mesh materials on the object with a flat
/// `StandardMaterial` built from the stored values.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MaterialOverride {
    /// Use the GLB's own PBR materials (no override).
    Default,
    /// Apply a named preset.
    Preset(MaterialPreset),
    /// Fully user-specified colour + roughness + metallic.
    Custom(CustomMaterial),
}

impl MaterialOverride {
    /// Returns `true` when no override is active.
    pub fn is_default(&self) -> bool {
        matches!(self, Self::Default)
    }

    /// Resolve to concrete `(base_color, roughness, metallic)`.
    /// Returns `None` for `Default` (keep original materials).
    pub fn resolve(&self) -> Option<(LinearRgba, f32, f32)> {
        match self {
            Self::Default => None,
            Self::Preset(p) => Some(p.values()),
            Self::Custom(c) => Some((c.base_color, c.roughness, c.metallic)),
        }
    }
}

impl std::default::Default for MaterialOverride {
    fn default() -> Self {
        Self::Default
    }
}

// ---------------------------------------------------------------------------
// Mesh entry (discovered after scene loads)
// ---------------------------------------------------------------------------

/// A single mesh primitive within a loaded GLB scene.
///
/// Populated by the app layer once the scene's child entities are enumerated.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshEntry {
    /// Human-readable name (from glTF node name, or fallback like "Mesh 0").
    pub name: String,
    /// Whether this individual mesh is visible.
    pub visible: bool,
}

impl MeshEntry {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            visible: true,
        }
    }
}

// ---------------------------------------------------------------------------
// Environment object
// ---------------------------------------------------------------------------

/// A single imported environment object (GLB/glTF, STL, OBJ, or PLY file).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentObject {
    /// Unique identifier.
    pub id: EnvironmentObjectId,
    /// Display name (defaults to filename stem, user-editable).
    pub name: String,
    /// Path to the source mesh file on disk (GLB/glTF/STL/OBJ/PLY).
    pub path: PathBuf,
    /// Master visibility toggle.
    pub visible: bool,
    /// Whether PBR textures/materials are applied (false → flat fallback).
    pub textures_enabled: bool,
    /// Material override (Default = keep GLB materials, Preset or Custom).
    pub material_override: MaterialOverride,
    /// Flip face normals (fixes inside-out meshes).
    pub flip_normals: bool,
    /// Render both sides of faces (disables backface culling).
    pub double_sided: bool,
    /// Source coordinate system preset (auto-configures swizzle).
    pub coordinate_preset: CoordinatePreset,
    /// Source winding order (CW meshes need normal flipping).
    pub winding_order: WindingOrder,
    /// Object transform (position / rotation / scale).
    pub transform: Transform3D,
    /// Axis remapping applied *before* position/rotation/scale.
    pub swizzle: AxisSwizzle,
    /// Individual mesh primitives discovered after the scene loads.
    /// Empty until the app layer enumerates child entities.
    pub meshes: Vec<MeshEntry>,
}

impl EnvironmentObject {
    /// Create a new environment object from a file path.
    ///
    /// The name defaults to the file stem (e.g. `"fountain"` from
    /// `"/models/fountain.glb"`).
    pub fn new(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let name = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("Object")
            .to_string();
        Self {
            id: EnvironmentObjectId::new(),
            name,
            path,
            visible: true,
            textures_enabled: true,
            material_override: MaterialOverride::default(),
            flip_normals: false,
            double_sided: false,
            coordinate_preset: CoordinatePreset::default(),
            winding_order: WindingOrder::default(),
            transform: Transform3D::default(),
            swizzle: AxisSwizzle::default(),
            meshes: Vec::new(),
        }
    }

    /// Returns `true` if the scene's meshes have been discovered by the app
    /// layer.
    pub fn meshes_discovered(&self) -> bool {
        !self.meshes.is_empty()
    }

    /// Toggle master visibility, returning the new state.
    pub fn toggle_visible(&mut self) -> bool {
        self.visible = !self.visible;
        self.visible
    }

    /// Toggle textures, returning the new state.
    pub fn toggle_textures(&mut self) -> bool {
        self.textures_enabled = !self.textures_enabled;
        self.textures_enabled
    }

    /// Set all mesh entries to the given visibility.
    pub fn set_all_meshes_visible(&mut self, visible: bool) {
        for mesh in &mut self.meshes {
            mesh.visible = visible;
        }
    }
}

// ---------------------------------------------------------------------------
// Store
// ---------------------------------------------------------------------------

/// Authoritative collection of all imported environment objects.
///
/// This is inserted as a Bevy `Resource` (via `derive(Resource)` in the app
/// layer) but contains no ECS logic itself.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct EnvironmentObjectStore {
    objects: Vec<EnvironmentObject>,
    /// Monotonically increasing counter bumped on every mutation.
    /// The app layer uses this to detect changes without Bevy change-detection
    /// on the inner `Vec`.
    #[serde(skip)]
    generation: u64,
}

impl EnvironmentObjectStore {
    pub fn new() -> Self {
        Self::default()
    }

    // -- Queries -------------------------------------------------------------

    /// Number of objects in the store.
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// Current generation counter (bumped on every add/remove/clear).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Iterate over all objects.
    pub fn iter(&self) -> impl Iterator<Item = &EnvironmentObject> {
        self.objects.iter()
    }

    /// Iterate mutably (does **not** bump generation — caller is responsible
    /// for calling [`bump_generation`] if they make meaningful changes).
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut EnvironmentObject> {
        self.objects.iter_mut()
    }

    /// Look up an object by id.
    pub fn get(&self, id: EnvironmentObjectId) -> Option<&EnvironmentObject> {
        self.objects.iter().find(|o| o.id == id)
    }

    /// Look up an object mutably by id.
    pub fn get_mut(&mut self, id: EnvironmentObjectId) -> Option<&mut EnvironmentObject> {
        self.objects.iter_mut().find(|o| o.id == id)
    }

    /// All object ids in insertion order.
    pub fn ids(&self) -> Vec<EnvironmentObjectId> {
        self.objects.iter().map(|o| o.id).collect()
    }

    // -- Mutations -----------------------------------------------------------

    /// Add an object, returning its id.
    pub fn add(&mut self, obj: EnvironmentObject) -> EnvironmentObjectId {
        let id = obj.id;
        self.objects.push(obj);
        self.generation += 1;
        id
    }

    /// Remove an object by id.  Returns `Some(object)` if found.
    pub fn remove(&mut self, id: EnvironmentObjectId) -> Option<EnvironmentObject> {
        if let Some(idx) = self.objects.iter().position(|o| o.id == id) {
            self.generation += 1;
            Some(self.objects.remove(idx))
        } else {
            None
        }
    }

    /// Remove all objects.  Returns the removed objects.
    pub fn clear(&mut self) -> Vec<EnvironmentObject> {
        if !self.objects.is_empty() {
            self.generation += 1;
        }
        std::mem::take(&mut self.objects)
    }

    /// Manually bump the generation counter (e.g. after mutating objects
    /// through `get_mut` or `iter_mut`).
    pub fn bump_generation(&mut self) {
        self.generation += 1;
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_object_has_filename_as_name() {
        let obj = EnvironmentObject::new("/models/cool_fountain.glb");
        assert_eq!(obj.name, "cool_fountain");
        assert!(obj.visible);
        assert!(obj.textures_enabled);
        assert!(obj.material_override.is_default());
        assert!(!obj.flip_normals);
        assert!(!obj.double_sided);
        assert_eq!(obj.coordinate_preset, CoordinatePreset::YUp);
        assert_eq!(obj.winding_order, WindingOrder::CCW);
        assert!(obj.meshes.is_empty());
        assert!(obj.swizzle.is_identity());
    }

    #[test]
    fn default_transform_is_identity() {
        let t = Transform3D::default();
        assert_eq!(t.position, [0.0, 0.0, 0.0]);
        assert_eq!(t.rotation_degrees, [0.0, 0.0, 0.0]);
        assert_eq!(t.scale, [1.0, 1.0, 1.0]);
    }

    #[test]
    fn transform_builder_methods() {
        let t = Transform3D::default()
            .with_position(1.0, 2.0, 3.0)
            .with_rotation_degrees(45.0, 90.0, 0.0)
            .with_uniform_scale(2.0);
        assert_eq!(t.position, [1.0, 2.0, 3.0]);
        assert_eq!(t.rotation_degrees, [45.0, 90.0, 0.0]);
        assert_eq!(t.scale, [2.0, 2.0, 2.0]);
    }

    #[test]
    fn store_add_and_get() {
        let mut store = EnvironmentObjectStore::new();
        assert_eq!(store.generation(), 0);

        let obj = EnvironmentObject::new("test.glb");
        let id = obj.id;
        store.add(obj);

        assert_eq!(store.len(), 1);
        assert_eq!(store.generation(), 1);
        assert!(store.get(id).is_some());
        assert_eq!(store.get(id).unwrap().name, "test");
    }

    #[test]
    fn store_remove() {
        let mut store = EnvironmentObjectStore::new();
        let id = store.add(EnvironmentObject::new("a.glb"));
        let _ = store.add(EnvironmentObject::new("b.glb"));
        assert_eq!(store.len(), 2);

        let removed = store.remove(id);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().name, "a");
        assert_eq!(store.len(), 1);
        assert!(store.get(id).is_none());
    }

    #[test]
    fn store_remove_nonexistent_returns_none() {
        let mut store = EnvironmentObjectStore::new();
        let bogus = EnvironmentObjectId::new();
        assert!(store.remove(bogus).is_none());
    }

    #[test]
    fn store_clear() {
        let mut store = EnvironmentObjectStore::new();
        store.add(EnvironmentObject::new("a.glb"));
        store.add(EnvironmentObject::new("b.glb"));
        let gen_before = store.generation();

        let removed = store.clear();
        assert_eq!(removed.len(), 2);
        assert!(store.is_empty());
        assert!(store.generation() > gen_before);
    }

    #[test]
    fn store_ids_preserves_insertion_order() {
        let mut store = EnvironmentObjectStore::new();
        let id1 = store.add(EnvironmentObject::new("first.glb"));
        let id2 = store.add(EnvironmentObject::new("second.glb"));
        let id3 = store.add(EnvironmentObject::new("third.glb"));

        assert_eq!(store.ids(), vec![id1, id2, id3]);
    }

    #[test]
    fn toggle_visible() {
        let mut obj = EnvironmentObject::new("test.glb");
        assert!(obj.visible);
        assert!(!obj.toggle_visible());
        assert!(!obj.visible);
        assert!(obj.toggle_visible());
        assert!(obj.visible);
    }

    #[test]
    fn toggle_textures() {
        let mut obj = EnvironmentObject::new("test.glb");
        assert!(obj.textures_enabled);
        assert!(!obj.toggle_textures());
        assert!(!obj.textures_enabled);
    }

    #[test]
    fn set_all_meshes_visible() {
        let mut obj = EnvironmentObject::new("test.glb");
        obj.meshes = vec![
            MeshEntry::new("Mesh A"),
            MeshEntry::new("Mesh B"),
            MeshEntry::new("Mesh C"),
        ];

        obj.set_all_meshes_visible(false);
        assert!(obj.meshes.iter().all(|m| !m.visible));

        obj.set_all_meshes_visible(true);
        assert!(obj.meshes.iter().all(|m| m.visible));
    }

    #[test]
    fn mesh_entry_defaults_to_visible() {
        let entry = MeshEntry::new("test");
        assert!(entry.visible);
        assert_eq!(entry.name, "test");
    }

    #[test]
    fn material_override_default_resolves_none() {
        let m = MaterialOverride::Default;
        assert!(m.is_default());
        assert!(m.resolve().is_none());
    }

    #[test]
    fn material_preset_resolves_values() {
        let m = MaterialOverride::Preset(MaterialPreset::Rock);
        assert!(!m.is_default());
        let (color, roughness, metallic) = m.resolve().unwrap();
        assert_eq!(color, [0.35, 0.33, 0.30, 1.0]);
        assert!(roughness > 0.8);
        assert_eq!(metallic, 0.0);
    }

    #[test]
    fn coordinate_preset_yup_is_identity_swizzle() {
        let s = CoordinatePreset::YUp.to_swizzle().unwrap();
        assert!(s.is_identity());
    }

    #[test]
    fn coordinate_preset_zup_swaps_yz() {
        let s = CoordinatePreset::ZUp.to_swizzle().unwrap();
        assert!(!s.is_identity());
        // Output Y reads from +Z, output Z reads from -Y
        assert_eq!(s.y.source, SourceAxis::Z);
        assert_eq!(s.y.sign, AxisSign::Positive);
        assert_eq!(s.z.source, SourceAxis::Y);
        assert_eq!(s.z.sign, AxisSign::Negative);
    }

    #[test]
    fn coordinate_preset_custom_returns_none() {
        assert!(CoordinatePreset::Custom.to_swizzle().is_none());
    }

    #[test]
    fn material_custom_resolves_values() {
        let m = MaterialOverride::Custom(CustomMaterial {
            base_color: [1.0, 0.0, 0.0, 1.0],
            roughness: 0.2,
            metallic: 0.9,
        });
        let (color, roughness, metallic) = m.resolve().unwrap();
        assert_eq!(color, [1.0, 0.0, 0.0, 1.0]);
        assert_eq!(roughness, 0.2);
        assert_eq!(metallic, 0.9);
    }

    #[test]
    fn swizzle_identity_matrix() {
        let s = AxisSwizzle::default();
        assert!(s.is_identity());
        let m = s.to_matrix();
        assert_eq!(m, [[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]]);
    }

    #[test]
    fn swizzle_flip_y() {
        let s = AxisSwizzle {
            x: AxisMapping::pos(SourceAxis::X),
            y: AxisMapping::neg(SourceAxis::Y),
            z: AxisMapping::pos(SourceAxis::Z),
        };
        assert!(!s.is_identity());
        let m = s.to_matrix();
        assert_eq!(m, [[1.0, 0.0, 0.0], [0.0, -1.0, 0.0], [0.0, 0.0, 1.0]]);
    }

    #[test]
    fn swizzle_y_up_to_z_up() {
        // Y-up → Z-up: X→+x, Y→+z, Z→-y
        let s = AxisSwizzle {
            x: AxisMapping::pos(SourceAxis::X),
            y: AxisMapping::pos(SourceAxis::Z),
            z: AxisMapping::neg(SourceAxis::Y),
        };
        let m = s.to_matrix();
        // col 0 (output X): +x → [1,0,0]
        assert_eq!(m[0], [1.0, 0.0, 0.0]);
        // col 1 (output Y): +z → [0,0,1]
        assert_eq!(m[1], [0.0, 0.0, 1.0]);
        // col 2 (output Z): -y → [0,-1,0]
        assert_eq!(m[2], [0.0, -1.0, 0.0]);
    }

    #[test]
    fn store_get_mut_and_bump() {
        let mut store = EnvironmentObjectStore::new();
        let id = store.add(EnvironmentObject::new("test.glb"));
        let gen = store.generation();

        if let Some(obj) = store.get_mut(id) {
            obj.name = "renamed".to_string();
        }
        store.bump_generation();

        assert_eq!(store.get(id).unwrap().name, "renamed");
        assert_eq!(store.generation(), gen + 1);
    }
}
