//! Global user-level settings for Seaview
//!
//! Global settings are stored at `~/.config/seaview/seaview.toml` (XDG compliant)
//! or `~/.seaview.toml` as a fallback.
//!
//! These provide defaults that apply across all projects. Per-directory settings
//! (in the sequence directory) override global settings when present.

use bevy::prelude::Resource;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Filename for global config
pub const GLOBAL_CONFIG_FILENAME: &str = "seaview.toml";

/// Global user-level settings
#[derive(Debug, Clone, Default, Resource, Serialize, Deserialize)]
#[serde(default)]
pub struct GlobalSettings {
    /// Memory management settings
    pub memory: Option<MemorySettings>,

    /// Default playback settings
    pub defaults: Option<DefaultSettings>,
}

/// Memory budget configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct MemorySettings {
    /// Manual memory budget in megabytes (overrides auto-detection)
    pub budget_mb: Option<u64>,

    /// Enable automatic memory detection (default: true)
    #[serde(default = "default_true")]
    pub auto_detect: bool,

    /// Fraction of available RAM to use (0.0-1.0, default: 0.5)
    #[serde(default = "default_ram_fraction")]
    pub ram_fraction: f32,

    /// Fraction of GPU VRAM to use (0.0-1.0, default: 0.5)
    #[serde(default = "default_gpu_fraction")]
    pub gpu_fraction: f32,
}

impl Default for MemorySettings {
    fn default() -> Self {
        Self {
            budget_mb: None,
            auto_detect: true,
            ram_fraction: 0.5,
            gpu_fraction: 0.5,
        }
    }
}

fn default_true() -> bool {
    true
}

fn default_ram_fraction() -> f32 {
    0.5
}

fn default_gpu_fraction() -> f32 {
    0.5
}

/// Default settings that apply to all sequences
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DefaultSettings {
    /// Default watch mode state
    pub watch: Option<bool>,

    /// Default loop mode state
    #[serde(rename = "loop")]
    pub loop_enabled: Option<bool>,

    /// Default playback speed multiplier
    pub speed: Option<f32>,

    /// Default coordinate system ("yup", "zup", "fluidx3d")
    pub source_coordinates: Option<String>,
}

impl Default for DefaultSettings {
    fn default() -> Self {
        Self {
            watch: None,
            loop_enabled: None,
            speed: None,
            source_coordinates: None,
        }
    }
}

impl GlobalSettings {
    /// Load global settings from the standard location
    pub fn load() -> Result<Self, GlobalSettingsError> {
        if let Some(path) = global_config_path() {
            Self::load_from_path(&path)
        } else {
            // No config found, return defaults
            Ok(Self::default())
        }
    }

    /// Load global settings from a specific path
    pub fn load_from_path(path: &Path) -> Result<Self, GlobalSettingsError> {
        if !path.exists() {
            return Ok(Self::default());
        }

        let contents = std::fs::read_to_string(path)
            .map_err(|e| GlobalSettingsError::IoError(path.to_path_buf(), e))?;

        let settings: Self = toml::from_str(&contents)
            .map_err(|e| GlobalSettingsError::ParseError(path.to_path_buf(), e))?;

        Ok(settings)
    }

    /// Save global settings to the standard location
    pub fn save(&self) -> Result<(), GlobalSettingsError> {
        let path = ensure_global_config_dir()?;
        self.save_to_path(&path)
    }

    /// Save global settings to a specific path
    pub fn save_to_path(&self, path: &Path) -> Result<(), GlobalSettingsError> {
        let contents = toml::to_string_pretty(self)
            .map_err(|e| GlobalSettingsError::SerializeError(path.to_path_buf(), e))?;

        std::fs::write(path, contents)
            .map_err(|e| GlobalSettingsError::IoError(path.to_path_buf(), e))?;

        Ok(())
    }
}

/// Determine the global config file path
///
/// Priority:
/// 1. `$XDG_CONFIG_HOME/seaview/seaview.toml`
/// 2. `~/.config/seaview/seaview.toml`
/// 3. `~/.seaview.toml`
pub fn global_config_path() -> Option<PathBuf> {
    // Try XDG_CONFIG_HOME first
    if let Ok(xdg_config) = std::env::var("XDG_CONFIG_HOME") {
        let path = PathBuf::from(xdg_config)
            .join("seaview")
            .join(GLOBAL_CONFIG_FILENAME);
        if path.exists() {
            return Some(path);
        }
    }

    // Try ~/.config/seaview/seaview.toml
    if let Some(home) = home_dir() {
        let path = home
            .join(".config")
            .join("seaview")
            .join(GLOBAL_CONFIG_FILENAME);
        if path.exists() {
            return Some(path);
        }
    }

    // Try ~/.seaview.toml as fallback
    if let Some(home) = home_dir() {
        let path = home.join(".seaview.toml");
        if path.exists() {
            return Some(path);
        }
    }

    // Return the preferred path even if it doesn't exist (for creation)
    if let Some(home) = home_dir() {
        return Some(home.join(".config").join("seaview").join(GLOBAL_CONFIG_FILENAME));
    }

    None
}

