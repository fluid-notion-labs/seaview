//! Environment & material presets.
//!
//! [`EnvironmentPreset`] — lighting combos (ambient + directional).
//! [`MaterialPreset`] — surface appearance (color, roughness, metallic, etc.).
//!
//! These are independent: pick any lighting with any material.
//!
//! This is pure data — no ECS, no systems, no queries.
//!
//! # Usage
//!
//! ```ignore
//! let env_light = EnvironmentPreset::Dusk.config();
//! let material  = MaterialPreset::Water.config();
//! ```

use bevy::prelude::*;

use crate::app::ui::state::MaterialConfig;
use crate::lib::lighting::EnvironmentLightConfig;

// ───────────────────────────────────────────────────────────────────────────
// Preset enum
// ───────────────────────────────────────────────────────────────────────────

/// A named environment preset that bundles lighting settings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EnvironmentPreset {
    /// Bright sunny day on Earth (~6500 K sun, blue sky ambient).
    Earth,
    /// Lunar surface — harsh unfiltered sun, black-sky ambient.
    Moon,
    /// Mars — dusty salmon sky, reduced illuminance, warm tones.
    Mars,
    /// Earth at night lit by a full moon — cool blue, very dim.
    Moonlight,
    /// Golden hour / civil twilight — warm low sun, purple-blue sky.
    Dusk,
    /// Deep red-orange sunset — dramatic, low illuminance.
    RedSunset,
}

impl EnvironmentPreset {
    /// All available presets in display order.
    pub const ALL: &'static [EnvironmentPreset] = &[
        EnvironmentPreset::Earth,
        EnvironmentPreset::Moon,
        EnvironmentPreset::Mars,
        EnvironmentPreset::Moonlight,
        EnvironmentPreset::Dusk,
        EnvironmentPreset::RedSunset,
    ];

    /// Human-readable label for UI display.
    pub fn label(self) -> &'static str {
        match self {
            Self::Earth => "Earth (Daylight)",
            Self::Moon => "Moon",
            Self::Mars => "Mars",
            Self::Moonlight => "Moonlight",
            Self::Dusk => "Dusk",
            Self::RedSunset => "Red Sunset",
        }
    }

    /// Short description shown as a tooltip.
    pub fn description(self) -> &'static str {
        match self {
            Self::Earth => "Bright sunny day, blue-sky ambient",
            Self::Moon => "Harsh unfiltered sun, no atmosphere",
            Self::Mars => "Dusty salmon sky, reduced illuminance, warm tones",
            Self::Moonlight => "Full-moon night, cool blue tones, dim",
            Self::Dusk => "Golden hour, warm low sun, purple-blue sky fill",
            Self::RedSunset => "Deep red-orange sunset, dramatic low light",
        }
    }

    /// Produce the [`EnvironmentLightConfig`] for this preset.
    pub fn config(self) -> EnvironmentLightConfig {
        match self {
            Self::Earth => earth(),
            Self::Moon => moon(),
            Self::Mars => mars(),
            Self::Moonlight => moonlight(),
            Self::Dusk => dusk(),
            Self::RedSunset => red_sunset(),
        }
    }
}

impl std::fmt::Display for EnvironmentPreset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Helper
// ───────────────────────────────────────────────────────────────────────────

