//! Per-directory settings persistence for Seaview
//!
//! Settings are stored as `seaview.toml` in the sequence directory.
//! They capture camera position, coordinate system, playback preferences,
//! named cameras, and environment settings (lighting, materials, environment light).

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::app::ui::state::MaterialConfig;
use crate::lib::coordinates::WindingOrder;
use crate::lib::environment_objects::{
    AxisMapping, AxisSign, AxisSwizzle, CoordinatePreset, CustomMaterial, EnvironmentObject,
    EnvironmentObjectStore, MaterialOverride, MaterialPreset, MeshEntry, SourceAxis, Transform3D,
};
use crate::lib::lighting::{Light, LightGroup, LightingRig};
use crate::lib::sequence::LoadSection;
use crate::lib::series::{SeriesManager, SeriesState};
use uuid::Uuid;

/// Filename used for per-directory settings
pub const SETTINGS_FILENAME: &str = "seaview.toml";

// ---------------------------------------------------------------------------
// Camera
// ---------------------------------------------------------------------------

/// Camera settings stored as human-editable values
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct CameraSettings {
    /// Camera position in world space [x, y, z]
    pub position: [f32; 3],
    /// Camera rotation as euler angles in degrees [pitch, yaw, roll]
    pub rotation: [f32; 3],
}

impl Default for CameraSettings {
    fn default() -> Self {
        Self {
            position: [100.0, 100.0, 100.0],
            rotation: [0.0, 0.0, 0.0],
        }
    }
}

impl CameraSettings {
    /// Create from a Bevy Transform
    pub fn from_transform(transform: &Transform) -> Self {
        let pos = transform.translation;
        let (yaw, pitch, roll) = transform.rotation.to_euler(EulerRot::YXZ);
        Self {
            position: [pos.x, pos.y, pos.z],
            rotation: [pitch.to_degrees(), yaw.to_degrees(), roll.to_degrees()],
        }
    }

    /// Convert to a Bevy Transform
    pub fn to_transform(&self) -> Transform {
        let translation = Vec3::new(self.position[0], self.position[1], self.position[2]);
        let pitch = self.rotation[0].to_radians();
        let yaw = self.rotation[1].to_radians();
        let roll = self.rotation[2].to_radians();
        let rotation = Quat::from_euler(EulerRot::YXZ, yaw, pitch, roll);
        Transform {
            translation,
            rotation,
            scale: Vec3::ONE,
        }
    }
}

// ---------------------------------------------------------------------------
// Sequence / Playback / Mesh bounds  (existing, unchanged)
// ---------------------------------------------------------------------------

/// Sequence/coordinate settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SequenceSettings {
    /// Source coordinate system: "yup", "zup", "fluidx3d"
    pub source_coordinates: Option<String>,
}

impl Default for SequenceSettings {
    fn default() -> Self {
        Self {
            source_coordinates: None,
        }
    }
}

/// Playback settings
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct PlaybackSettings {
    /// Playback speed multiplier (e.g. 1.0 = normal, 2.0 = double speed)
    pub speed: Option<f32>,
    /// Whether to loop playback
    #[serde(rename = "loop")]
    pub loop_enabled: Option<bool>,
    /// Whether to watch directory for new frames
    pub watch: Option<bool>,
}

impl Default for PlaybackSettings {
    fn default() -> Self {
        Self {
            speed: None,
            loop_enabled: None,
            watch: None,
        }
    }
}

/// Cached mesh bounding-box dimensions (in metres).
/// Stored so the UI can show dimensions immediately without re-scanning vertices.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MeshBoundsSettings {
    /// Axis-aligned minimum corner [x, y, z]
    pub min: [f32; 3],
    /// Axis-aligned maximum corner [x, y, z]
    pub max: [f32; 3],
    /// Dimensions (max - min) per axis [x, y, z]
    pub dimensions: [f32; 3],
}

impl Default for MeshBoundsSettings {
    fn default() -> Self {
        Self {
            min: [0.0; 3],
            max: [0.0; 3],
            dimensions: [0.0; 3],
        }
    }
}

// ---------------------------------------------------------------------------
// Load Section — frame range to keep in memory
// ---------------------------------------------------------------------------

/// Settings for the loaded frame range.
///
/// Defines which frames are kept in memory. The range is stored as
/// frame indices `[start, end)` (exclusive end).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LoadSectionSettings {
    /// Start frame index (inclusive)
    pub start: usize,
    /// End frame index (exclusive)
    pub end: usize,
    /// Whether this section is active (frames loaded into memory).
    /// Defaults to `true` for backwards compatibility with old TOML files.
    #[serde(default = "default_true")]
    pub active: bool,
}

impl Default for LoadSectionSettings {
    fn default() -> Self {
        Self {
            start: 0,
            end: 100, // Default to first 100 frames
            active: true,
        }
    }
}

impl LoadSectionSettings {
    /// Create a new load section
    pub fn new(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            active: true,
        }
    }

    /// Get the number of frames in this section
    pub fn frame_count(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Check if a frame index is within this section
    pub fn contains(&self, frame: usize) -> bool {
        frame >= self.start && frame < self.end
    }

    /// Clamp the section to valid bounds given total frames and max allowed
    pub fn clamp(&mut self, total_frames: usize, max_frames: usize) {
        // Ensure end doesn't exceed total
        self.end = self.end.min(total_frames);
        // Ensure start is before end
        self.start = self.start.min(self.end);
        // Ensure we don't exceed max frames
        let current_count = self.frame_count();
        if current_count > max_frames {
            // Shrink from the end
            self.end = self.start + max_frames;
        }
    }

    /// Expand the section by the given amount on each side, respecting limits
    pub fn expand(&mut self, amount: usize, total_frames: usize, max_frames: usize) {
        let new_start = self.start.saturating_sub(amount);
        let new_end = (self.end + amount).min(total_frames);
        let new_count = new_end - new_start;

        if new_count <= max_frames {
            self.start = new_start;
            self.end = new_end;
        }
    }
}

// -- Conversions between LoadSectionSettings ↔ LoadSection ----------------

impl From<LoadSectionSettings> for LoadSection {
    fn from(s: LoadSectionSettings) -> Self {
        let mut ls = LoadSection::new(s.start, s.end);
        ls.active = s.active;
        ls
    }
}

impl From<&LoadSectionSettings> for LoadSection {
    fn from(s: &LoadSectionSettings) -> Self {
        let mut ls = LoadSection::new(s.start, s.end);
        ls.active = s.active;
        ls
    }
}

impl From<LoadSection> for LoadSectionSettings {
    fn from(s: LoadSection) -> Self {
        let mut ls = LoadSectionSettings::new(s.start, s.end);
        ls.active = s.active;
        ls
    }
}

impl From<&LoadSection> for LoadSectionSettings {
    fn from(s: &LoadSection) -> Self {
        let mut ls = LoadSectionSettings::new(s.start, s.end);
        ls.active = s.active;
        ls
    }
}

// ---------------------------------------------------------------------------
// Series — multi-series persistence
// ---------------------------------------------------------------------------

/// Settings for a single series entry in `seaview.toml`.
///
/// Stored as `[[series]]` array-of-tables.  Each entry captures the series
/// label, optional base directory, per-series load sections, and playhead.
///
/// ```toml
/// [[series]]
/// name = "splash_v3"
/// path = "/data/sims/splash_v3/meshes"
/// active = true
/// playhead = 42
///
/// [[series.load_sections]]
/// start = 0
/// end = 100
/// active = true
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SeriesSettings {
    /// Human-readable label for this series.
    pub name: String,

    /// Base directory for the series' mesh frames.
    /// Optional — when loading from the same directory as `seaview.toml`,
    /// this can be omitted and the directory is inferred.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,

    /// Whether this series is the active (visible) one.
    #[serde(default)]
    pub active: bool,

    /// Playhead position (frame index) when the session was saved.
    #[serde(default)]
    pub playhead: usize,

    /// Per-series load sections.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub load_sections: Vec<LoadSectionSettings>,
}

impl Default for SeriesSettings {
    fn default() -> Self {
        Self {
            name: String::new(),
            path: None,
            active: false,
            playhead: 0,
            load_sections: Vec::new(),
        }
    }
}

impl From<&SeriesState> for SeriesSettings {
    fn from(state: &SeriesState) -> Self {
        Self {
            name: state.label.clone(),
            path: Some(state.base_dir.to_string_lossy().into_owned()),
            active: false, // caller sets this based on active_id
            playhead: state.timeline.playhead,
            load_sections: state
                .timeline
                .sections
                .iter()
                .map(LoadSectionSettings::from)
                .collect(),
        }
    }
}

impl SeriesSettings {
    /// Convert to runtime [`LoadSection`] values.
    pub fn get_load_sections(&self) -> Vec<LoadSection> {
        self.load_sections.iter().map(LoadSection::from).collect()
    }
}

impl From<&SeriesSettings> for crate::lib::series::SavedSeriesEntry {
    fn from(s: &SeriesSettings) -> Self {
        Self {
            name: s.name.clone(),
            path: s.path.as_ref().map(std::path::PathBuf::from),
            active: s.active,
            playhead: s.playhead,
            load_sections: s.load_sections.iter().map(LoadSection::from).collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// Environment — Lighting bridge
// ---------------------------------------------------------------------------

/// TOML-friendly bridge for [`NightLightingConfig`] (spot-light rig).
///
/// Bevy `Color` is stored as `[f32; 3]` sRGB arrays for human readability.
/// `PlacementAlgorithm` is stored as a `snake_case` string.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LightingSettings {
    pub num_lights: usize,
    pub height: f32,
    pub cone_angle: f32,
    /// One of: `"uniform_grid"`, `"hexagonal_packing"`, `"poisson_disk"`,
    /// `"single_row"`, `"single_column"`.
    pub placement_algorithm: String,
    pub enabled: bool,
    /// Legacy field — migrated into `EnvironmentLightSettings.enabled`.
    /// Kept for backwards-compatible TOML parsing; ignored at runtime.
    #[serde(default = "default_true")]
    pub global_lighting_enabled: bool,
    pub intensity: f32,
    /// sRGB colour as `[r, g, b]` in 0–1 range.
    pub color: [f32; 3],
    pub range: f32,
    pub show_markers: bool,
    pub marker_size: f32,
    pub coverage_pct: f32,
}

fn default_true() -> bool {
    true
}

// ---------------------------------------------------------------------------
// Environment — Environment Light bridge
// ---------------------------------------------------------------------------

/// TOML-friendly bridge for [`EnvironmentLightConfig`].
///
/// Controls the global scene lights (directional + ambient), independent of
/// the spot-light rig.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct EnvironmentLightSettings {
    pub enabled: bool,
    /// Ambient light sRGB colour `[r, g, b]` in 0–1 range.
    pub ambient_color: [f32; 3],
    /// Ambient light brightness.
    pub ambient_brightness: f32,
    /// Directional light illuminance (lux).
    pub directional_illuminance: f32,
    /// Directional light sRGB colour `[r, g, b]` in 0–1 range.
    pub directional_color: [f32; 3],
}

impl Default for EnvironmentLightSettings {
    fn default() -> Self {
        let cfg = crate::lib::lighting::EnvironmentLightConfig::default();
        Self::from(&cfg)
    }
}

impl From<&crate::lib::lighting::EnvironmentLightConfig> for EnvironmentLightSettings {
    fn from(cfg: &crate::lib::lighting::EnvironmentLightConfig) -> Self {
        Self {
            enabled: cfg.enabled,
            ambient_color: color_to_srgb3(&cfg.ambient_color),
            ambient_brightness: cfg.ambient_brightness,
            directional_illuminance: cfg.directional_illuminance,
            directional_color: color_to_srgb3(&cfg.directional_color),
        }
    }
}

impl From<&EnvironmentLightSettings> for crate::lib::lighting::EnvironmentLightConfig {
    fn from(s: &EnvironmentLightSettings) -> Self {
        Self {
            enabled: s.enabled,
            ambient_color: Color::srgb(s.ambient_color[0], s.ambient_color[1], s.ambient_color[2]),
            ambient_brightness: s.ambient_brightness,
            directional_illuminance: s.directional_illuminance,
            directional_color: Color::srgb(
                s.directional_color[0],
                s.directional_color[1],
                s.directional_color[2],
            ),
        }
    }
}

impl Default for LightingSettings {
    fn default() -> Self {
        Self {
            num_lights: 9,
            height: 50.0,
            cone_angle: 60.0,
            placement_algorithm: "uniform_grid".to_string(),
            enabled: true,
            global_lighting_enabled: true,
            intensity: 800_000.0,
            color: [1.0, 0.95, 0.9],
            range: 500.0,
            show_markers: true,
            marker_size: 0.5,
            coverage_pct: 120.0,
        }
    }
}

// ---------------------------------------------------------------------------
// Environment — Lighting v2 bridge
// ---------------------------------------------------------------------------

/// TOML-friendly settings for a single [`Light`] in the v2 lighting rig.
///
/// Colour stored as `[f32; 3]` sRGB, position as `[f32; 2]` (XZ plane).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LightSettings {
    pub name: String,
    /// XZ position `[x, z]` in metres.
    pub position: [f32; 2],
    /// Height above sea level (metres).
    pub height: f32,
    /// Intensity in lumens.
    pub intensity: f32,
    /// Maximum effective distance (metres).
    pub range: f32,
    /// Full cone angle (degrees).
    pub cone_angle: f32,
    /// sRGB colour `[r, g, b]` in 0–1 range.
    pub color: [f32; 3],
    /// Per-light visibility toggle.
    pub visible: bool,
}