/// Ensure the global config directory exists and return the config file path
pub fn ensure_global_config_dir() -> Result<PathBuf, GlobalSettingsError> {
    let path = if let Ok(xdg_config) = std::env::var("XDG_CONFIG_HOME") {
        PathBuf::from(xdg_config).join("seaview")
    } else if let Some(home) = home_dir() {
        home.join(".config").join("seaview")
    } else {
        return Err(GlobalSettingsError::NoHomeDir);
    };

    std::fs::create_dir_all(&path)
        .map_err(|e| GlobalSettingsError::IoError(path.clone(), e))?;

    Ok(path.join(GLOBAL_CONFIG_FILENAME))
}

/// Get the user's home directory
fn home_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("USERPROFILE").ok().map(PathBuf::from)
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME").ok().map(PathBuf::from)
    }
}

/// Errors that can occur when loading/saving global settings
#[derive(Debug)]
pub enum GlobalSettingsError {
    /// Failed to determine home directory
    NoHomeDir,
    /// IO error (path, error)
    IoError(PathBuf, std::io::Error),
    /// TOML parse error (path, error)
    ParseError(PathBuf, toml::de::Error),
    /// TOML serialize error (path, error)
    SerializeError(PathBuf, toml::ser::Error),
}

impl std::fmt::Display for GlobalSettingsError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NoHomeDir => write!(f, "Could not determine home directory"),
            Self::IoError(path, e) => write!(f, "IO error at {:?}: {}", path, e),
            Self::ParseError(path, e) => write!(f, "Failed to parse {:?}: {}", path, e),
            Self::SerializeError(path, e) => write!(f, "Failed to serialize {:?}: {}", path, e),
        }
    }
}

impl std::error::Error for GlobalSettingsError {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_settings() {
        let settings = GlobalSettings::default();
        assert!(settings.memory.is_none());
        assert!(settings.defaults.is_none());
    }

    #[test]
    fn test_memory_settings_defaults() {
        let mem = MemorySettings::default();
        assert!(mem.auto_detect);
        assert_eq!(mem.ram_fraction, 0.5);
        assert_eq!(mem.gpu_fraction, 0.5);
        assert!(mem.budget_mb.is_none());
    }

    #[test]
    fn test_serialize_deserialize() {
        let settings = GlobalSettings {
            memory: Some(MemorySettings {
                budget_mb: Some(4096),
                auto_detect: false,
                ram_fraction: 0.6,
                gpu_fraction: 0.7,
            }),
            defaults: Some(DefaultSettings {
                watch: Some(true),
                loop_enabled: Some(false),
                speed: Some(1.5),
                source_coordinates: Some("yup".to_string()),
            }),
        };

        let toml_str = toml::to_string_pretty(&settings).unwrap();
        let recovered: GlobalSettings = toml::from_str(&toml_str).unwrap();

        let mem = recovered.memory.unwrap();
        assert_eq!(mem.budget_mb, Some(4096));
        assert!(!mem.auto_detect);
        assert_eq!(mem.ram_fraction, 0.6);

        let defaults = recovered.defaults.unwrap();
        assert_eq!(defaults.watch, Some(true));
        assert_eq!(defaults.loop_enabled, Some(false));
        assert_eq!(defaults.speed, Some(1.5));
        assert_eq!(defaults.source_coordinates.as_deref(), Some("yup"));
    }

    #[test]
    fn test_partial_toml() {
        // Test that missing fields use defaults
        let toml_str = r#"
[memory]
budget_mb = 2048

[defaults]
watch = true
"#;

        let settings: GlobalSettings = toml::from_str(toml_str).unwrap();
        let mem = settings.memory.unwrap();
        assert_eq!(mem.budget_mb, Some(2048));
        assert!(mem.auto_detect); // Should use default
        assert_eq!(mem.ram_fraction, 0.5); // Should use default

        let defaults = settings.defaults.unwrap();
        assert_eq!(defaults.watch, Some(true));
        assert!(defaults.loop_enabled.is_none());
    }
}
