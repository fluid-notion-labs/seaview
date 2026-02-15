//! Per-directory settings persistence for Seaview
//!
//! Settings are stored as `seaview.toml` in the sequence directory.
//! They capture camera position, coordinate system, playback preferences,
//! named cameras, and environment settings (lighting, materials).

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::app::ui::state::MaterialConfig;
use crate::lib::lighting::{NightLightingConfig, PlacementAlgorithm};
use crate::lib::sequence::LoadSection;

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
}

impl Default for LoadSectionSettings {
    fn default() -> Self {
        Self {
            start: 0,
            end: 100, // Default to first 100 frames
        }
    }
}

impl LoadSectionSettings {
    /// Create a new load section
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
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
        LoadSection::new(s.start, s.end)
    }
}

impl From<&LoadSectionSettings> for LoadSection {
    fn from(s: &LoadSectionSettings) -> Self {
        LoadSection::new(s.start, s.end)
    }
}

impl From<LoadSection> for LoadSectionSettings {
    fn from(s: LoadSection) -> Self {
        LoadSectionSettings::new(s.start, s.end)
    }
}

impl From<&LoadSection> for LoadSectionSettings {
    fn from(s: &LoadSection) -> Self {
        LoadSectionSettings::new(s.start, s.end)
    }
}

// ---------------------------------------------------------------------------
// Environment — Lighting bridge
// ---------------------------------------------------------------------------

/// TOML-friendly bridge for [`NightLightingConfig`].
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
    pub global_lighting_enabled: bool,
    pub intensity: f32,
    /// sRGB colour as `[r, g, b]` in 0–1 range.
    pub color: [f32; 3],
    pub range: f32,
    pub show_markers: bool,
    pub marker_size: f32,
    pub coverage_pct: f32,
}

impl Default for LightingSettings {
    fn default() -> Self {
        let cfg = NightLightingConfig::default();
        Self::from(&cfg)
    }
}

impl From<&NightLightingConfig> for LightingSettings {
    fn from(cfg: &NightLightingConfig) -> Self {
        Self {
            num_lights: cfg.num_lights,
            height: cfg.height,
            cone_angle: cfg.cone_angle,
            placement_algorithm: placement_to_snake(&cfg.placement_algorithm),
            enabled: cfg.enabled,
            global_lighting_enabled: cfg.global_lighting_enabled,
            intensity: cfg.intensity,
            color: color_to_srgb3(&cfg.color),
            range: cfg.range,
            show_markers: cfg.show_markers,
            marker_size: cfg.marker_size,
            coverage_pct: cfg.coverage_pct,
        }
    }
}

impl From<&LightingSettings> for NightLightingConfig {
    fn from(s: &LightingSettings) -> Self {
        // Use Kilolumens as the default display unit (matches Default impl)
        let unit = crate::lib::lighting::LightIntensityUnit::Kilolumens;
        let display_value = unit.from_lumens(s.intensity, s.cone_angle);

        Self {
            num_lights: s.num_lights,
            height: s.height,
            cone_angle: s.cone_angle,
            placement_algorithm: snake_to_placement(&s.placement_algorithm),
            enabled: s.enabled,
            global_lighting_enabled: s.global_lighting_enabled,
            intensity: s.intensity,
            intensity_display_value: display_value,
            intensity_unit: unit,
            color: Color::srgb(s.color[0], s.color[1], s.color[2]),
            range: s.range,
            show_markers: s.show_markers,
            marker_size: s.marker_size,
            coverage_pct: s.coverage_pct,
        }
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
// EnvironmentSettings — grouping struct
// ---------------------------------------------------------------------------

/// Groups all visual-appearance settings.
///
/// Future sections (props, background, post-processing) are reserved as
/// `Option` fields so they can be omitted from TOML when absent.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct EnvironmentSettings {
    /// Spot-light rig configuration.
    pub lighting: Option<LightingSettings>,
    /// Named materials map. `"fluid"` is the default key for the sequence mesh.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub materials: HashMap<String, MaterialSettings>,
    // --- future sections (reserved, not implemented) ---
    // pub props:           Option<Vec<PropSettings>>,
    // pub background:      Option<BackgroundSettings>,
    // pub post_processing: Option<PostProcessingSettings>,
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
    pub fn set_environment(
        &mut self,
        lighting_cfg: &NightLightingConfig,
        material_cfg: &MaterialConfig,
    ) {
        let env = self
            .environment
            .get_or_insert_with(EnvironmentSettings::default);
        env.lighting = Some(LightingSettings::from(lighting_cfg));
        env.materials
            .insert("fluid".to_string(), MaterialSettings::from(material_cfg));
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
// PlacementAlgorithm ↔ snake_case string helpers
// ---------------------------------------------------------------------------

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
        // Fall back to the default
        _ => PlacementAlgorithm::UniformGrid,
    }
}

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
    fn test_lighting_settings_roundtrip() {
        let cfg = NightLightingConfig::default();
        let settings = LightingSettings::from(&cfg);
        let recovered = NightLightingConfig::from(&settings);

        assert_eq!(cfg.num_lights, recovered.num_lights);
        assert_eq!(cfg.height, recovered.height);
        assert_eq!(cfg.cone_angle, recovered.cone_angle);
        assert_eq!(cfg.placement_algorithm, recovered.placement_algorithm);
        assert_eq!(cfg.enabled, recovered.enabled);
        assert_eq!(
            cfg.global_lighting_enabled,
            recovered.global_lighting_enabled
        );
        assert_eq!(cfg.intensity, recovered.intensity);
        assert_eq!(cfg.range, recovered.range);
        assert_eq!(cfg.show_markers, recovered.show_markers);
        assert_eq!(cfg.marker_size, recovered.marker_size);
        assert_eq!(cfg.coverage_pct, recovered.coverage_pct);
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
        let lighting = NightLightingConfig::default();
        let material = MaterialConfig::default();

        let mut settings = Settings::default();
        settings.set_environment(&lighting, &material);

        let env = settings.environment.as_ref().unwrap();
        assert!(env.lighting.is_some());
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
                lighting: Some(LightingSettings::default()),
                materials: {
                    let mut m = HashMap::new();
                    m.insert("fluid".to_string(), MaterialSettings::default());
                    m
                },
            }),
            load_section: None,
            load_sections: vec![],
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
}