impl Default for LightSettings {
    fn default() -> Self {
        let l = Light::new("default");
        Self::from(&l)
    }
}

impl From<&Light> for LightSettings {
    fn from(l: &Light) -> Self {
        Self {
            name: l.name.clone(),
            position: [l.position.x, l.position.y],
            height: l.height,
            intensity: l.intensity,
            range: l.range,
            cone_angle: l.cone_angle,
            color: color_to_srgb3(&l.color),
            visible: l.visible,
        }
    }
}

impl From<&LightSettings> for Light {
    fn from(s: &LightSettings) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: s.name.clone(),
            position: Vec2::new(s.position[0], s.position[1]),
            height: s.height,
            intensity: s.intensity,
            range: s.range,
            cone_angle: s.cone_angle,
            color: Color::srgb(s.color[0], s.color[1], s.color[2]),
            visible: s.visible,
        }
    }
}

/// TOML-friendly settings for a [`LightGroup`] in the v2 lighting rig.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct LightGroupSettings {
    pub name: String,
    pub visible: bool,
    #[serde(default)]
    pub lights: Vec<LightSettings>,
}

impl Default for LightGroupSettings {
    fn default() -> Self {
        Self {
            name: "New Group".to_string(),
            visible: true,
            lights: Vec::new(),
        }
    }
}

impl From<&LightGroup> for LightGroupSettings {
    fn from(g: &LightGroup) -> Self {
        Self {
            name: g.name.clone(),
            visible: g.visible,
            lights: g.lights.iter().map(LightSettings::from).collect(),
        }
    }
}

impl From<&LightGroupSettings> for LightGroup {
    fn from(s: &LightGroupSettings) -> Self {
        Self {
            id: Uuid::new_v4(),
            name: s.name.clone(),
            visible: s.visible,
            lights: s.lights.iter().map(Light::from).collect(),
        }
    }
}

/// Convert a [`LightingRig`] to a `Vec<LightGroupSettings>` for TOML
/// serialization.
pub fn lighting_rig_to_settings(rig: &LightingRig) -> Vec<LightGroupSettings> {
    rig.groups.iter().map(LightGroupSettings::from).collect()
}

/// Reconstruct a [`LightingRig`] from a `Vec<LightGroupSettings>` loaded
/// from TOML. UUIDs are regenerated; `active_group` resets to `None`.
pub fn settings_to_lighting_rig(groups: &[LightGroupSettings]) -> LightingRig {
    LightingRig {
        groups: groups.iter().map(LightGroup::from).collect(),
        active_group: None,
    }
}

// ---------------------------------------------------------------------------
// Environment — Material bridge
// ---------------------------------------------------------------------------

/// TOML-friendly bridge for [`MaterialConfig`].
///
/// Colours stored as `[f32; 3]` sRGB arrays. `alpha_mode` is a string
/// (`"opaque"`, `"mask"`, `"blend"`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MaterialSettings {
    /// Base colour `[r, g, b]` sRGB 0–1.
    pub base_color: [f32; 3],
    pub perceptual_roughness: f32,
    pub metallic: f32,
    pub reflectance: f32,
    /// Emissive colour `[r, g, b]` sRGB 0–1.
    pub emissive: [f32; 3],
    pub emissive_intensity: f32,
    pub double_sided: bool,
    /// `"opaque"` | `"mask"` | `"blend"`
    pub alpha_mode: String,
    pub alpha_cutoff: f32,
}

impl Default for MaterialSettings {
    fn default() -> Self {
        let cfg = MaterialConfig::default();
        Self::from(&cfg)
    }
}

impl From<&MaterialConfig> for MaterialSettings {
    fn from(cfg: &MaterialConfig) -> Self {
        let alpha_mode = if cfg.opacity < 1.0 {
            "blend".to_string()
        } else {
            "opaque".to_string()
        };

        Self {
            base_color: color_to_srgb3(&cfg.base_color),
            perceptual_roughness: cfg.perceptual_roughness,
            metallic: cfg.metallic,
            reflectance: cfg.reflectance,
            emissive: color_to_srgb3(&cfg.emissive),
            emissive_intensity: cfg.emissive_intensity,
            double_sided: cfg.double_sided,
            alpha_mode,
            alpha_cutoff: if cfg.opacity < 1.0 { cfg.opacity } else { 0.5 },
        }
    }
}

impl From<&MaterialSettings> for MaterialConfig {
    fn from(s: &MaterialSettings) -> Self {
        let opacity = match s.alpha_mode.as_str() {
            "blend" => s.alpha_cutoff.clamp(0.0, 1.0),
            _ => 1.0,
        };

        Self {
            base_color: Color::srgb(s.base_color[0], s.base_color[1], s.base_color[2]),
            opacity,
            perceptual_roughness: s.perceptual_roughness,
            metallic: s.metallic,
            reflectance: s.reflectance,
            emissive: Color::srgb(s.emissive[0], s.emissive[1], s.emissive[2]),
            emissive_intensity: s.emissive_intensity,
            double_sided: s.double_sided,
        }
    }
}

// ---------------------------------------------------------------------------
// EnvironmentObjectSettings — persistence for imported GLB scene props
// ---------------------------------------------------------------------------

/// Per-mesh visibility override saved to TOML.
///
/// On reload, after mesh discovery, the app layer matches by name and applies
/// the saved visibility state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshVisibilitySettings {
    pub name: String,
    #[serde(default = "default_true")]
    pub visible: bool,
}

/// Persisted state for a single environment object (GLB/glTF scene prop).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentObjectSettings {
    /// Absolute path to the GLB/glTF file on disk.
    pub path: String,

    /// User-edited display name.  Falls back to filename stem on load if
    /// absent (old TOML files).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Master visibility toggle.
    #[serde(default = "default_true")]
    pub visible: bool,

    /// Whether PBR textures are enabled (false → flat fallback material).
    #[serde(default = "default_true")]
    pub textures_enabled: bool,

    /// Flip face normals (fixes inside-out meshes).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub flip_normals: bool,

    /// Render both sides of faces (disables backface culling).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub double_sided: bool,

    /// Source coordinate system preset: "yup", "zup", or "custom".
    #[serde(
        default = "default_coordinate_preset",
        skip_serializing_if = "is_default_coordinate_preset"
    )]
    pub coordinate_preset: String,

    /// Source winding order: "ccw" or "cw".
    #[serde(
        default = "default_winding_order",
        skip_serializing_if = "is_default_winding_order"
    )]
    pub winding_order: String,

    /// World-space position `[x, y, z]`.
    #[serde(default, skip_serializing_if = "is_zero_vec3")]
    pub position: [f32; 3],

    /// Euler rotation in degrees `[pitch, yaw, roll]`.
    #[serde(default, skip_serializing_if = "is_zero_vec3")]
    pub rotation_degrees: [f32; 3],

    /// Non-uniform scale `[x, y, z]`.
    #[serde(default = "default_one_vec3", skip_serializing_if = "is_one_vec3")]
    pub scale: [f32; 3],

    /// Per-mesh visibility overrides (matched by name after scene loads).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mesh_visibility: Vec<MeshVisibilitySettings>,

    /// Material override mode: "default", "preset:<name>", or "custom".
    #[serde(
        default = "default_material_mode",
        skip_serializing_if = "is_default_material_mode"
    )]
    pub material_mode: String,

    /// Custom material base color [r, g, b, a] in linear sRGB.
    #[serde(
        default = "default_material_color",
        skip_serializing_if = "is_default_material_color"
    )]
    pub material_color: [f32; 4],

    /// Custom material roughness 0.0–1.0.
    #[serde(
        default = "default_material_roughness",
        skip_serializing_if = "is_default_material_roughness"
    )]
    pub material_roughness: f32,

    /// Custom material metallic 0.0–1.0.
    #[serde(default, skip_serializing_if = "is_zero_f32")]
    pub material_metallic: f32,

    /// Axis swizzle: output X reads from this source axis.
    #[serde(
        default = "default_swizzle_x",
        skip_serializing_if = "is_default_swizzle_x"
    )]
    pub swizzle_x: String,
    /// Axis swizzle: output Y reads from this source axis.
    #[serde(
        default = "default_swizzle_y",
        skip_serializing_if = "is_default_swizzle_y"
    )]
    pub swizzle_y: String,
    /// Axis swizzle: output Z reads from this source axis.
    #[serde(
        default = "default_swizzle_z",
        skip_serializing_if = "is_default_swizzle_z"
    )]
    pub swizzle_z: String,
}

fn is_zero_vec3(v: &[f32; 3]) -> bool {
    v[0] == 0.0 && v[1] == 0.0 && v[2] == 0.0
}

fn is_zero_f32(v: &f32) -> bool {
    *v == 0.0
}

fn default_coordinate_preset() -> String {
    "yup".to_string()
}
fn is_default_coordinate_preset(s: &str) -> bool {
    s == "yup"
}
fn default_winding_order() -> String {
    "ccw".to_string()
}
fn is_default_winding_order(s: &str) -> bool {
    s == "ccw"
}

fn parse_coordinate_preset(s: &str) -> CoordinatePreset {
    match s.to_lowercase().as_str() {
        "yup" | "y-up" | "y_up" => CoordinatePreset::YUp,
        "zup" | "z-up" | "z_up" => CoordinatePreset::ZUp,
        "custom" => CoordinatePreset::Custom,
        _ => {
            log::warn!("Unknown coordinate preset '{}', defaulting to Y-up", s);
            CoordinatePreset::YUp
        }
    }
}

fn format_coordinate_preset(p: &CoordinatePreset) -> String {
    match p {
        CoordinatePreset::YUp => "yup".to_string(),
        CoordinatePreset::ZUp => "zup".to_string(),
        CoordinatePreset::Custom => "custom".to_string(),
    }
}

fn parse_winding_order(s: &str) -> WindingOrder {
    match s.to_lowercase().as_str() {
        "ccw" => WindingOrder::CCW,
        "cw" => WindingOrder::CW,
        _ => {
            log::warn!("Unknown winding order '{}', defaulting to CCW", s);
            WindingOrder::CCW
        }
    }
}

fn format_winding_order(w: &WindingOrder) -> String {
    match w {
        WindingOrder::CCW => "ccw".to_string(),
        WindingOrder::CW => "cw".to_string(),
    }
}

fn default_material_mode() -> String {
    "default".to_string()
}
fn is_default_material_mode(s: &str) -> bool {
    s == "default"
}
fn default_material_color() -> [f32; 4] {
    [0.5, 0.5, 0.5, 1.0]
}
fn is_default_material_color(c: &[f32; 4]) -> bool {
    c == &[0.5, 0.5, 0.5, 1.0]
}
fn default_material_roughness() -> f32 {
    0.5
}
fn is_default_material_roughness(v: &f32) -> bool {
    *v == 0.5
}

/// Parse a material_mode string into a [`MaterialOverride`].
fn parse_material_override(
    mode: &str,
    color: [f32; 4],
    roughness: f32,
    metallic: f32,
) -> MaterialOverride {
    match mode {
        "default" => MaterialOverride::Default,
        "custom" => MaterialOverride::Custom(CustomMaterial {
            base_color: color,
            roughness,
            metallic,
        }),
        s if s.starts_with("preset:") => {
            let name = &s["preset:".len()..];
            match name.to_lowercase().as_str() {
                "terrain" => MaterialOverride::Preset(MaterialPreset::Terrain),
                "sand" => MaterialOverride::Preset(MaterialPreset::Sand),
                "rock" => MaterialOverride::Preset(MaterialPreset::Rock),
                "water" => MaterialOverride::Preset(MaterialPreset::Water),
                "asphalt" => MaterialOverride::Preset(MaterialPreset::Asphalt),
                "snow" => MaterialOverride::Preset(MaterialPreset::Snow),
                _ => {
                    log::warn!(
                        "Unknown material preset '{}', falling back to default",
                        name
                    );
                    MaterialOverride::Default
                }
            }
        }
        _ => {
            log::warn!("Unknown material_mode '{}', falling back to default", mode);
            MaterialOverride::Default
        }
    }
}

/// Format a [`MaterialOverride`] as a material_mode string.
fn format_material_mode(m: &MaterialOverride) -> String {
    match m {
        MaterialOverride::Default => "default".to_string(),
        MaterialOverride::Preset(p) => format!("preset:{}", p.label().to_lowercase()),
        MaterialOverride::Custom(_) => "custom".to_string(),
    }
}

/// Extract color/roughness/metallic for serialisation.
fn material_override_values(m: &MaterialOverride) -> ([f32; 4], f32, f32) {
    match m {
        MaterialOverride::Default => (default_material_color(), default_material_roughness(), 0.0),
        MaterialOverride::Preset(p) => p.values(),
        MaterialOverride::Custom(c) => (c.base_color, c.roughness, c.metallic),
    }
}

fn default_one_vec3() -> [f32; 3] {
    [1.0, 1.0, 1.0]
}

fn is_one_vec3(v: &[f32; 3]) -> bool {
    v[0] == 1.0 && v[1] == 1.0 && v[2] == 1.0
}

