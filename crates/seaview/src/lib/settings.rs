//! Per-directory settings persistence for Seaview
//!
//! Settings are stored as `seaview.toml` in the sequence directory.
//! They capture camera position, coordinate system, and playback preferences.

use bevy::prelude::*;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Filename used for per-directory settings
pub const SETTINGS_FILENAME: &str = "seaview.toml";

/// Camera settings stored as human-editable values
#[derive(Debug, Clone, Serialize, Deserialize)]
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

/// Sequence/coordinate settings
#[derive(Debug, Clone, Serialize, Deserialize)]
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
pub struct PlaybackSettings {
    /// Playback speed multiplier (e.g. 1.0 = normal, 2.0 = double speed)
    pub speed: Option<f32>,
    /// Whether to loop playback
    #[serde(rename = "loop")]
    pub loop_enabled: Option<bool>,
}

impl Default for PlaybackSettings {
    fn default() -> Self {
        Self {
            speed: None,
            loop_enabled: None,
        }
    }
}

/// Cached mesh bounding-box dimensions (in metres).
/// Stored so the UI can show dimensions immediately without re-scanning vertices.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MeshBoundsSettings {
    /// Axis-aligned minimum corner [x, y, z]
    pub min: [f32; 3],
    /// Axis-aligned maximum corner [x, y, z]
    pub max: [f32; 3],
    /// Dimensions (max - min) per axis [x, y, z]
    pub dimensions: [f32; 3],
}

/// Top-level settings struct, serialized as seaview.toml
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Settings {
    /// Camera position and orientation
    pub camera: Option<CameraSettings>,
    /// Sequence/coordinate settings
    pub sequence: Option<SequenceSettings>,
    /// Playback settings
    pub playback: Option<PlaybackSettings>,
    /// Cached mesh bounding-box (computed on first load)
    pub mesh: Option<MeshBoundsSettings>,
}

impl Settings {
    /// Load settings from a seaview.toml file in the given directory.
    /// Returns Ok(None) if the file doesn't exist.
    pub fn load_from_dir(dir: &Path) -> Result<Option<Self>, SettingsError> {
        let path = dir.join(SETTINGS_FILENAME);
        if !path.exists() {
            return Ok(None);
        }
        let contents = std::fs::read_to_string(&path).map_err(|e| SettingsError::Io {
            path: path.clone(),
            source: e,
        })?;
        let settings: Settings =
            toml::from_str(&contents).map_err(|e| SettingsError::ParseToml {
                path: path.clone(),
                source: e,
            })?;
        debug!("Loaded settings from {:?}", path);
        Ok(Some(settings))
    }

    /// Save settings to seaview.toml in the given directory.
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

    /// Merge camera settings from a Transform
    pub fn set_camera_from_transform(&mut self, transform: &Transform) {
        self.camera = Some(CameraSettings::from_transform(transform));
    }

    /// Merge playback settings
    pub fn set_playback(&mut self, speed: f32, loop_enabled: bool) {
        self.playback = Some(PlaybackSettings {
            speed: Some(speed),
            loop_enabled: Some(loop_enabled),
        });
    }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_camera_roundtrip() {
        let transform =
            Transform::from_xyz(10.0, 20.0, 30.0).looking_at(Vec3::ZERO, Vec3::Y);
        let cam = CameraSettings::from_transform(&transform);
        let recovered = cam.to_transform();

        assert!((transform.translation - recovered.translation).length() < 0.001);
        // Quaternion dot product close to 1.0 means same rotation
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
        let settings = Settings {
            camera: Some(CameraSettings {
                position: [1.0, 2.0, 3.0],
                rotation: [10.0, 20.0, 0.0],
            }),
            sequence: Some(SequenceSettings {
                source_coordinates: Some("fluidx3d".to_string()),
            }),
            playback: Some(PlaybackSettings {
                speed: Some(1.5),
                loop_enabled: Some(true),
            }),
            mesh: None,
        };

        let toml_str = toml::to_string_pretty(&settings).unwrap();
        let recovered: Settings = toml::from_str(&toml_str).unwrap();

        let cam = recovered.camera.unwrap();
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
        let toml_str = r#"
[camera]
position = [5.0, 10.0, 15.0]
rotation = [0.0, 45.0, 0.0]
"#;
        let settings: Settings = toml::from_str(toml_str).unwrap();
        assert!(settings.camera.is_some());
        assert!(settings.sequence.is_none());
        assert!(settings.playback.is_none());
    }

    #[test]
    fn test_save_and_load_roundtrip() {
        let dir = std::env::temp_dir().join("seaview_test_settings");
        let _ = std::fs::create_dir_all(&dir);

        let settings = Settings {
            camera: Some(CameraSettings {
                position: [42.0, 0.0, -5.0],
                rotation: [-15.0, 90.0, 0.0],
            }),
            sequence: None,
            playback: Some(PlaybackSettings {
                speed: Some(2.0),
                loop_enabled: Some(false),
            }),
            mesh: None,
        };

        settings.save_to_dir(&dir).unwrap();
        let loaded = Settings::load_from_dir(&dir).unwrap().unwrap();

        let cam = loaded.camera.unwrap();
        assert_eq!(cam.position[0], 42.0);

        let pb = loaded.playback.unwrap();
        assert_eq!(pb.speed.unwrap(), 2.0);
        assert_eq!(pb.loop_enabled.unwrap(), false);

        // Clean up
        let _ = std::fs::remove_file(dir.join(SETTINGS_FILENAME));
        let _ = std::fs::remove_dir(&dir);
    }
}