fn env_light(
    ambient_color: [f32; 3],
    ambient_brightness: f32,
    directional_color: [f32; 3],
    directional_illuminance: f32,
) -> EnvironmentLightConfig {
    EnvironmentLightConfig {
        enabled: true,
        ambient_color: Color::srgb(ambient_color[0], ambient_color[1], ambient_color[2]),
        ambient_brightness,
        directional_color: Color::srgb(
            directional_color[0],
            directional_color[1],
            directional_color[2],
        ),
        directional_illuminance,
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Preset definitions
// ───────────────────────────────────────────────────────────────────────────

/// Earth — bright sunny day.
///
/// Sun: ~6500 K warm white, 100 000 lux.
/// Ambient: sky-blue fill.
fn earth() -> EnvironmentLightConfig {
    env_light(
        [0.53, 0.81, 0.92], // sky blue
        150.0,
        [1.0, 0.95, 0.85], // warm white sun
        100_000.0,
    )
}

/// Moon — lunar surface, no atmosphere.
///
/// Sun: ~5800 K unfiltered, 130 000 lux (no atmospheric scattering).
/// Ambient: almost nothing — space is black.
fn moon() -> EnvironmentLightConfig {
    env_light(
        [0.08, 0.08, 0.10], // near-black, faint blue bounce from regolith
        15.0,
        [1.0, 0.98, 0.94], // unfiltered sunlight
        130_000.0,
    )
}

/// Mars — thin CO₂ atmosphere, dusty salmon sky.
///
/// Sun: ~40 % Earth illuminance, filtered through dust → warmer.
/// Ambient: salmon-butterscotch sky dome.
fn mars() -> EnvironmentLightConfig {
    env_light(
        [0.80, 0.55, 0.35], // salmon/butterscotch sky
        80.0,
        [1.0, 0.82, 0.65], // dust-filtered sun
        40_000.0,
    )
}

/// Moonlight — Earth at night under a full moon.
///
/// Very dim (artistically boosted for visibility).
/// Cool blue-white moonlight, deep-blue ambient.
fn moonlight() -> EnvironmentLightConfig {
    env_light(
        [0.12, 0.15, 0.28], // deep blue night sky
        25.0,
        [0.70, 0.78, 0.95], // cool blue-white moonlight
        2_000.0,             // artistic — real is ~0.25 lux
    )
}

/// Dusk — golden hour / civil twilight.
///
/// Low sun, warm golden direct light, purple-blue sky fill.
fn dusk() -> EnvironmentLightConfig {
    env_light(
        [0.45, 0.35, 0.55], // purple-blue twilight sky
        100.0,
        [1.0, 0.72, 0.38], // deep golden sun
        20_000.0,
    )
}

/// Red Sunset — deep red-orange, dramatic.
///
/// Sun near horizon, heavy atmospheric scattering, warm dark ambient.
fn red_sunset() -> EnvironmentLightConfig {
    env_light(
        [0.35, 0.15, 0.10], // dark warm ambient
        60.0,
        [1.0, 0.32, 0.08], // deep red-orange
        8_000.0,
    )
}

// ═══════════════════════════════════════════════════════════════════════════
// Material presets
// ═══════════════════════════════════════════════════════════════════════════

/// A named material preset for the fluid surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MaterialPreset {
    /// Realistic water — deep blue-teal, smooth, translucent, strong Fresnel.
    Water,
    /// Deep ocean — darker blue, near-opaque, mirror-smooth.
    Ocean,
    /// Ice — pale blue-white, glassy, slightly translucent.
    Ice,
    /// Clear glass — colourless, fully transparent, very high reflectance.
    Glass,
    /// Clay — matte, opaque, neutral grey. Good for seeing shape.
    Clay,
    /// Lava — dark crust with orange emissive glow.
    Lava,
    /// Chrome — mirror-polished metal, high metallic.
    Chrome,
    /// Plastic — moderate roughness, coloured, no metallic (the "default" look).
    Plastic,
}

impl MaterialPreset {
    /// All available presets in display order.
    pub const ALL: &'static [MaterialPreset] = &[
        MaterialPreset::Water,
        MaterialPreset::Ocean,
        MaterialPreset::Ice,
        MaterialPreset::Glass,
        MaterialPreset::Clay,
        MaterialPreset::Lava,
        MaterialPreset::Chrome,
        MaterialPreset::Plastic,
    ];

    /// Human-readable label for UI display.
    pub fn label(self) -> &'static str {
        match self {
            Self::Water => "Water",
            Self::Ocean => "Ocean",
            Self::Ice => "Ice",
            Self::Glass => "Glass",
            Self::Clay => "Clay",
            Self::Lava => "Lava",
            Self::Chrome => "Chrome",
            Self::Plastic => "Plastic",
        }
    }

    /// Short description shown as a tooltip.
    pub fn description(self) -> &'static str {
        match self {
            Self::Water => "Deep blue-teal, smooth, translucent, strong Fresnel",
            Self::Ocean => "Dark blue, near-opaque, mirror-smooth",
            Self::Ice => "Pale blue-white, glassy, slightly translucent",
            Self::Glass => "Colourless, fully transparent, high reflectance",
            Self::Clay => "Matte, opaque, neutral grey — good for shape",
            Self::Lava => "Dark crust with orange emissive glow",
            Self::Chrome => "Mirror-polished metal, high metallic",
            Self::Plastic => "Moderate roughness, coloured, dielectric",
        }
    }

    /// Produce the [`MaterialConfig`] for this preset.
    pub fn config(self) -> MaterialConfig {
        match self {
            Self::Water => mat_water(),
            Self::Ocean => mat_ocean(),
            Self::Ice => mat_ice(),
            Self::Glass => mat_glass(),
            Self::Clay => mat_clay(),
            Self::Lava => mat_lava(),
            Self::Chrome => mat_chrome(),
            Self::Plastic => mat_plastic(),
        }
    }
}