fn default_swizzle_x() -> String {
    "+x".to_string()
}
fn default_swizzle_y() -> String {
    "+y".to_string()
}
fn default_swizzle_z() -> String {
    "+z".to_string()
}
fn is_default_swizzle_x(s: &str) -> bool {
    s == "+x"
}
fn is_default_swizzle_y(s: &str) -> bool {
    s == "+y"
}
fn is_default_swizzle_z(s: &str) -> bool {
    s == "+z"
}

/// Parse a swizzle string like "+x", "-z" into an [`AxisMapping`].
fn parse_axis_mapping(s: &str) -> AxisMapping {
    let s = s.trim();
    let (sign, axis_char) = if s.starts_with('-') {
        (AxisSign::Negative, &s[1..])
    } else if s.starts_with('+') {
        (AxisSign::Positive, &s[1..])
    } else {
        (AxisSign::Positive, s)
    };
    let source = match axis_char {
        "x" | "X" => SourceAxis::X,
        "y" | "Y" => SourceAxis::Y,
        "z" | "Z" => SourceAxis::Z,
        _ => {
            log::warn!("Unknown swizzle axis '{}', defaulting to +x", s);
            return AxisMapping::pos(SourceAxis::X);
        }
    };
    AxisMapping::new(sign, source)
}

/// Format an [`AxisMapping`] as a string like "+x", "-z".
fn format_axis_mapping(m: &AxisMapping) -> String {
    format!("{}{}", m.sign, m.source)
}

impl From<&EnvironmentObject> for EnvironmentObjectSettings {
    fn from(obj: &EnvironmentObject) -> Self {
        let (mat_color, mat_roughness, mat_metallic) =
            material_override_values(&obj.material_override);
        Self {
            path: obj.path.to_string_lossy().to_string(),
            name: Some(obj.name.clone()),
            visible: obj.visible,
            textures_enabled: obj.textures_enabled,
            flip_normals: obj.flip_normals,
            double_sided: obj.double_sided,
            coordinate_preset: format_coordinate_preset(&obj.coordinate_preset),
            winding_order: format_winding_order(&obj.winding_order),
            position: obj.transform.position,
            rotation_degrees: obj.transform.rotation_degrees,
            scale: obj.transform.scale,
            mesh_visibility: obj
                .meshes
                .iter()
                .map(|m| MeshVisibilitySettings {
                    name: m.name.clone(),
                    visible: m.visible,
                })
                .collect(),
            material_mode: format_material_mode(&obj.material_override),
            material_color: mat_color,
            material_roughness: mat_roughness,
            material_metallic: mat_metallic,
            swizzle_x: format_axis_mapping(&obj.swizzle.x),
            swizzle_y: format_axis_mapping(&obj.swizzle.y),
            swizzle_z: format_axis_mapping(&obj.swizzle.z),
        }
    }
}

impl From<&EnvironmentObjectSettings> for EnvironmentObject {
    fn from(s: &EnvironmentObjectSettings) -> Self {
        let path = std::path::PathBuf::from(&s.path);
        let name = s.name.clone().unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Object")
                .to_string()
        });
        Self {
            id: crate::lib::environment_objects::EnvironmentObjectId::new(),
            name,
            path,
            visible: s.visible,
            textures_enabled: s.textures_enabled,
            flip_normals: s.flip_normals,
            double_sided: s.double_sided,
            coordinate_preset: parse_coordinate_preset(&s.coordinate_preset),
            winding_order: parse_winding_order(&s.winding_order),
            material_override: parse_material_override(
                &s.material_mode,
                s.material_color,
                s.material_roughness,
                s.material_metallic,
            ),
            transform: Transform3D {
                position: s.position,
                rotation_degrees: s.rotation_degrees,
                scale: s.scale,
            },
            swizzle: AxisSwizzle {
                x: parse_axis_mapping(&s.swizzle_x),
                y: parse_axis_mapping(&s.swizzle_y),
                z: parse_axis_mapping(&s.swizzle_z),
            },
            meshes: s
                .mesh_visibility
                .iter()
                .map(|mv| MeshEntry {
                    name: mv.name.clone(),
                    visible: mv.visible,
                })
                .collect(),
        }
    }
}

/// Convert an [`EnvironmentObjectStore`] to settings for persistence.
pub fn environment_objects_to_settings(
    store: &EnvironmentObjectStore,
) -> Vec<EnvironmentObjectSettings> {
    store.iter().map(EnvironmentObjectSettings::from).collect()
}

/// Reconstruct an [`EnvironmentObjectStore`] from persisted settings.
///
/// Missing files are logged as warnings but still added to the store so the
/// user can see them in the UI and fix paths manually.
pub fn settings_to_environment_objects(
    settings: &[EnvironmentObjectSettings],
) -> EnvironmentObjectStore {
    let mut store = EnvironmentObjectStore::new();
    for s in settings {
        let path = std::path::Path::new(&s.path);
        if !path.exists() {
            log::warn!(
                "Environment object file not found: {} — adding anyway",
                s.path
            );
        }
        store.add(EnvironmentObject::from(s));
    }
    store
}

// ---------------------------------------------------------------------------
// EnvironmentSettings — grouping struct
// ---------------------------------------------------------------------------

/// Groups all visual-appearance settings.
///
/// Future sections (props, background, post-processing) are reserved as
/// `Option` fields so they can be omitted from TOML when absent.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct EnvironmentSettings {
    /// Environment (global) light: directional + ambient.
    pub environment_light: Option<EnvironmentLightSettings>,
    /// Spot-light rig configuration (legacy v1 — kept for backwards compat on
    /// deserialize, ignored when `lighting_groups` is non-empty).
    pub lighting: Option<LightingSettings>,
    /// Lighting v2: named groups with per-light params.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lighting_groups: Vec<LightGroupSettings>,
    /// Named materials map. `"fluid"` is the default key for the sequence mesh.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub materials: HashMap<String, MaterialSettings>,

    /// Imported environment objects (GLB/glTF scene props).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub objects: Vec<EnvironmentObjectSettings>,
}

// ---------------------------------------------------------------------------
// SavedCameras resource  (for runtime camera switching)
// ---------------------------------------------------------------------------

/// Bevy resource holding all named cameras from `seaview.toml`.
///
/// Inserted at startup so the camera-switching UI can access them.
#[derive(Resource, Debug, Clone)]
pub struct SavedCameras {
    /// Named camera presets.
    pub cameras: HashMap<String, CameraSettings>,
    /// Currently active camera name.
    pub active: String,
}

