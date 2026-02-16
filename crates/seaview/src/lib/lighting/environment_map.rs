//! Environment map (IBL + skybox) configuration and sync system.
//!
//! [`EnvironmentMapConfig`] is a Bevy `Resource` that tracks whether the
//! environment map is enabled and which map is selected.
//!
//! [`EnvironmentMapCatalog`] holds the list of discovered map entries.
//! Each entry follows the naming convention:
//!   `environment_maps/{name}_diffuse_rgb9e5_zstd.ktx2`
//!   `environment_maps/{name}_specular_rgb9e5_zstd.ktx2`
//!
//! The [`sync_environment_map`] system adds/removes `EnvironmentMapLight` and
//! `Skybox` components on the camera based on the current config.

use bevy::core_pipeline::Skybox;
use bevy::prelude::EnvironmentMapLight;
use bevy::prelude::*;

// ───────────────────────────────────────────────────────────────────────────
// Catalog entry
// ───────────────────────────────────────────────────────────────────────────

/// A single environment map available for selection.
#[derive(Debug, Clone)]
pub struct EnvironmentMapEntry {
    /// Human-readable display name (e.g. "Pisa").
    pub name: String,
    /// Asset path for the diffuse (irradiance) cubemap.
    pub diffuse_path: String,
    /// Asset path for the specular (pre-filtered) cubemap.
    pub specular_path: String,
}

// ───────────────────────────────────────────────────────────────────────────
// Catalog resource
// ───────────────────────────────────────────────────────────────────────────

/// Registry of all available environment maps.
///
/// Populated at startup. Maps follow the naming convention
/// `environment_maps/{stem}_diffuse_rgb9e5_zstd.ktx2` /
/// `environment_maps/{stem}_specular_rgb9e5_zstd.ktx2`.
#[derive(Resource, Debug, Clone)]
pub struct EnvironmentMapCatalog {
    pub entries: Vec<EnvironmentMapEntry>,
}

impl Default for EnvironmentMapCatalog {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
        }
    }
}

impl EnvironmentMapCatalog {
    /// Register a map from a `(name, stem)` pair.
    ///
    /// The stem is used to build the conventional asset paths.
    pub fn register(&mut self, name: impl Into<String>, stem: impl AsRef<str>) {
        let stem = stem.as_ref();
        self.entries.push(EnvironmentMapEntry {
            name: name.into(),
            diffuse_path: format!("environment_maps/{}_diffuse_rgb9e5_zstd.ktx2", stem),
            specular_path: format!("environment_maps/{}_specular_rgb9e5_zstd.ktx2", stem),
        });
    }

    /// Find an entry by name (case-insensitive).
    pub fn find(&self, name: &str) -> Option<&EnvironmentMapEntry> {
        self.entries
            .iter()
            .find(|e| e.name.eq_ignore_ascii_case(name))
    }