impl std::fmt::Display for MaterialPreset {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.label())
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Material helper
// ───────────────────────────────────────────────────────────────────────────

fn mat(
    base_color: [f32; 3],
    roughness: f32,
    metallic: f32,
    reflectance: f32,
    opacity: f32,
    emissive: [f32; 3],
    emissive_intensity: f32,
    double_sided: bool,
) -> MaterialConfig {
    MaterialConfig {
        base_color: Color::srgb(base_color[0], base_color[1], base_color[2]),
        opacity,
        perceptual_roughness: roughness,
        metallic,
        reflectance,
        emissive: Color::srgb(emissive[0], emissive[1], emissive[2]),
        emissive_intensity,
        double_sided,
    }
}

// ───────────────────────────────────────────────────────────────────────────
// Material preset definitions
// ───────────────────────────────────────────────────────────────────────────

/// Realistic water — per water_material.md reference.
fn mat_water() -> MaterialConfig {
    mat(
        [0.102, 0.431, 0.557], // #1A6E8E deep blue-teal
        0.03,                  // near-mirror
        0.0,                   // dielectric
        0.6,                   // strong Fresnel
        0.92,                  // slightly translucent
        [0.0, 0.0, 0.0],
        0.0,
        false,
    )
}

/// Deep ocean — darker, denser.
fn mat_ocean() -> MaterialConfig {
    mat(
        [0.04, 0.12, 0.22], // very dark navy
        0.02,               // mirror-smooth
        0.0,
        0.65,               // strong Fresnel
        0.96,               // near-opaque
        [0.0, 0.0, 0.0],
        0.0,
        false,
    )
}

/// Ice — pale blue-white, glassy.
fn mat_ice() -> MaterialConfig {
    mat(
        [0.75, 0.85, 0.92], // pale blue-white
        0.08,               // slightly rough (micro-fractures)
        0.0,
        0.5,
        0.88, // translucent
        [0.0, 0.0, 0.0],
        0.0,
        false,
    )
}

/// Clear glass — colourless, transparent.
fn mat_glass() -> MaterialConfig {
    mat(
        [0.95, 0.95, 0.95], // near-white
        0.01,               // almost perfect mirror
        0.0,
        0.7, // very high Fresnel
        0.3, // very transparent
        [0.0, 0.0, 0.0],
        0.0,
        true, // see-through → double-sided
    )
}

/// Clay — matte, opaque, neutral.
fn mat_clay() -> MaterialConfig {
    mat(
        [0.55, 0.52, 0.48], // warm grey
        0.85,               // very rough / matte
        0.0,
        0.3, // low reflectance
        1.0, // fully opaque
        [0.0, 0.0, 0.0],
        0.0,
        true,
    )
}

/// Lava — dark crust, orange emissive.
fn mat_lava() -> MaterialConfig {
    mat(
        [0.08, 0.04, 0.02], // near-black crust
        0.7,                // rough volcanic surface
        0.0,
        0.3,
        1.0,
        [1.0, 0.35, 0.02], // orange glow
        8.0,                // strong emissive
        true,
    )
}