impl Default for SavedCameras {
    fn default() -> Self {
        Self {
            cameras: HashMap::new(),
            active: "default".to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// Top-level Settings
// ---------------------------------------------------------------------------

/// Top-level settings struct, serialized as `seaview.toml`.
///
/// # Migration
///
/// Old files with a single `[camera]` section (no `[cameras.*]`) are
/// automatically migrated: the camera is moved into `cameras.default`
/// and `active_camera` is set to `"default"`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    // -- cameras ----------------------------------------------------------
    /// Name of the active camera preset.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub active_camera: Option<String>,

    /// Named camera presets (`[cameras.<name>]`).
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub cameras: HashMap<String, CameraSettings>,

    /// **Legacy** — single `[camera]` section from older files.
    /// Automatically migrated into `cameras["default"]` on load.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub camera: Option<CameraSettings>,

    // -- data / workflow --------------------------------------------------
    /// Sequence/coordinate settings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sequence: Option<SequenceSettings>,

    /// Playback settings.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub playback: Option<PlaybackSettings>,

    /// Cached mesh bounding-box (computed on first load).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mesh: Option<MeshBoundsSettings>,

    // -- environment ------------------------------------------------------
    /// Visual appearance: lighting, materials, (future) props/bg/post.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub environment: Option<EnvironmentSettings>,

    // -- memory / loading -------------------------------------------------
    /// Single load section (legacy / single-window mode).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub load_section: Option<LoadSectionSettings>,

    /// Multiple load sections for non-contiguous playback.
    /// Takes precedence over `load_section` when present.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub load_sections: Vec<LoadSectionSettings>,

    // -- multi-series -----------------------------------------------------
    /// Named series entries for multi-series sessions.
    ///
    /// Each entry stores a series label, optional path, per-series load
    /// sections, and playhead.  When present, these take precedence over
    /// the top-level `load_section` / `load_sections` for the *first*
    /// (active) series.
    ///
    /// **Backward compatibility:** if `series` is empty, the top-level
    /// `load_section`/`load_sections` are used as a single default series
    /// (legacy behavior).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub series: Vec<SeriesSettings>,
}

impl Settings {
    /// Get all configured load sections as runtime [`LoadSection`] values.
    ///
    /// Resolution order:
    /// 1. `load_sections` (plural) if non-empty
    /// 2. `load_section` (singular) wrapped in a `Vec`
    /// 3. Empty `Vec` (no sections configured)
    pub fn get_load_sections(&self) -> Vec<LoadSection> {
        if !self.load_sections.is_empty() {
            self.load_sections.iter().map(LoadSection::from).collect()
        } else if let Some(ref single) = self.load_section {
            vec![LoadSection::from(single)]
        } else {
            Vec::new()
        }
    }

    /// Set load sections from runtime [`LoadSection`] values.
    ///
    /// If there is exactly one section, stores it in `load_section` (singular)
    /// for backward-compatible TOML output. Otherwise uses `load_sections`.
    pub fn set_load_sections(&mut self, sections: &[LoadSection]) {
        match sections.len() {
            0 => {
                self.load_section = None;
                self.load_sections.clear();
            }
            1 => {
                self.load_section = Some(LoadSectionSettings::from(&sections[0]));
                self.load_sections.clear();
            }
            _ => {
                self.load_section = None;
                self.load_sections = sections.iter().map(LoadSectionSettings::from).collect();
            }
        }
    }

    // -- Multi-series settings helpers ------------------------------------

    /// Get persisted series entries.
    ///
    /// Returns `&[SeriesSettings]` — may be empty for legacy single-series
    /// files.
    pub fn get_series(&self) -> &[SeriesSettings] {
        &self.series
    }

    /// Populate `series` entries from a [`SeriesManager`].
    ///
    /// Also updates the top-level `load_section`/`load_sections` from the
    /// *active* series for backward compatibility (so older seaview versions
    /// can still read the file).
    pub fn set_series_from_manager(&mut self, mgr: &SeriesManager) {
        let active_id = mgr.active_id();

        self.series = mgr
            .iter()
            .map(|state| {
                let mut s = SeriesSettings::from(state);
                s.active = active_id == Some(state.id);
                s
            })
            .collect();

        // Backward compat: mirror the active series' load sections into
        // the top-level fields so older seaview versions still work.
        if let Some(active) = mgr.active() {
            self.set_load_sections(&active.timeline.sections);
        }
    }

    /// Check whether this settings file has multi-series entries.
    pub fn has_series(&self) -> bool {
        !self.series.is_empty()
    }
}

impl Settings {
    // -- migration (continued) --------------------------------------------

    /// Migrate legacy `[camera]` → `cameras["default"]`.
    ///
    /// Call this after deserializing. If `camera` is `Some` and `cameras`
    /// is empty, the single camera is moved into the map under `"default"`
    /// and `active_camera` is set.
    pub fn migrate(&mut self) {
        if let Some(legacy) = self.camera.take() {
            if self.cameras.is_empty() {
                self.cameras.insert("default".to_string(), legacy);
                if self.active_camera.is_none() {
                    self.active_camera = Some("default".to_string());
                }
            } else {
                // Both old and new present — put legacy back, prefer new map.
                // (Shouldn't happen in practice, but be defensive.)
                self.camera = Some(legacy);
            }
        }
    }

    // -- load / save ------------------------------------------------------

    /// Load settings from a `seaview.toml` file in the given directory.
    /// Returns `Ok(None)` if the file doesn't exist.
    /// Automatically runs migration on the loaded settings.
    pub fn load_from_dir(dir: &Path) -> Result<Option<Self>, SettingsError> {
        let path = dir.join(SETTINGS_FILENAME);
        if !path.exists() {
            return Ok(None);
        }
        let contents = std::fs::read_to_string(&path).map_err(|e| SettingsError::Io {
            path: path.clone(),
            source: e,
        })?;
        let mut settings: Settings =
            toml::from_str(&contents).map_err(|e| SettingsError::ParseToml {
                path: path.clone(),
                source: e,
            })?;

        // Run migration for old [camera] → cameras["default"]
        settings.migrate();

        debug!("Loaded settings from {:?}", path);
        Ok(Some(settings))
    }

    /// Save settings to `seaview.toml` in the given directory.
    /// Merges with existing file if present (preserves unknown keys).
    pub fn save_to_dir(&self, dir: &Path) -> Result<(), SettingsError> {
        let path = dir.join(SETTINGS_FILENAME);

        // If a file already exists, load it as a raw TOML table so we can merge
        let merged = if path.exists() {
            let existing_contents =
                std::fs::read_to_string(&path).map_err(|e| SettingsError::Io {
                    path: path.clone(),
                    source: e,
                })?;
            let mut existing_table: toml::Table =
                toml::from_str(&existing_contents).unwrap_or_default();

            // Serialize our settings to a table and merge
            let our_toml_str =
                toml::to_string_pretty(self).map_err(|e| SettingsError::SerializeToml {
                    path: path.clone(),
                    source: e,
                })?;
            let our_table: toml::Table = toml::from_str(&our_toml_str).unwrap_or_default();

            for (key, value) in our_table {
                existing_table.insert(key, value);
            }

            // Remove legacy [camera] key if we have [cameras.*] — clean up on save
            if existing_table.contains_key("cameras") {
                existing_table.remove("camera");
            }

            toml::to_string_pretty(&existing_table).map_err(|e| SettingsError::SerializeToml {
                path: path.clone(),
                source: e,
            })?
        } else {
            toml::to_string_pretty(self).map_err(|e| SettingsError::SerializeToml {
                path: path.clone(),
                source: e,
            })?
        };

        std::fs::write(&path, merged).map_err(|e| SettingsError::Io {
            path: path.clone(),
            source: e,
        })?;
        debug!("Saved settings to {:?}", path);
        Ok(())
    }

    // -- helpers ----------------------------------------------------------

    /// Resolve the active camera name, falling back to `"default"`.
    pub fn active_camera_name(&self) -> &str {
        self.active_camera.as_deref().unwrap_or("default")
    }

    /// Look up the active camera, falling back to the first entry, then `None`.
    pub fn active_camera_settings(&self) -> Option<&CameraSettings> {
        let name = self.active_camera_name();
        self.cameras
            .get(name)
            .or_else(|| self.cameras.values().next())
    }

    /// Save a camera transform under the given name.
    pub fn save_camera(&mut self, name: &str, transform: &Transform) {
        self.cameras
            .insert(name.to_string(), CameraSettings::from_transform(transform));
        self.active_camera = Some(name.to_string());
        // Clear legacy field
        self.camera = None;
    }

    /// Build and set environment from the current runtime resources.
    ///
    /// Saves environment light and material settings. The lighting rig is
    /// saved separately via [`Self::set_lighting_rig`].
    pub fn set_environment(
        &mut self,
        env_light_cfg: &crate::lib::lighting::EnvironmentLightConfig,
        material_cfg: &MaterialConfig,
    ) {
        let env = self
            .environment
            .get_or_insert_with(EnvironmentSettings::default);
        env.environment_light = Some(EnvironmentLightSettings::from(env_light_cfg));
        env.materials
            .insert("fluid".to_string(), MaterialSettings::from(material_cfg));
    }

    /// Save a [`LightingRig`] into the environment settings (v2 format).
    ///
    /// Clears the legacy `lighting` field so only `lighting_groups` is
    /// written to TOML.
    pub fn set_lighting_rig(&mut self, rig: &LightingRig) {
        let env = self
            .environment
            .get_or_insert_with(EnvironmentSettings::default);
        env.lighting_groups = lighting_rig_to_settings(rig);
        env.lighting = None; // drop legacy v1
    }

    /// Reconstruct a [`LightingRig`] from the persisted settings.
    ///
    /// Returns `None` if there are no `lighting_groups` in the TOML.
    pub fn get_lighting_rig(&self) -> Option<LightingRig> {
        let env = self.environment.as_ref()?;
        if env.lighting_groups.is_empty() {
            return None;
        }
        Some(settings_to_lighting_rig(&env.lighting_groups))
    }

    /// Convenience: set camera from a Transform under the active name.
    pub fn set_camera_from_transform(&mut self, transform: &Transform) {
        let name = self.active_camera_name().to_string();
        self.save_camera(&name, transform);
    }

    /// Merge playback settings
    pub fn set_playback(&mut self, speed: f32, loop_enabled: bool, watch: bool) {
        self.playback = Some(PlaybackSettings {
            speed: Some(speed),
            loop_enabled: Some(loop_enabled),
            watch: Some(watch),
        });
    }

    /// Save environment objects into the environment settings.
    pub fn set_environment_objects(&mut self, store: &EnvironmentObjectStore) {
        let env = self
            .environment
            .get_or_insert_with(EnvironmentSettings::default);
        env.objects = environment_objects_to_settings(store);
    }

    /// Reconstruct an [`EnvironmentObjectStore`] from persisted settings.
    ///
    /// Returns `None` if there are no objects in the TOML.
    pub fn get_environment_objects(&self) -> Option<EnvironmentObjectStore> {
        let env = self.environment.as_ref()?;
        if env.objects.is_empty() {
            return None;
        }
        Some(settings_to_environment_objects(&env.objects))
    }

    /// Build a [`SavedCameras`] resource from the current settings.
    pub fn to_saved_cameras(&self) -> SavedCameras {
        SavedCameras {
            cameras: self.cameras.clone(),
            active: self.active_camera_name().to_string(),
        }
    }
}

// ---------------------------------------------------------------------------
// SettingsResource (Bevy resource)
// ---------------------------------------------------------------------------

/// Bevy resource holding the loaded settings and the directory they came from
#[derive(Resource)]
pub struct SettingsResource {
    /// The loaded (or default) settings
    pub settings: Settings,
    /// The directory the settings file lives in (sequence directory)
    pub directory: Option<PathBuf>,
}

impl Default for SettingsResource {
    fn default() -> Self {
        Self {
            settings: Settings::default(),
            directory: None,
        }
    }
}

impl SettingsResource {
    /// Save current settings to disk
    pub fn save(&self) -> Result<(), SettingsError> {
        if let Some(ref dir) = self.directory {
            self.settings.save_to_dir(dir)
        } else {
            Err(SettingsError::NoDirectory)
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur during settings operations
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("I/O error at {path}: {source}")]
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("Failed to parse TOML at {path}: {source}")]
    ParseToml {
        path: PathBuf,
        source: toml::de::Error,
    },
    #[error("Failed to serialize TOML for {path}: {source}")]
    SerializeToml {
        path: PathBuf,
        source: toml::ser::Error,
    },
    #[error("No directory set - cannot save settings")]
    NoDirectory,
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

/// Resolve the settings directory from a path argument.
/// If the path is a file, returns its parent directory.
/// If the path is a directory, returns it directly.
pub fn resolve_settings_dir(path: &Path) -> Option<PathBuf> {
    let canonical = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    if canonical.is_dir() {
        Some(canonical)
    } else if canonical.is_file() {
        canonical.parent().map(|p| p.to_path_buf())
    } else {
        None
    }
}

/// Event sent when the user requests to save the current view
#[derive(Message)]
pub struct SaveViewEvent;

// ---------------------------------------------------------------------------
// Helper: extract (r, g, b, a) without relying on a method that may not exist
// ---------------------------------------------------------------------------

/// Helper to convert a Bevy `Color` to `[f32; 3]` sRGB.
fn color_to_srgb3(color: &Color) -> [f32; 3] {
    let srgba = color.to_srgba();
    [srgba.red, srgba.green, srgba.blue]
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lib::lighting::PlacementAlgorithm;

    fn placement_to_snake(alg: &PlacementAlgorithm) -> String {
        match alg {
            PlacementAlgorithm::UniformGrid => "uniform_grid",
            PlacementAlgorithm::HexagonalPacking => "hexagonal_packing",
            PlacementAlgorithm::PoissonDisk => "poisson_disk",
            PlacementAlgorithm::SingleRow => "single_row",
            PlacementAlgorithm::SingleColumn => "single_column",
        }
        .to_string()
    }

    fn snake_to_placement(s: &str) -> PlacementAlgorithm {
        match s {
            "uniform_grid" => PlacementAlgorithm::UniformGrid,
            "hexagonal_packing" => PlacementAlgorithm::HexagonalPacking,
            "poisson_disk" => PlacementAlgorithm::PoissonDisk,
            "single_row" => PlacementAlgorithm::SingleRow,
            "single_column" => PlacementAlgorithm::SingleColumn,
            _ => PlacementAlgorithm::UniformGrid,
        }
    }

    #[test]
    fn test_camera_roundtrip() {
        let transform = Transform::from_xyz(10.0, 20.0, 30.0).looking_at(Vec3::ZERO, Vec3::Y);
        let cam = CameraSettings::from_transform(&transform);
        let recovered = cam.to_transform();

        assert!((transform.translation - recovered.translation).length() < 0.001);
        let dot = transform.rotation.dot(recovered.rotation).abs();
        assert!(
            dot > 0.999,
            "rotation mismatch: dot={}, original={:?}, recovered={:?}",
            dot,
            transform.rotation,
            recovered.rotation
        );
    }

    #[test]
    fn test_settings_serialize_deserialize() {
        let mut cameras = HashMap::new();
        cameras.insert(
            "default".to_string(),
            CameraSettings {
                position: [1.0, 2.0, 3.0],
                rotation: [10.0, 20.0, 0.0],
            },
        );

        let settings = Settings {
            active_camera: Some("default".to_string()),
            cameras,
            camera: None,
            sequence: Some(SequenceSettings {
                source_coordinates: Some("fluidx3d".to_string()),
            }),
            playback: Some(PlaybackSettings {
                speed: Some(1.5),
                loop_enabled: Some(true),
                watch: Some(false),
            }),
            mesh: None,
            environment: None,
            load_section: None,
            load_sections: vec![],
            series: vec![],
        };

        let toml_str = toml::to_string_pretty(&settings).unwrap();
        let recovered: Settings = toml::from_str(&toml_str).unwrap();

        assert_eq!(recovered.active_camera.as_deref(), Some("default"));
        let cam = recovered.cameras.get("default").unwrap();
        assert_eq!(cam.position, [1.0, 2.0, 3.0]);
        assert_eq!(cam.rotation, [10.0, 20.0, 0.0]);

        let seq = recovered.sequence.unwrap();
        assert_eq!(seq.source_coordinates.unwrap(), "fluidx3d");

        let pb = recovered.playback.unwrap();
        assert_eq!(pb.speed.unwrap(), 1.5);
        assert_eq!(pb.loop_enabled.unwrap(), true);
    }

    #[test]
    fn test_settings_partial_toml() {
        // Old-style single [camera] still parses
        let toml_str = r#"
[camera]
position = [5.0, 10.0, 15.0]
rotation = [0.0, 45.0, 0.0]
"#;
        let settings: Settings = toml::from_str(toml_str).unwrap();
        assert!(settings.camera.is_some());
        assert!(settings.cameras.is_empty());
        assert!(settings.sequence.is_none());
        assert!(settings.playback.is_none());
        assert!(settings.environment.is_none());
    }

    #[test]
    fn test_migration_old_camera_to_cameras_map() {
        let toml_str = r#"
[camera]
position = [42.0, 0.0, -5.0]
rotation = [-15.0, 90.0, 0.0]

[playback]
speed = 2.0
"#;
        let mut settings: Settings = toml::from_str(toml_str).unwrap();
        assert!(settings.camera.is_some());
        assert!(settings.cameras.is_empty());

        settings.migrate();

        assert!(settings.camera.is_none());
        assert_eq!(settings.cameras.len(), 1);
        assert!(settings.cameras.contains_key("default"));
        assert_eq!(settings.active_camera.as_deref(), Some("default"));

        let cam = settings.cameras.get("default").unwrap();
        assert_eq!(cam.position, [42.0, 0.0, -5.0]);
        assert_eq!(cam.rotation, [-15.0, 90.0, 0.0]);
    }

    #[test]
    fn test_migration_does_not_overwrite_new_cameras() {
        // If both old [camera] and new [cameras.*] are present, keep both
        // (cameras map takes precedence, legacy field left alone)
        let toml_str = r#"
active_camera = "closeup"

[camera]
position = [1.0, 1.0, 1.0]
rotation = [0.0, 0.0, 0.0]

[cameras.closeup]
position = [10.0, 5.0, 15.0]
rotation = [-30.0, 90.0, 0.0]
"#;
        let mut settings: Settings = toml::from_str(toml_str).unwrap();
        settings.migrate();

        // cameras map already had entries, so legacy is left alone
        assert!(settings.camera.is_some());
        assert_eq!(settings.cameras.len(), 1);
        assert!(settings.cameras.contains_key("closeup"));
    }

    #[test]
    fn test_empty_toml() {
        let settings: Settings = toml::from_str("").unwrap();
        assert!(settings.camera.is_none());
        assert!(settings.cameras.is_empty());
        assert!(settings.active_camera.is_none());
        assert!(settings.environment.is_none());
    }

    #[test]
    fn test_lighting_settings_default_is_sane() {
        let settings = LightingSettings::default();
        assert_eq!(settings.num_lights, 9);
        assert_eq!(settings.height, 50.0);
        assert_eq!(settings.placement_algorithm, "uniform_grid");
        assert!(settings.enabled);
    }

    #[test]
    fn test_lighting_settings_toml_roundtrip() {
        let settings = LightingSettings {
            num_lights: 16,
            height: 80.0,
            cone_angle: 45.0,
            placement_algorithm: "hexagonal_packing".to_string(),
            enabled: true,
            global_lighting_enabled: false,
            intensity: 1_200_000.0,
            color: [1.0, 0.95, 0.85],
            range: 600.0,
            show_markers: false,
            marker_size: 0.5,
            coverage_pct: 150.0,
        };

        let toml_str = toml::to_string_pretty(&settings).unwrap();
        let recovered: LightingSettings = toml::from_str(&toml_str).unwrap();

        assert_eq!(recovered.num_lights, 16);
        assert_eq!(recovered.placement_algorithm, "hexagonal_packing");
        assert_eq!(recovered.color, [1.0, 0.95, 0.85]);
    }

    #[test]
    fn test_material_settings_roundtrip() {
        let cfg = MaterialConfig::default();
        let settings = MaterialSettings::from(&cfg);
        let recovered = MaterialConfig::from(&settings);

        // Compare base_color channels via sRGB
        let orig_srgb = color_to_srgb3(&cfg.base_color);
        let recv_srgb = color_to_srgb3(&recovered.base_color);
        for i in 0..3 {
            assert!(
                (orig_srgb[i] - recv_srgb[i]).abs() < 0.001,
                "base_color channel {} mismatch: {} vs {}",
                i,
                orig_srgb[i],
                recv_srgb[i]
            );
        }

        assert_eq!(cfg.perceptual_roughness, recovered.perceptual_roughness);
        assert_eq!(cfg.metallic, recovered.metallic);
        assert_eq!(cfg.reflectance, recovered.reflectance);
        assert_eq!(cfg.emissive_intensity, recovered.emissive_intensity);
        assert_eq!(cfg.double_sided, recovered.double_sided);
        assert_eq!(cfg.opacity, recovered.opacity);
    }

    #[test]
    fn test_material_settings_toml_roundtrip() {
        let settings = MaterialSettings {
            base_color: [0.3, 0.5, 0.8],
            perceptual_roughness: 0.4,
            metallic: 0.1,
            reflectance: 0.5,
            emissive: [0.0, 0.0, 0.0],
            emissive_intensity: 0.0,
            double_sided: true,
            alpha_mode: "opaque".to_string(),
            alpha_cutoff: 0.5,
        };

        let toml_str = toml::to_string_pretty(&settings).unwrap();
        let recovered: MaterialSettings = toml::from_str(&toml_str).unwrap();

        assert_eq!(recovered.base_color, [0.3, 0.5, 0.8]);
        assert_eq!(recovered.alpha_mode, "opaque");
    }

    #[test]
    fn test_environment_settings_toml() {
        let toml_str = r#"
[environment.lighting]
num_lights = 16
height = 80.0
cone_angle = 45.0
placement_algorithm = "hexagonal_packing"
enabled = true
global_lighting_enabled = false
intensity = 1200000.0
color = [1.0, 0.95, 0.85]
range = 600.0
show_markers = false
marker_size = 0.5
coverage_pct = 150.0

[environment.materials.fluid]
base_color = [0.3, 0.5, 0.8]
perceptual_roughness = 0.4
metallic = 0.1
reflectance = 0.5
emissive = [0.0, 0.0, 0.0]
emissive_intensity = 0.0
double_sided = true
alpha_mode = "opaque"
alpha_cutoff = 0.5
"#;

        let settings: Settings = toml::from_str(toml_str).unwrap();
        let env = settings.environment.unwrap();

        let lighting = env.lighting.unwrap();
        assert_eq!(lighting.num_lights, 16);
        assert_eq!(lighting.placement_algorithm, "hexagonal_packing");

        let fluid = env.materials.get("fluid").unwrap();
        assert_eq!(fluid.base_color, [0.3, 0.5, 0.8]);
        assert_eq!(fluid.alpha_mode, "opaque");
    }

    #[test]
    fn test_partial_lighting_fills_defaults() {
        let toml_str = r#"
[environment.lighting]
num_lights = 25
"#;

        let settings: Settings = toml::from_str(toml_str).unwrap();
        let env = settings.environment.unwrap();
        let lighting = env.lighting.unwrap();

        assert_eq!(lighting.num_lights, 25);
        // All other fields should be defaults
        let defaults = LightingSettings::default();
        assert_eq!(lighting.height, defaults.height);
        assert_eq!(lighting.cone_angle, defaults.cone_angle);
        assert_eq!(lighting.enabled, defaults.enabled);
    }

    #[test]
    fn test_partial_material_fills_defaults() {
        let toml_str = r#"
[environment.materials.fluid]
base_color = [1.0, 0.0, 0.0]
"#;

        let settings: Settings = toml::from_str(toml_str).unwrap();
        let env = settings.environment.unwrap();
        let fluid = env.materials.get("fluid").unwrap();

        assert_eq!(fluid.base_color, [1.0, 0.0, 0.0]);
        // Rest should be defaults
        let defaults = MaterialSettings::default();
        assert_eq!(fluid.perceptual_roughness, defaults.perceptual_roughness);
        assert_eq!(fluid.metallic, defaults.metallic);
    }

    #[test]
    fn test_multiple_cameras_toml() {
        let toml_str = r#"
active_camera = "closeup"

[cameras.default]
position = [100.0, 100.0, 100.0]
rotation = [0.0, 0.0, 0.0]

[cameras.closeup]
position = [10.0, 5.0, 15.0]
rotation = [-30.0, 90.0, 0.0]

[cameras.overhead]
position = [50.0, 300.0, 50.0]
rotation = [-89.0, 0.0, 0.0]
"#;

        let settings: Settings = toml::from_str(toml_str).unwrap();
        assert_eq!(settings.active_camera.as_deref(), Some("closeup"));
        assert_eq!(settings.cameras.len(), 3);
        assert!(settings.cameras.contains_key("default"));
        assert!(settings.cameras.contains_key("closeup"));
        assert!(settings.cameras.contains_key("overhead"));

        let closeup = settings.cameras.get("closeup").unwrap();
        assert_eq!(closeup.position, [10.0, 5.0, 15.0]);
    }

    #[test]
    fn test_active_camera_lookup() {
        let mut cameras = HashMap::new();
        cameras.insert(
            "overhead".to_string(),
            CameraSettings {
                position: [50.0, 300.0, 50.0],
                rotation: [-89.0, 0.0, 0.0],
            },
        );

        let settings = Settings {
            active_camera: Some("overhead".to_string()),
            cameras,
            ..Default::default()
        };

        let cam = settings.active_camera_settings().unwrap();
        assert_eq!(cam.position, [50.0, 300.0, 50.0]);
    }

    #[test]
    fn test_active_camera_fallback_to_first() {
        let mut cameras = HashMap::new();
        cameras.insert("myview".to_string(), CameraSettings::default());

        let settings = Settings {
            active_camera: Some("nonexistent".to_string()),
            cameras,
            ..Default::default()
        };

        // Should fall back to the first (only) entry
        assert!(settings.active_camera_settings().is_some());
    }

    #[test]
    fn test_active_camera_none_when_empty() {
        let settings = Settings::default();
        assert!(settings.active_camera_settings().is_none());
    }

    #[test]
    fn test_set_environment_helper() {
        let env_light = crate::lib::lighting::EnvironmentLightConfig::default();
        let material = MaterialConfig::default();

        let mut settings = Settings::default();
        settings.set_environment(&env_light, &material);

        let env = settings.environment.as_ref().unwrap();
        assert!(env.environment_light.is_some());
        assert!(env.materials.contains_key("fluid"));
    }

    #[test]
    fn test_save_camera_helper() {
        let mut settings = Settings::default();
        let transform = Transform::from_xyz(1.0, 2.0, 3.0);
        settings.save_camera("my_view", &transform);

        assert_eq!(settings.active_camera.as_deref(), Some("my_view"));
        assert!(settings.cameras.contains_key("my_view"));
        assert!(settings.camera.is_none()); // legacy cleared
    }

    #[test]
    fn test_full_toml_roundtrip() {
        let toml_str = r#"
active_camera = "default"

[cameras.default]
position = [120.5, 85.3, 200.0]
rotation = [-15.0, 45.0, 0.0]

[cameras.closeup]
position = [10.0, 5.0, 15.0]
rotation = [-30.0, 90.0, 0.0]

[sequence]
source_coordinates = "zup"

[playback]
speed = 1.5
loop = true

[mesh]
min = [0.0, 0.0, 0.0]
max = [75.0, 50.0, 75.0]
dimensions = [75.0, 50.0, 75.0]

[environment.lighting]
num_lights = 16
height = 80.0
cone_angle = 45.0
placement_algorithm = "hexagonal_packing"
enabled = true
global_lighting_enabled = false
intensity = 1200000.0
color = [1.0, 0.95, 0.85]
range = 600.0
show_markers = false
marker_size = 0.5
coverage_pct = 150.0

[environment.materials.fluid]
base_color = [0.3, 0.5, 0.8]
perceptual_roughness = 0.4
metallic = 0.1
reflectance = 0.5
emissive = [0.0, 0.0, 0.0]
emissive_intensity = 0.0
double_sided = true
alpha_mode = "opaque"
alpha_cutoff = 0.5
"#;

        let settings: Settings = toml::from_str(toml_str).unwrap();

        // Re-serialize
        let reserialized = toml::to_string_pretty(&settings).unwrap();

        // Parse again
        let recovered: Settings = toml::from_str(&reserialized).unwrap();

        assert_eq!(recovered.active_camera.as_deref(), Some("default"));
        assert_eq!(recovered.cameras.len(), 2);

        let env = recovered.environment.unwrap();
        let lighting = env.lighting.unwrap();
        assert_eq!(lighting.num_lights, 16);

        let fluid = env.materials.get("fluid").unwrap();
        assert_eq!(fluid.base_color, [0.3, 0.5, 0.8]);
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join("seaview_test_settings_v2");
        let _ = std::fs::create_dir_all(&dir);

        let mut cameras = HashMap::new();
        cameras.insert(
            "default".to_string(),
            CameraSettings {
                position: [42.0, 0.0, -5.0],
                rotation: [-15.0, 90.0, 0.0],
            },
        );

        let settings = Settings {
            active_camera: Some("default".to_string()),
            cameras,
            camera: None,
            sequence: None,
            playback: Some(PlaybackSettings {
                speed: Some(2.0),
                loop_enabled: Some(false),
                watch: Some(true),
            }),
            mesh: None,
            environment: Some(EnvironmentSettings {
                environment_light: Some(EnvironmentLightSettings::default()),
                lighting: Some(LightingSettings::default()),
                lighting_groups: vec![],
                materials: {
                    let mut m = HashMap::new();
                    m.insert("fluid".to_string(), MaterialSettings::default());
                    m
                },
                objects: vec![],
            }),
            load_section: None,
            load_sections: vec![],
            series: vec![],
        };

        settings.save_to_dir(&dir).unwrap();
        let loaded = Settings::load_from_dir(&dir).unwrap().unwrap();

        let cam = loaded.cameras.get("default").unwrap();
        assert_eq!(cam.position[0], 42.0);

        let pb = loaded.playback.unwrap();
        assert_eq!(pb.speed.unwrap(), 2.0);
        assert_eq!(pb.loop_enabled.unwrap(), false);

        let env = loaded.environment.unwrap();
        assert!(env.lighting.is_some());
        assert!(env.materials.contains_key("fluid"));

        // Clean up
        let _ = std::fs::remove_file(dir.join(SETTINGS_FILENAME));
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn test_materials_unknown_keys_preserved() {
        let toml_str = r#"
[environment.materials.fluid]
base_color = [0.3, 0.5, 0.8]
perceptual_roughness = 0.4
metallic = 0.1
reflectance = 0.5
emissive = [0.0, 0.0, 0.0]
emissive_intensity = 0.0
double_sided = true
alpha_mode = "opaque"
alpha_cutoff = 0.5

[environment.materials.ground]
base_color = [0.4, 0.35, 0.3]
perceptual_roughness = 0.9
metallic = 0.0
reflectance = 0.3
emissive = [0.0, 0.0, 0.0]
emissive_intensity = 0.0
double_sided = false
alpha_mode = "opaque"
alpha_cutoff = 0.5
"#;

        let settings: Settings = toml::from_str(toml_str).unwrap();
        let env = settings.environment.clone().unwrap();
        assert_eq!(env.materials.len(), 2);
        assert!(env.materials.contains_key("fluid"));
        assert!(env.materials.contains_key("ground"));

        // Re-serialize should keep both
        let reserialized = toml::to_string_pretty(&settings.clone()).unwrap();
        let recovered: Settings = toml::from_str(&reserialized).unwrap();
        assert_eq!(recovered.environment.unwrap().materials.len(), 2);
    }

    #[test]
    fn test_placement_algorithm_snake_roundtrip() {
        for alg in PlacementAlgorithm::all() {
            let snake = placement_to_snake(alg);
            let recovered = snake_to_placement(&snake);
            assert_eq!(*alg, recovered, "failed for {:?} → {:?}", alg, snake);
        }
    }

    #[test]
    fn test_placement_algorithm_unknown_falls_back() {
        let recovered = snake_to_placement("some_future_algorithm");
        assert_eq!(recovered, PlacementAlgorithm::UniformGrid);
    }

    #[test]
    fn test_to_saved_cameras() {
        let mut cameras = HashMap::new();
        cameras.insert("a".to_string(), CameraSettings::default());
        cameras.insert("b".to_string(), CameraSettings::default());

        let settings = Settings {
            active_camera: Some("b".to_string()),
            cameras,
            ..Default::default()
        };

        let saved = settings.to_saved_cameras();
        assert_eq!(saved.active, "b");
        assert_eq!(saved.cameras.len(), 2);
    }

    #[test]
    fn test_backward_compat_old_file_no_environment() {
        // Simulates an old seaview.toml that has no environment or cameras sections
        let toml_str = r#"
[camera]
position = [100.0, 50.0, 200.0]
rotation = [-10.0, 30.0, 0.0]

[sequence]
source_coordinates = "zup"

[playback]
speed = 1.0
loop = false

[mesh]
min = [0.0, 0.0, 0.0]
max = [75.0, 50.0, 75.0]
dimensions = [75.0, 50.0, 75.0]
"#;

        let mut settings: Settings = toml::from_str(toml_str).unwrap();
        settings.migrate();

        // camera migrated
        assert!(settings.camera.is_none());
        assert_eq!(settings.cameras.len(), 1);
        let cam = settings.cameras.get("default").unwrap();
        assert_eq!(cam.position, [100.0, 50.0, 200.0]);

        // No environment — should be None, not an error
        assert!(settings.environment.is_none());

        // Other fields intact
        assert!(settings.sequence.is_some());
        assert!(settings.playback.is_some());
        assert!(settings.mesh.is_some());
    }

    // -----------------------------------------------------------------------
    // Lighting v2 settings
    // -----------------------------------------------------------------------

    #[test]
    fn test_light_settings_roundtrip() {
        let light = Light::new("Spot A").with_position(Vec2::new(10.0, -20.0));
        let settings = LightSettings::from(&light);
        let recovered = Light::from(&settings);

        assert_eq!(recovered.name, "Spot A");
        assert_eq!(recovered.position, Vec2::new(10.0, -20.0));
        assert_eq!(recovered.height, light.height);
        assert_eq!(recovered.intensity, light.intensity);
        assert_eq!(recovered.range, light.range);
        assert_eq!(recovered.cone_angle, light.cone_angle);
        assert_eq!(recovered.visible, light.visible);
        // id is regenerated — just check it's valid
        assert_ne!(recovered.id, Uuid::nil());
    }

    #[test]
    fn test_light_settings_toml_roundtrip() {
        let settings = LightSettings {
            name: "Test Light".to_string(),
            position: [-5.0, 12.5],
            height: 30.0,
            intensity: 500_000.0,
            range: 300.0,
            cone_angle: 45.0,
            color: [1.0, 0.8, 0.6],
            visible: false,
        };

        let toml_str = toml::to_string_pretty(&settings).unwrap();
        let recovered: LightSettings = toml::from_str(&toml_str).unwrap();

        assert_eq!(recovered.name, "Test Light");
        assert_eq!(recovered.position, [-5.0, 12.5]);
        assert_eq!(recovered.height, 30.0);
        assert_eq!(recovered.intensity, 500_000.0);
        assert_eq!(recovered.cone_angle, 45.0);
        assert_eq!(recovered.color, [1.0, 0.8, 0.6]);
        assert!(!recovered.visible);
    }

    #[test]
    fn test_light_group_settings_roundtrip() {
        let mut group = LightGroup::new("Overhead Grid");
        group.add_light(Light::new("L1").with_position(Vec2::new(1.0, 2.0)));
        group.add_light(Light::new("L2").with_position(Vec2::new(3.0, 4.0)));
        group.visible = true;

        let settings = LightGroupSettings::from(&group);
        let recovered = LightGroup::from(&settings);

        assert_eq!(recovered.name, "Overhead Grid");
        assert!(recovered.visible);
        assert_eq!(recovered.lights.len(), 2);
        assert_eq!(recovered.lights[0].name, "L1");
        assert_eq!(recovered.lights[0].position, Vec2::new(1.0, 2.0));
        assert_eq!(recovered.lights[1].name, "L2");
    }

    #[test]
    fn test_light_group_settings_toml_roundtrip() {
        let settings = LightGroupSettings {
            name: "Ring".to_string(),
            visible: true,
            lights: vec![
                LightSettings {
                    name: "R1".to_string(),
                    position: [0.0, 10.0],
                    ..Default::default()
                },
                LightSettings {
                    name: "R2".to_string(),
                    position: [10.0, 0.0],
                    ..Default::default()
                },
            ],
        };

        let toml_str = toml::to_string_pretty(&settings).unwrap();
        let recovered: LightGroupSettings = toml::from_str(&toml_str).unwrap();

        assert_eq!(recovered.name, "Ring");
        assert!(recovered.visible);
        assert_eq!(recovered.lights.len(), 2);
        assert_eq!(recovered.lights[0].name, "R1");
        assert_eq!(recovered.lights[1].name, "R2");
    }

    #[test]
    fn test_lighting_rig_settings_roundtrip() {
        let mut rig = LightingRig::new();
        let mut g1 = LightGroup::new("Grid A");
        g1.add_light(Light::new("A1").with_position(Vec2::new(1.0, 2.0)));
        g1.add_light(Light::new("A2").with_position(Vec2::new(3.0, 4.0)));
        rig.add_group(g1);

        let mut g2 = LightGroup::new("Grid B");
        g2.visible = false;
        g2.add_light(Light::new("B1"));
        rig.add_group(g2);

        let group_settings = lighting_rig_to_settings(&rig);
        let recovered = settings_to_lighting_rig(&group_settings);

        assert_eq!(recovered.groups.len(), 2);
        assert_eq!(recovered.groups[0].name, "Grid A");
        assert!(recovered.groups[0].visible);
        assert_eq!(recovered.groups[0].lights.len(), 2);
        assert_eq!(recovered.groups[0].lights[0].name, "A1");
        assert_eq!(recovered.groups[1].name, "Grid B");
        assert!(!recovered.groups[1].visible);
        assert_eq!(recovered.groups[1].lights.len(), 1);
        assert!(recovered.active_group.is_none());
    }

    #[test]
    fn test_lighting_rig_full_toml_roundtrip() {
        let toml_str = r#"
[[environment.lighting_groups]]
name = "Overhead Grid"
visible = true

[[environment.lighting_groups.lights]]
name = "Light 1"
position = [-10.0, 25.0]
height = 50.0
intensity = 800000.0
range = 500.0
cone_angle = 60.0
color = [1.0, 0.95, 0.9]
visible = true

[[environment.lighting_groups.lights]]
name = "Light 2"
position = [10.0, 25.0]
height = 50.0
intensity = 800000.0
range = 500.0
cone_angle = 60.0
color = [1.0, 0.95, 0.9]
visible = false

[[environment.lighting_groups]]
name = "Accent"
visible = false

[[environment.lighting_groups.lights]]
name = "Spot A"
position = [0.0, 0.0]
height = 30.0
intensity = 400000.0
range = 200.0
cone_angle = 45.0
color = [0.8, 0.8, 1.0]
visible = true
"#;

        let settings: Settings = toml::from_str(toml_str).unwrap();
        let rig = settings.get_lighting_rig().unwrap();

        assert_eq!(rig.groups.len(), 2);
        assert_eq!(rig.groups[0].name, "Overhead Grid");
        assert!(rig.groups[0].visible);
        assert_eq!(rig.groups[0].lights.len(), 2);
        assert_eq!(rig.groups[0].lights[0].name, "Light 1");
        assert_eq!(rig.groups[0].lights[0].position, Vec2::new(-10.0, 25.0));
        assert!(rig.groups[0].lights[0].visible);
        assert!(!rig.groups[0].lights[1].visible);

        assert_eq!(rig.groups[1].name, "Accent");
        assert!(!rig.groups[1].visible);
        assert_eq!(rig.groups[1].lights[0].intensity, 400_000.0);

        // Re-serialize and verify structure survives
        let mut roundtrip_settings = Settings::default();
        roundtrip_settings.set_lighting_rig(&rig);
        let toml_out = toml::to_string_pretty(&roundtrip_settings).unwrap();
        let reloaded: Settings = toml::from_str(&toml_out).unwrap();
        let rig2 = reloaded.get_lighting_rig().unwrap();

        assert_eq!(rig2.groups.len(), 2);
        assert_eq!(rig2.groups[0].lights.len(), 2);
        assert_eq!(rig2.groups[0].lights[0].name, "Light 1");
    }

    #[test]
    fn test_set_lighting_rig_clears_legacy() {
        let mut settings = Settings::default();
        // Manually set legacy v1 lighting field
        let env = settings
            .environment
            .get_or_insert_with(EnvironmentSettings::default);
        env.lighting = Some(LightingSettings::default());
        assert!(settings.environment.as_ref().unwrap().lighting.is_some());

        // Now set v2 rig — should clear legacy
        let mut rig = LightingRig::new();
        rig.add_group(LightGroup::new("Test"));
        settings.set_lighting_rig(&rig);

        let env = settings.environment.as_ref().unwrap();
        assert!(env.lighting.is_none(), "legacy lighting should be cleared");
        assert_eq!(env.lighting_groups.len(), 1);
    }

    #[test]
    fn test_get_lighting_rig_returns_none_when_empty() {
        let settings = Settings::default();
        assert!(settings.get_lighting_rig().is_none());

        // Also None when environment exists but no groups
        let mut settings2 = Settings::default();
        settings2.environment = Some(EnvironmentSettings::default());
        assert!(settings2.get_lighting_rig().is_none());
    }

    #[test]
    fn test_lighting_groups_omitted_when_empty() {
        let mut settings = Settings::default();
        settings.environment = Some(EnvironmentSettings {
            environment_light: Some(EnvironmentLightSettings::default()),
            lighting: None,
            lighting_groups: vec![],
            materials: HashMap::new(),
            objects: vec![],
        });

        let toml_str = toml::to_string_pretty(&settings).unwrap();
        assert!(
            !toml_str.contains("lighting_groups"),
            "empty lighting_groups should be omitted from TOML, got:\n{}",
            toml_str
        );
    }

    #[test]
    fn test_legacy_v1_toml_still_loads() {
        let toml_str = r#"
[environment.lighting]
num_lights = 9
height = 50.0
cone_angle = 60.0
placement_algorithm = "uniform_grid"
enabled = true
intensity = 800000.0
color = [1.0, 0.95, 0.9]
range = 500.0
show_markers = true
marker_size = 0.5
coverage_pct = 120.0
"#;

        let settings: Settings = toml::from_str(toml_str).unwrap();
        let env = settings.environment.as_ref().unwrap();

        // Legacy field still accessible
        assert!(env.lighting.is_some());
        assert_eq!(env.lighting.as_ref().unwrap().num_lights, 9);

        // No v2 groups
        assert!(env.lighting_groups.is_empty());
        assert!(settings.get_lighting_rig().is_none());
    }

    // -----------------------------------------------------------------------
    // LoadSectionSettings
    // -----------------------------------------------------------------------

    #[test]
    fn test_load_section_settings_default() {
        let ls = LoadSectionSettings::default();
        assert_eq!(ls.start, 0);
        assert_eq!(ls.end, 100);
        assert_eq!(ls.frame_count(), 100);
    }

    #[test]
    fn test_load_section_settings_roundtrip() {
        let ls = LoadSectionSettings::new(50, 200);
        let toml_str = toml::to_string_pretty(&ls).unwrap();
        let recovered: LoadSectionSettings = toml::from_str(&toml_str).unwrap();
        assert_eq!(recovered.start, 50);
        assert_eq!(recovered.end, 200);
    }

    #[test]
    fn test_load_section_settings_in_settings() {
        let settings = Settings {
            load_section: Some(LoadSectionSettings::new(10, 300)),
            ..Default::default()
        };

        let toml_str = toml::to_string_pretty(&settings).unwrap();
        assert!(toml_str.contains("[load_section]"));
        assert!(toml_str.contains("start = 10"));
        assert!(toml_str.contains("end = 300"));

        let recovered: Settings = toml::from_str(&toml_str).unwrap();
        let ls = recovered.load_section.unwrap();
        assert_eq!(ls.start, 10);
        assert_eq!(ls.end, 300);
    }

    #[test]
    fn test_load_section_settings_none_omitted() {
        let settings = Settings {
            load_section: None,
            ..Default::default()
        };
        let toml_str = toml::to_string_pretty(&settings).unwrap();
        assert!(!toml_str.contains("load_section"));
    }

    #[test]
    fn test_load_section_settings_backward_compat() {
        // Old files without load_section should parse fine
        let toml_str = r#"
[cameras.default]
position = [1.0, 2.0, 3.0]
rotation = [0.0, 0.0, 0.0]
"#;
        let settings: Settings = toml::from_str(toml_str).unwrap();
        assert!(settings.load_section.is_none());
    }

    #[test]
    fn test_load_section_settings_contains() {
        let ls = LoadSectionSettings::new(10, 50);
        assert!(!ls.contains(9));
        assert!(ls.contains(10));
        assert!(ls.contains(49));
        assert!(!ls.contains(50));
    }

    #[test]
    fn test_load_section_settings_clamp() {
        let mut ls = LoadSectionSettings::new(0, 500);
        ls.clamp(200, 100);
        assert_eq!(ls.end, 100);
        assert_eq!(ls.start, 0);
        assert_eq!(ls.frame_count(), 100);
    }

    // ── Environment object persistence ────────────────────────────────

    #[test]
    fn test_env_object_settings_roundtrip() {
        use crate::lib::environment_objects::{EnvironmentObject, MeshEntry, Transform3D};

        let mut obj = EnvironmentObject::new("/models/fountain.glb");
        obj.name = "My Fountain".to_string();
        obj.visible = false;
        obj.textures_enabled = false;
        obj.transform = Transform3D::default()
            .with_position(1.0, 2.0, 3.0)
            .with_rotation_degrees(45.0, 90.0, 0.0)
            .with_uniform_scale(0.5);
        obj.meshes = vec![
            MeshEntry {
                name: "Base".to_string(),
                visible: true,
            },
            MeshEntry {
                name: "Water".to_string(),
                visible: false,
            },
        ];

        let settings = EnvironmentObjectSettings::from(&obj);
        assert_eq!(settings.path, "/models/fountain.glb");
        assert_eq!(settings.name.as_deref(), Some("My Fountain"));
        assert!(!settings.visible);
        assert!(!settings.textures_enabled);
        assert_eq!(settings.position, [1.0, 2.0, 3.0]);
        assert_eq!(settings.rotation_degrees, [45.0, 90.0, 0.0]);
        assert_eq!(settings.scale, [0.5, 0.5, 0.5]);
        assert_eq!(settings.mesh_visibility.len(), 2);
        assert!(settings.mesh_visibility[0].visible);
        assert!(!settings.mesh_visibility[1].visible);

        // Convert back to runtime type.
        let restored = EnvironmentObject::from(&settings);
        assert_eq!(restored.name, "My Fountain");
        assert_eq!(restored.path.to_str().unwrap(), "/models/fountain.glb");
        assert!(!restored.visible);
        assert!(!restored.textures_enabled);
        assert_eq!(restored.transform.position, [1.0, 2.0, 3.0]);
        assert_eq!(restored.transform.scale, [0.5, 0.5, 0.5]);
        assert_eq!(restored.meshes.len(), 2);
        assert_eq!(restored.meshes[0].name, "Base");
        assert!(restored.meshes[0].visible);
        assert_eq!(restored.meshes[1].name, "Water");
        assert!(!restored.meshes[1].visible);
    }

    #[test]
    fn test_env_object_settings_toml_roundtrip() {
        let settings = EnvironmentObjectSettings {
            path: "/home/user/models/building.glb".to_string(),
            name: Some("Office".to_string()),
            visible: true,
            textures_enabled: true,
            position: [10.0, 0.0, -5.0],
            rotation_degrees: [0.0, 0.0, 0.0],
            scale: [1.0, 1.0, 1.0],
            mesh_visibility: vec![MeshVisibilitySettings {
                name: "Roof".to_string(),
                visible: false,
            }],
            flip_normals: false,
            double_sided: false,
            coordinate_preset: default_coordinate_preset(),
            winding_order: default_winding_order(),
            material_mode: default_material_mode(),
            material_color: default_material_color(),
            material_roughness: default_material_roughness(),
            material_metallic: 0.0,
            swizzle_x: default_swizzle_x(),
            swizzle_y: default_swizzle_y(),
            swizzle_z: default_swizzle_z(),
        };

        let mut env_settings = EnvironmentSettings::default();
        env_settings.objects = vec![settings];

        let mut s = Settings::default();
        s.environment = Some(env_settings);

        let toml_str = toml::to_string_pretty(&s).unwrap();
        assert!(
            toml_str.contains("building.glb"),
            "path should appear in TOML:\n{toml_str}"
        );
        assert!(
            toml_str.contains("Office"),
            "name should appear in TOML:\n{toml_str}"
        );

        // Rotation and scale at defaults should be omitted.
        assert!(
            !toml_str.contains("rotation_degrees"),
            "zero rotation should be omitted:\n{toml_str}"
        );
        assert!(
            !toml_str.contains("scale"),
            "identity scale should be omitted:\n{toml_str}"
        );

        // Position is non-zero, should be present.
        assert!(
            toml_str.contains("position"),
            "non-zero position should be present:\n{toml_str}"
        );

        // Parse back.
        let loaded: Settings = toml::from_str(&toml_str).unwrap();
        let env = loaded.environment.unwrap();
        assert_eq!(env.objects.len(), 1);
        assert_eq!(env.objects[0].path, "/home/user/models/building.glb");
        assert_eq!(env.objects[0].name.as_deref(), Some("Office"));
        assert_eq!(env.objects[0].position, [10.0, 0.0, -5.0]);
        // Omitted fields should deserialize to defaults.
        assert_eq!(env.objects[0].rotation_degrees, [0.0, 0.0, 0.0]);
        assert_eq!(env.objects[0].scale, [1.0, 1.0, 1.0]);
        assert_eq!(env.objects[0].mesh_visibility.len(), 1);
        assert_eq!(env.objects[0].mesh_visibility[0].name, "Roof");
        assert!(!env.objects[0].mesh_visibility[0].visible);
    }

    #[test]
    fn test_env_objects_store_roundtrip() {
        use crate::lib::environment_objects::{EnvironmentObject, EnvironmentObjectStore};

        let mut store = EnvironmentObjectStore::new();
        let mut obj1 = EnvironmentObject::new("/a.glb");
        obj1.name = "Alpha".to_string();
        obj1.visible = false;
        let mut obj2 = EnvironmentObject::new("/b.glb");
        obj2.name = "Beta".to_string();
        obj2.transform = Transform3D::default().with_position(5.0, 0.0, 0.0);
        store.add(obj1);
        store.add(obj2);

        let settings_vec = environment_objects_to_settings(&store);
        assert_eq!(settings_vec.len(), 2);

        let restored = settings_to_environment_objects(&settings_vec);
        assert_eq!(restored.len(), 2);

        let items: Vec<_> = restored.iter().collect();
        assert_eq!(items[0].name, "Alpha");
        assert!(!items[0].visible);
        assert_eq!(items[1].name, "Beta");
        assert_eq!(items[1].transform.position, [5.0, 0.0, 0.0]);
    }

    #[test]
    fn test_env_objects_set_get_on_settings() {
        use crate::lib::environment_objects::{EnvironmentObject, EnvironmentObjectStore};

        let mut store = EnvironmentObjectStore::new();
        store.add(EnvironmentObject::new("/test.glb"));

        let mut settings = Settings::default();
        settings.set_environment_objects(&store);

        let restored = settings.get_environment_objects().unwrap();
        assert_eq!(restored.len(), 1);
        assert_eq!(restored.iter().next().unwrap().name, "test");
    }

    #[test]
    fn test_env_objects_get_returns_none_when_empty() {
        let settings = Settings::default();
        assert!(settings.get_environment_objects().is_none());
    }

    #[test]
    fn test_env_objects_empty_omitted_from_toml() {
        let mut settings = Settings::default();
        settings.environment = Some(EnvironmentSettings::default());

        let toml_str = toml::to_string_pretty(&settings).unwrap();
        assert!(
            !toml_str.contains("objects"),
            "empty objects should be omitted:\n{toml_str}"
        );
    }

    #[test]
    fn test_env_object_name_falls_back_to_stem() {
        let settings = EnvironmentObjectSettings {
            path: "/some/dir/cool_mesh.glb".to_string(),
            name: None,
            visible: true,
            textures_enabled: true,
            position: [0.0; 3],
            rotation_degrees: [0.0; 3],
            scale: [1.0, 1.0, 1.0],
            mesh_visibility: vec![],
            flip_normals: false,
            double_sided: false,
            coordinate_preset: default_coordinate_preset(),
            winding_order: default_winding_order(),
            material_mode: default_material_mode(),
            material_color: default_material_color(),
            material_roughness: default_material_roughness(),
            material_metallic: 0.0,
            swizzle_x: default_swizzle_x(),
            swizzle_y: default_swizzle_y(),
            swizzle_z: default_swizzle_z(),
        };

        let obj = EnvironmentObject::from(&settings);
        assert_eq!(obj.name, "cool_mesh");
    }

    #[test]
    fn test_env_object_save_load_file_roundtrip() {
        use crate::lib::environment_objects::{EnvironmentObject, EnvironmentObjectStore};

        let dir = tempfile::tempdir().unwrap();

        let mut store = EnvironmentObjectStore::new();
        let mut obj = EnvironmentObject::new("/models/statue.glb");
        obj.transform = Transform3D::default()
            .with_position(1.0, 2.0, 3.0)
            .with_rotation_degrees(0.0, 180.0, 0.0);
        store.add(obj);

        let mut settings = Settings::default();
        settings.set_environment_objects(&store);
        settings.save_to_dir(dir.path()).unwrap();

        let loaded = Settings::load_from_dir(dir.path()).unwrap().unwrap();
        let restored = loaded.get_environment_objects().unwrap();
        assert_eq!(restored.len(), 1);

        let item = restored.iter().next().unwrap();
        assert_eq!(item.name, "statue");
        assert_eq!(item.transform.position, [1.0, 2.0, 3.0]);
        assert_eq!(item.transform.rotation_degrees, [0.0, 180.0, 0.0]);
        assert_eq!(item.transform.scale, [1.0, 1.0, 1.0]);
    }

    // -- Series settings --------------------------------------------------

    #[test]
    fn test_series_settings_default() {
        let s = SeriesSettings::default();
        assert_eq!(s.name, "");
        assert!(s.path.is_none());
        assert!(!s.active);
        assert_eq!(s.playhead, 0);
        assert!(s.load_sections.is_empty());
    }

    #[test]
    fn test_series_settings_toml_roundtrip() {
        let series = SeriesSettings {
            name: "splash_v3".to_string(),
            path: Some("/data/sims/splash_v3/meshes".to_string()),
            active: true,
            playhead: 42,
            load_sections: vec![
                LoadSectionSettings::new(0, 100),
                LoadSectionSettings::new(200, 350),
            ],
        };

        let toml_str = toml::to_string_pretty(&series).unwrap();
        assert!(toml_str.contains("splash_v3"));
        assert!(toml_str.contains("42"));

        let recovered: SeriesSettings = toml::from_str(&toml_str).unwrap();
        assert_eq!(recovered.name, "splash_v3");
        assert_eq!(recovered.path.as_deref(), Some("/data/sims/splash_v3/meshes"));
        assert!(recovered.active);
        assert_eq!(recovered.playhead, 42);
        assert_eq!(recovered.load_sections.len(), 2);
        assert_eq!(recovered.load_sections[0].start, 0);
        assert_eq!(recovered.load_sections[0].end, 100);
        assert_eq!(recovered.load_sections[1].start, 200);
        assert_eq!(recovered.load_sections[1].end, 350);
    }

    #[test]
    fn test_series_in_settings_toml_roundtrip() {
        let mut settings = Settings::default();
        settings.series = vec![
            SeriesSettings {
                name: "alpha".to_string(),
                path: Some("/tmp/alpha".to_string()),
                active: true,
                playhead: 10,
                load_sections: vec![LoadSectionSettings::new(0, 50)],
            },
            SeriesSettings {
                name: "beta".to_string(),
                path: Some("/tmp/beta".to_string()),
                active: false,
                playhead: 0,
                load_sections: vec![LoadSectionSettings::new(0, 200)],
            },
        ];

        let toml_str = toml::to_string_pretty(&settings).unwrap();
        assert!(toml_str.contains("[[series]]"), "should have [[series]] tables:\n{toml_str}");
        assert!(toml_str.contains("alpha"));
        assert!(toml_str.contains("beta"));

        let recovered: Settings = toml::from_str(&toml_str).unwrap();
        assert_eq!(recovered.series.len(), 2);
        assert_eq!(recovered.series[0].name, "alpha");
        assert!(recovered.series[0].active);
        assert_eq!(recovered.series[0].playhead, 10);
        assert_eq!(recovered.series[1].name, "beta");
        assert!(!recovered.series[1].active);
    }

    #[test]
    fn test_series_empty_omitted_from_toml() {
        let settings = Settings::default();
        let toml_str = toml::to_string_pretty(&settings).unwrap();
        assert!(
            !toml_str.contains("series"),
            "empty series should be omitted:\n{toml_str}"
        );
    }

    #[test]
    fn test_series_backward_compat_no_series_field() {
        // Simulates an old seaview.toml with no [[series]] at all —
        // just top-level load_section.
        let toml_str = r#"
[load_section]
start = 10
end = 200
"#;
        let settings: Settings = toml::from_str(toml_str).unwrap();
        assert!(settings.series.is_empty());
        assert!(!settings.has_series());

        // Legacy path should still work
        let sections = settings.get_load_sections();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].start, 10);
        assert_eq!(sections[0].end, 200);
    }

    #[test]
    fn test_series_settings_get_load_sections() {
        let series = SeriesSettings {
            name: "test".to_string(),
            load_sections: vec![
                LoadSectionSettings::new(5, 50),
                LoadSectionSettings::new(100, 200),
            ],
            ..Default::default()
        };

        let sections = series.get_load_sections();
        assert_eq!(sections.len(), 2);
        assert_eq!(sections[0].start, 5);
        assert_eq!(sections[0].end, 50);
        assert!(sections[0].active);
        assert_eq!(sections[1].start, 100);
        assert_eq!(sections[1].end, 200);
    }

    #[test]
    fn test_set_series_from_manager() {
        use crate::lib::coordinates::SourceOrientation;
        use crate::lib::sequence::{FrameInfo, Sequence};
        use crate::lib::series::SeriesManager;

        let mut mgr = SeriesManager::new();

        // Create two sequences
        let mut seq1 = Sequence::new(
            "alpha".to_string(),
            std::path::PathBuf::from("/tmp/alpha"),
            "frame_*.stl".to_string(),
            SourceOrientation::default(),
        );
        for i in 0..100 {
            seq1.frames.push(FrameInfo {
                path: std::path::PathBuf::from(format!("/tmp/alpha/frame_{:04}.stl", i)),
                frame_number: i,
                filename: format!("frame_{:04}.stl", i),
                file_size: 1024,
            });
        }

        let mut seq2 = Sequence::new(
            "beta".to_string(),
            std::path::PathBuf::from("/tmp/beta"),
            "frame_*.stl".to_string(),
            SourceOrientation::default(),
        );
        for i in 0..200 {
            seq2.frames.push(FrameInfo {
                path: std::path::PathBuf::from(format!("/tmp/beta/frame_{:04}.stl", i)),
                frame_number: i,
                filename: format!("frame_{:04}.stl", i),
                file_size: 1024,
            });
        }

        let _id1 = mgr.add(seq1, 40);
        let _id2 = mgr.add(seq2, 60);

        // Advance playhead on the active series
        mgr.active_mut().unwrap().timeline.playhead = 15;

        let mut settings = Settings::default();
        settings.set_series_from_manager(&mgr);

        assert_eq!(settings.series.len(), 2);

        // First series (alpha) is active
        assert_eq!(settings.series[0].name, "alpha");
        assert!(settings.series[0].active);
        assert_eq!(settings.series[0].playhead, 15);
        assert_eq!(settings.series[0].load_sections.len(), 1);
        assert_eq!(settings.series[0].load_sections[0].start, 0);
        assert_eq!(settings.series[0].load_sections[0].end, 40);

        // Second series (beta) is not active
        assert_eq!(settings.series[1].name, "beta");
        assert!(!settings.series[1].active);
        assert_eq!(settings.series[1].playhead, 0);
        assert_eq!(settings.series[1].load_sections[0].end, 60);

        // Backward compat: top-level load_section mirrors the active series
        let top_level = settings.get_load_sections();
        assert_eq!(top_level.len(), 1);
        assert_eq!(top_level[0].start, 0);
        assert_eq!(top_level[0].end, 40);
    }

    #[test]
    fn test_set_series_from_manager_empty() {
        use crate::lib::series::SeriesManager;

        let mgr = SeriesManager::new();
        let mut settings = Settings::default();
        settings.set_series_from_manager(&mgr);

        assert!(settings.series.is_empty());
    }

    #[test]
    fn test_series_settings_from_series_state() {
        use crate::lib::coordinates::SourceOrientation;
        use crate::lib::sequence::{FrameInfo, Sequence};
        use crate::lib::series::{SeriesId, SeriesState};

        let mut seq = Sequence::new(
            "test_seq".to_string(),
            std::path::PathBuf::from("/data/test"),
            "mesh_*.stl".to_string(),
            SourceOrientation::default(),
        );
        for i in 0..50 {
            seq.frames.push(FrameInfo {
                path: std::path::PathBuf::from(format!("/data/test/mesh_{:04}.stl", i)),
                frame_number: i,
                filename: format!("mesh_{:04}.stl", i),
                file_size: 512,
            });
        }

        let mut state = SeriesState::new(SeriesId::from_raw(0), seq, 30);
        state.timeline.playhead = 12;
        state.label = "My Series".to_string();

        let ss = SeriesSettings::from(&state);
        assert_eq!(ss.name, "My Series");
        assert_eq!(ss.path.as_deref(), Some("/data/test"));
        assert!(!ss.active); // caller sets this
        assert_eq!(ss.playhead, 12);
        assert_eq!(ss.load_sections.len(), 1);
        assert_eq!(ss.load_sections[0].start, 0);
        assert_eq!(ss.load_sections[0].end, 30);
    }

    #[test]
    fn test_series_settings_toml_roundtrip_full_settings() {
        // Ensure [[series]] survives a full Settings save/load cycle
        let dir = tempfile::tempdir().unwrap();
        let mut settings = Settings::default();
        settings.series = vec![
            SeriesSettings {
                name: "run_a".to_string(),
                path: Some("/sims/run_a".to_string()),
                active: true,
                playhead: 50,
                load_sections: vec![LoadSectionSettings::new(0, 100)],
            },
        ];
        // Also set backward-compat top-level sections
        settings.load_section = Some(LoadSectionSettings::new(0, 100));

        settings.save_to_dir(dir.path()).unwrap();

        let loaded = Settings::load_from_dir(dir.path()).unwrap().unwrap();
        assert!(loaded.has_series());
        assert_eq!(loaded.series.len(), 1);
        assert_eq!(loaded.series[0].name, "run_a");
        assert!(loaded.series[0].active);
        assert_eq!(loaded.series[0].playhead, 50);

        // Top-level still present for backward compat
        let top = loaded.get_load_sections();
        assert_eq!(top.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Integration: full save → disk → load → restore round-trip
    // -----------------------------------------------------------------------

    #[test]
    fn test_multi_series_full_roundtrip() {
        use crate::lib::sequence::{FrameInfo, LoadSection, Sequence, Timeline};
        use crate::lib::series::{SavedSeriesEntry, SeriesManager};

        let dir = tempfile::tempdir().unwrap();

        // -- Step 1: Build a SeriesManager with two series --------------------
        let make_sequence = |name: &str, base: &str, n: usize| -> Sequence {
            let base_dir = std::path::PathBuf::from(base);
            let frames: Vec<FrameInfo> = (0..n)
                .map(|i| {
                    let path = base_dir.join(format!("{name}_{i:04}.obj"));
                    FrameInfo {
                        filename: path.file_name().unwrap().to_string_lossy().into_owned(),
                        path,
                        frame_number: i,
                        file_size: 1024,
                    }
                })
                .collect();
            Sequence {
                name: name.to_string(),
                frames,
                base_dir,
                source_orientation: crate::lib::coordinates::SourceOrientation::default(),
                pattern: format!("{name}_*.obj"),
            }
        };

        let mut mgr = SeriesManager::new();
        let seq_a = make_sequence("splash_v3", "/data/sims/splash_v3", 200);
        let seq_b = make_sequence("flow_v7", "/data/sims/flow_v7", 500);

        let id_a = mgr.add(seq_a, 200);
        let id_b = mgr.add_with_sections(
            seq_b,
            500,
            vec![LoadSection::new(10, 100), LoadSection::new(200, 350)],
        );

        // Make B the active series and set its playhead
        mgr.set_active(id_b);
        mgr.get_mut(id_a).unwrap().timeline.playhead = 42;
        mgr.get_mut(id_b).unwrap().timeline.playhead = 175;

        // Rename A to a custom label
        mgr.rename(id_a, "Splash (renamed)");

        // -- Step 2: Persist to Settings and write to disk --------------------
        let mut settings = Settings::default();
        settings.set_series_from_manager(&mgr);

        // Verify in-memory state before write
        assert_eq!(settings.series.len(), 2);
        assert!(!settings.series[0].active); // A is not active
        assert!(settings.series[1].active);  // B is active
        assert_eq!(settings.series[0].name, "Splash (renamed)");
        assert_eq!(settings.series[1].name, "flow_v7");
        assert_eq!(settings.series[0].playhead, 42);
        assert_eq!(settings.series[1].playhead, 175);

        // B has 2 load sections, A has 1 (the default)
        assert_eq!(settings.series[0].load_sections.len(), 1);
        assert_eq!(settings.series[1].load_sections.len(), 2);

        // Top-level backward-compat mirrors active series (B)
        let top = settings.get_load_sections();
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].start, 10);
        assert_eq!(top[1].start, 200);

        settings.save_to_dir(dir.path()).unwrap();

        // -- Step 3: Load from disk into fresh Settings -----------------------
        let loaded = Settings::load_from_dir(dir.path()).unwrap().unwrap();
        assert!(loaded.has_series());
        assert_eq!(loaded.series.len(), 2);

        // Verify loaded matches written
        assert_eq!(loaded.series[0].name, "Splash (renamed)");
        assert_eq!(loaded.series[1].name, "flow_v7");
        assert!(!loaded.series[0].active);
        assert!(loaded.series[1].active);
        assert_eq!(loaded.series[0].playhead, 42);
        assert_eq!(loaded.series[1].playhead, 175);
        assert_eq!(loaded.series[1].load_sections.len(), 2);
        assert_eq!(loaded.series[1].load_sections[0].start, 10);
        assert_eq!(loaded.series[1].load_sections[0].end, 100);
        assert_eq!(loaded.series[1].load_sections[1].start, 200);
        assert_eq!(loaded.series[1].load_sections[1].end, 350);

        // -- Step 4: Simulate fresh discovery (new SeriesManager) -------------
        // In real app, discovery finds the same sequences on disk.
        let mut fresh_mgr = SeriesManager::new();
        let fresh_a = make_sequence("splash_v3", "/data/sims/splash_v3", 200);
        let fresh_b = make_sequence("flow_v7", "/data/sims/flow_v7", 500);
        fresh_mgr.add(fresh_a, 200);
        fresh_mgr.add(fresh_b, 500);

        // Before restore: A is active (first added), default sections
        assert_eq!(
            fresh_mgr.active().unwrap().label,
            "splash_v3",
        );

        // -- Step 5: Restore from loaded settings ----------------------------
        let entries: Vec<SavedSeriesEntry> = loaded
            .get_series()
            .iter()
            .map(SavedSeriesEntry::from)
            .collect();
        let matched = fresh_mgr.restore_from_saved(&entries, 500);
        assert_eq!(matched, 2);

        // -- Step 6: Verify restored state matches original -------------------
        // Active should now be B (flow_v7)
        let active = fresh_mgr.active().unwrap();
        assert_eq!(active.label, "flow_v7");
        assert_eq!(active.timeline.playhead, 175);
        assert_eq!(active.timeline.sections.len(), 2);
        assert_eq!(active.timeline.sections[0].start, 10);
        assert_eq!(active.timeline.sections[0].end, 100);
        assert_eq!(active.timeline.sections[1].start, 200);
        assert_eq!(active.timeline.sections[1].end, 350);

        // A should have restored label and playhead
        let a = fresh_mgr
            .find_by_dir(std::path::Path::new("/data/sims/splash_v3"))
            .unwrap();
        assert_eq!(a.label, "Splash (renamed)");
        assert_eq!(a.timeline.playhead, 42);
    }

    #[test]
    fn test_multi_series_roundtrip_backward_compat_no_series() {
        // A legacy seaview.toml with no [[series]] should still work:
        // get_series() returns empty, and the top-level load_section is used.
        let dir = tempfile::tempdir().unwrap();

        let mut settings = Settings::default();
        settings.load_section = Some(LoadSectionSettings::new(10, 80));
        settings.save_to_dir(dir.path()).unwrap();

        let loaded = Settings::load_from_dir(dir.path()).unwrap().unwrap();
        assert!(!loaded.has_series());
        assert!(loaded.get_series().is_empty());

        // Top-level load section is still there
        let sections = loaded.get_load_sections();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].start, 10);
        assert_eq!(sections[0].end, 80);
    }