    /// Find the index of an entry by name (case-insensitive).
    pub fn index_of(&self, name: &str) -> Option<usize> {
        self.entries
            .iter()
            .position(|e| e.name.eq_ignore_ascii_case(name))
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Config resource
// ───────────────────────────────────────────────────────────────────────────

/// Runtime configuration for the environment map (IBL + skybox).
///
/// Disabled by default. When `enabled` is true the sync system attaches
/// `EnvironmentMapLight` and `Skybox` to the camera; when false it removes
/// them.
#[derive(Resource, Debug, Clone)]
pub struct EnvironmentMapConfig {
    /// Master toggle.
    pub enabled: bool,
    /// Index into [`EnvironmentMapCatalog::entries`], or `None` for "none selected".
    pub selected: Option<usize>,
    /// IBL intensity multiplier (drives `EnvironmentMapLight::intensity`).
    pub intensity: f32,
    /// Skybox brightness (drives `Skybox::brightness`).
    pub skybox_brightness: f32,
}

impl Default for EnvironmentMapConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            selected: None,
            intensity: 900.0,
            skybox_brightness: 1000.0,
        }
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Marker — tracks whether we currently have env map components on the camera
// ───────────────────────────────────────────────────────────────────────────

/// Marker component added alongside `EnvironmentMapLight` + `Skybox` so the
/// sync system knows whether to insert or remove them.
#[derive(Component)]
pub struct EnvironmentMapActive;

// ───────────────────────────────────────────────────────────────────────────
// Sync system
// ───────────────────────────────────────────────────────────────────────────

/// System that syncs `EnvironmentMapLight` and `Skybox` on the camera entity
/// to match the current [`EnvironmentMapConfig`].
///
/// Runs every frame (cheap early-out when nothing changed).
pub fn sync_environment_map(
    mut commands: Commands,
    config: Res<EnvironmentMapConfig>,
    catalog: Res<EnvironmentMapCatalog>,
    asset_server: Res<AssetServer>,
    camera_q: Query<(Entity, Has<EnvironmentMapActive>), With<Camera3d>>,
) {
    let Ok((camera_entity, has_marker)) = camera_q.single() else {
        return;
    };

    let should_be_active = config.enabled && config.selected.is_some();

    if should_be_active {
        let Some(idx) = config.selected else { return };
        let Some(entry) = catalog.entries.get(idx) else {
            return;
        };

        // Always update when config changes (handles intensity/brightness tweaks
        // and map switches). Bevy deduplicates identical asset loads internally.
        if config.is_changed() {
            let specular_handle: Handle<Image> = asset_server.load(&entry.specular_path);
            let diffuse_handle: Handle<Image> = asset_server.load(&entry.diffuse_path);

            commands.entity(camera_entity).insert((
                EnvironmentMapLight {
                    diffuse_map: diffuse_handle,
                    specular_map: specular_handle.clone(),
                    intensity: config.intensity,
                    ..default()
                },
                Skybox {
                    image: specular_handle,
                    brightness: config.skybox_brightness,
                    ..default()
                },
                EnvironmentMapActive,
            ));
        }
    } else if has_marker {
        // Remove env map components
        commands
            .entity(camera_entity)
            .remove::<EnvironmentMapLight>()
            .remove::<Skybox>()
            .remove::<EnvironmentMapActive>();
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Tests
// ───────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_register_builds_paths() {
        let mut catalog = EnvironmentMapCatalog::default();
        catalog.register("Pisa", "pisa");
        assert_eq!(catalog.entries.len(), 1);
        assert_eq!(
            catalog.entries[0].diffuse_path,
            "environment_maps/pisa_diffuse_rgb9e5_zstd.ktx2"
        );
        assert_eq!(
            catalog.entries[0].specular_path,
            "environment_maps/pisa_specular_rgb9e5_zstd.ktx2"
        );
    }

    #[test]
    fn catalog_find_case_insensitive() {
        let mut catalog = EnvironmentMapCatalog::default();
        catalog.register("Pisa", "pisa");
        assert!(catalog.find("pisa").is_some());
        assert!(catalog.find("PISA").is_some());
        assert!(catalog.find("missing").is_none());
    }

    #[test]
    fn catalog_index_of() {
        let mut catalog = EnvironmentMapCatalog::default();
        catalog.register("Pisa", "pisa");
        catalog.register("Venice", "venice");
        assert_eq!(catalog.index_of("Pisa"), Some(0));
        assert_eq!(catalog.index_of("venice"), Some(1));
        assert_eq!(catalog.index_of("nope"), None);
    }

    #[test]
    fn default_config_is_disabled() {
        let cfg = EnvironmentMapConfig::default();
        assert!(!cfg.enabled);
        assert!(cfg.selected.is_none());
    }

    #[test]
    fn default_catalog_is_empty() {
        let catalog = EnvironmentMapCatalog::default();
        assert!(catalog.entries.is_empty());
    }
}