/// Chrome — mirror-polished metal.
fn mat_chrome() -> MaterialConfig {
    mat(
        [0.8, 0.8, 0.82], // bright silver
        0.02,              // mirror-smooth
        1.0,               // fully metallic
        0.5,
        1.0,
        [0.0, 0.0, 0.0],
        0.0,
        false,
    )
}

/// Plastic — the classic default look.
fn mat_plastic() -> MaterialConfig {
    mat(
        [0.3, 0.5, 0.8], // blue (matches old default)
        0.4,              // moderate roughness
        0.0,
        0.5,
        1.0,
        [0.0, 0.0, 0.0],
        0.0,
        true,
    )
}

// ───────────────────────────────────────────────────────────────────────────
// Tests
// ───────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_presets_produce_enabled_light() {
        for &preset in EnvironmentPreset::ALL {
            let light = preset.config();
            assert!(
                light.enabled,
                "{}: light should be enabled",
                preset.label(),
            );
            assert!(
                light.directional_illuminance > 0.0,
                "{}: illuminance must be positive",
                preset.label(),
            );
            assert!(
                light.ambient_brightness > 0.0,
                "{}: ambient brightness must be positive",
                preset.label(),
            );
        }
    }

    #[test]
    fn env_preset_count_matches_all_slice() {
        assert_eq!(EnvironmentPreset::ALL.len(), 6);
    }

    #[test]
    fn env_labels_are_unique() {
        let labels: Vec<&str> = EnvironmentPreset::ALL.iter().map(|p| p.label()).collect();
        for (i, a) in labels.iter().enumerate() {
            for (j, b) in labels.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "duplicate env label");
                }
            }
        }
    }

    #[test]
    fn env_display_matches_label() {
        for &preset in EnvironmentPreset::ALL {
            assert_eq!(format!("{}", preset), preset.label());
        }
    }

    // -- Material preset tests ---------------------------------------------

    #[test]
    fn all_material_presets_produce_valid_config() {
        for &preset in MaterialPreset::ALL {
            let cfg = preset.config();
            assert!(
                cfg.opacity > 0.0 && cfg.opacity <= 1.0,
                "{}: opacity out of range: {}",
                preset.label(),
                cfg.opacity,
            );
            assert!(
                cfg.perceptual_roughness >= 0.0 && cfg.perceptual_roughness <= 1.0,
                "{}: roughness out of range",
                preset.label(),
            );
            assert!(
                cfg.metallic >= 0.0 && cfg.metallic <= 1.0,
                "{}: metallic out of range",
                preset.label(),
            );
            assert!(
                cfg.reflectance >= 0.0 && cfg.reflectance <= 1.0,
                "{}: reflectance out of range",
                preset.label(),
            );
        }
    }

    #[test]
    fn mat_preset_count_matches_all_slice() {
        assert_eq!(MaterialPreset::ALL.len(), 8);
    }

    #[test]
    fn mat_labels_are_unique() {
        let labels: Vec<&str> = MaterialPreset::ALL.iter().map(|p| p.label()).collect();
        for (i, a) in labels.iter().enumerate() {
            for (j, b) in labels.iter().enumerate() {
                if i != j {
                    assert_ne!(a, b, "duplicate mat label");
                }
            }
        }
    }

    #[test]
    fn mat_display_matches_label() {
        for &preset in MaterialPreset::ALL {
            assert_eq!(format!("{}", preset), preset.label());
        }
    }

    #[test]
    fn water_presets_are_dielectric() {
        for &preset in &[MaterialPreset::Water, MaterialPreset::Ocean, MaterialPreset::Ice] {
            let cfg = preset.config();
            assert!(
                cfg.metallic < 0.01,
                "{}: water/ice must be dielectric, got {}",
                preset.label(),
                cfg.metallic,
            );
        }
    }

    #[test]
    fn chrome_is_metallic() {
        let cfg = MaterialPreset::Chrome.config();
        assert!(cfg.metallic > 0.9, "chrome should be metallic");
    }

    #[test]
    fn lava_has_emissive() {
        let cfg = MaterialPreset::Lava.config();
        assert!(cfg.emissive_intensity > 0.0, "lava should glow");
    }
}