    #[test]
    fn test_multi_series_roundtrip_partial_match() {
        // Saved settings have 3 series, but only 2 are discovered on disk.
        // The unmatched entry should be silently skipped.
        use crate::lib::sequence::{FrameInfo, LoadSection, Sequence};
        use crate::lib::series::{SavedSeriesEntry, SeriesManager};

        let dir = tempfile::tempdir().unwrap();

        let mut settings = Settings::default();
        settings.series = vec![
            SeriesSettings {
                name: "still_here".to_string(),
                path: Some("/data/still_here".to_string()),
                active: false,
                playhead: 10,
                load_sections: vec![LoadSectionSettings::new(0, 50)],
            },
            SeriesSettings {
                name: "gone".to_string(),
                path: Some("/data/deleted_run".to_string()),
                active: true, // was active but no longer discovered
                playhead: 99,
                load_sections: vec![],
            },
            SeriesSettings {
                name: "also_here".to_string(),
                path: Some("/data/also_here".to_string()),
                active: false,
                playhead: 5,
                load_sections: vec![],
            },
        ];
        settings.save_to_dir(dir.path()).unwrap();

        let loaded = Settings::load_from_dir(dir.path()).unwrap().unwrap();
        assert_eq!(loaded.series.len(), 3);

        // Simulate discovery that only finds 2 of the 3
        let make_seq = |name: &str, base: &str, n: usize| -> Sequence {
            let base_dir = std::path::PathBuf::from(base);
            let frames: Vec<FrameInfo> = (0..n)
                .map(|i| {
                    let path = base_dir.join(format!("{name}_{i:04}.obj"));
                    FrameInfo {
                        filename: path.file_name().unwrap().to_string_lossy().into_owned(),
                        path,
                        frame_number: i,
                        file_size: 1024,
                    }
                })
                .collect();
            Sequence {
                name: name.to_string(),
                frames,
                base_dir,
                source_orientation: crate::lib::coordinates::SourceOrientation::default(),
                pattern: format!("{name}_*.obj"),
            }
        };

        let mut mgr = SeriesManager::new();
        mgr.add(make_seq("still_here", "/data/still_here", 100), 100);
        mgr.add(make_seq("also_here", "/data/also_here", 80), 80);

        let entries: Vec<SavedSeriesEntry> = loaded
            .get_series()
            .iter()
            .map(SavedSeriesEntry::from)
            .collect();
        let matched = mgr.restore_from_saved(&entries, 100);
        assert_eq!(matched, 2); // "gone" was skipped

        // "still_here" got its playhead restored
        let sh = mgr
            .find_by_dir(std::path::Path::new("/data/still_here"))
            .unwrap();
        assert_eq!(sh.timeline.playhead, 10);

        // Active should still be the first series (since the "active" entry
        // was the deleted one that didn't match)
        let active = mgr.active().unwrap();
        assert_eq!(active.label, "still_here");
    }
}
