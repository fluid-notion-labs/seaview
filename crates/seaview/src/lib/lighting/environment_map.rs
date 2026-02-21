//! Environment map (IBL + skybox) configuration and sync system.
//!
//! [`EnvironmentMapConfig`] is a Bevy `Resource` that tracks whether the
//! environment map is enabled and which map is selected.
//!
//! [`EnvironmentMapCatalog`] holds the list of discovered map entries.
//! Entries come in two flavours:
//!
//! 1. **KTX2 pairs** — pre-baked diffuse + specular cubemaps following the
//!    naming convention `environment_maps/{stem}_diffuse_rgb9e5_zstd.ktx2` /
//!    `environment_maps/{stem}_specular_rgb9e5_zstd.ktx2`.  These provide
//!    full IBL (image-based lighting) with proper pre-filtered specular.
//!
//! 2. **HDR / EXR files** — equirectangular panoramas placed in the
//!    `environment_maps/` asset directory (e.g. `environment_maps/sunset.hdr`
//!    or `environment_maps/studio.exr`).  These are loaded, converted to
//!    cubemaps on the CPU, and used for both skybox and approximate IBL.
//!    The specular pre-filtering is *not* done — both diffuse and specular
//!    channels receive the same cubemap — so reflections will be sharper
//!    than physically correct.  This is a good-enough approximation for
//!    visualisation work and avoids a GPU compute pass.
//!
//! [`sync_environment_map`] adds/removes `EnvironmentMapLight` and `Skybox`
//! components on the camera based on the current config.
//!
//! [`process_pending_hdr_cubemaps`] watches for loaded HDR/EXR assets and
//! converts them from equirectangular 2D textures into cubemaps.

use bevy::core_pipeline::Skybox;
use bevy::prelude::EnvironmentMapLight;
use bevy::prelude::*;
use bevy::render::render_resource::{TextureViewDescriptor, TextureViewDimension};
use std::f32::consts::PI;

// ───────────────────────────────────────────────────────────────────────────
// Catalog entry
// ───────────────────────────────────────────────────────────────────────────

/// The source format of an environment map entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvironmentMapSource {
    /// Pre-baked KTX2 cubemap pair (diffuse + specular).
    Ktx2 {
        diffuse_path: String,
        specular_path: String,
    },
    /// Equirectangular HDR or EXR panorama (single file).
    /// Will be converted to a cubemap at runtime.
    Hdr {
        /// Asset path for the panorama (e.g. `environment_maps/sunset.hdr`).
        asset_path: String,
    },
}

/// A single environment map available for selection.
#[derive(Debug, Clone)]
pub struct EnvironmentMapEntry {
    /// Human-readable display name (e.g. "Pisa", "sunset").
    pub name: String,
    /// Where the map data comes from.
    pub source: EnvironmentMapSource,
}

// ───────────────────────────────────────────────────────────────────────────
// Catalog resource
// ───────────────────────────────────────────────────────────────────────────

/// Registry of all available environment maps.
///
/// Populated at startup.  KTX2 maps follow the naming convention
/// `environment_maps/{stem}_diffuse_rgb9e5_zstd.ktx2` /
/// `environment_maps/{stem}_specular_rgb9e5_zstd.ktx2`.
/// HDR/EXR maps are single files in `environment_maps/`.
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
    /// Register a KTX2 cubemap pair from a `(name, stem)` pair.
    ///
    /// The stem is used to build the conventional asset paths.
    pub fn register(&mut self, name: impl Into<String>, stem: impl AsRef<str>) {
        let stem = stem.as_ref();
        self.entries.push(EnvironmentMapEntry {
            name: name.into(),
            source: EnvironmentMapSource::Ktx2 {
                diffuse_path: format!("environment_maps/{}_diffuse_rgb9e5_zstd.ktx2", stem),
                specular_path: format!("environment_maps/{}_specular_rgb9e5_zstd.ktx2", stem),
            },
        });
    }

    /// Register an HDR or EXR equirectangular panorama.
    pub fn register_hdr(&mut self, name: impl Into<String>, asset_path: impl Into<String>) {
        self.entries.push(EnvironmentMapEntry {
            name: name.into(),
            source: EnvironmentMapSource::Hdr {
                asset_path: asset_path.into(),
            },
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
// Discovery — scan assets dir for .hdr / .exr files
// ───────────────────────────────────────────────────────────────────────────

/// Scan the `environment_maps/` asset directory for `.hdr` and `.exr` files
/// and register them in the catalog.
///
/// This is a pure function — it takes the asset base path, reads the
/// filesystem, and returns `(display_name, asset_path)` pairs.
pub fn discover_hdr_files(asset_dir: &std::path::Path) -> Vec<(String, String)> {
    let env_dir = asset_dir.join("environment_maps");
    let mut results = Vec::new();

    let entries = match std::fs::read_dir(&env_dir) {
        Ok(e) => e,
        Err(_) => return results,
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());

        match ext.as_deref() {
            Some("hdr") | Some("exr") => {}
            _ => continue,
        }

        let stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };

        // Build a human-readable name: replace underscores/hyphens with
        // spaces, title-case the first letter of each word.
        let display_name = stem
            .replace(['_', '-'], " ")
            .split_whitespace()
            .map(|word| {
                let mut chars = word.chars();
                match chars.next() {
                    Some(c) => {
                        let upper: String = c.to_uppercase().collect();
                        format!("{}{}", upper, chars.as_str())
                    }
                    None => String::new(),
                }
            })
            .collect::<Vec<_>>()
            .join(" ");

        let filename = match path.file_name().and_then(|f| f.to_str()) {
            Some(f) => f.to_string(),
            None => continue,
        };

        let asset_path = format!("environment_maps/{}", filename);
        results.push((display_name, asset_path));
    }

    results.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    results
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
// Pending HDR cubemap conversion
// ───────────────────────────────────────────────────────────────────────────

/// Tracks an HDR/EXR image that has been loaded but not yet converted to a
/// cubemap.  The [`process_pending_hdr_cubemaps`] system picks these up.
#[derive(Resource, Default)]
pub struct PendingHdrCubemaps {
    /// `(catalog_index, image_handle)` pairs awaiting conversion.
    pub pending: Vec<(usize, Handle<Image>)>,
    /// Catalog indices that have already been converted (avoid re-processing).
    pub converted: Vec<usize>,
}

// ───────────────────────────────────────────────────────────────────────────
// Equirectangular → cubemap conversion (CPU)
// ───────────────────────────────────────────────────────────────────────────

/// Sample an equirectangular panorama at a given 3D direction.
///
/// Returns `(u, v)` in `[0, 1]` for the source image.
fn direction_to_equirect_uv(dir: [f32; 3]) -> (f32, f32) {
    // Normalise (should already be normalised but be safe)
    let len = (dir[0] * dir[0] + dir[1] * dir[1] + dir[2] * dir[2]).sqrt();
    let (x, y, z) = (dir[0] / len, dir[1] / len, dir[2] / len);

    // Spherical coordinates
    // longitude: atan2(z, x), latitude: asin(y)
    let u = 0.5 + z.atan2(x) / (2.0 * PI);
    let v = 0.5 - y.asin() / PI;
    (u.clamp(0.0, 1.0), v.clamp(0.0, 1.0))
}

/// The six cubemap face directions.
/// Order: +X, -X, +Y, -Y, +Z, -Z
/// Each face is defined by (right, up, forward) vectors used to generate
/// per-pixel ray directions.
///
/// Note: Cubemap coordinates are left-handed Y-up (GPU convention).
/// +Z in cubemap space points into the screen (left-handed), which is -Z
/// in Bevy's right-handed world space.
const CUBEMAP_FACES: [([f32; 3], [f32; 3], [f32; 3]); 6] = [
    // +X: right face
    ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0], [1.0, 0.0, 0.0]),
    // -X: left face
    ([0.0, 0.0, 1.0], [0.0, 1.0, 0.0], [-1.0, 0.0, 0.0]),
    // +Y: top face
    ([1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
    // -Y: bottom face
    ([1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.0, -1.0, 0.0]),
    // +Z: front face (left-handed)
    ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
    // -Z: back face (left-handed)
    ([-1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, -1.0]),
];

/// Convert an equirectangular panorama (as raw `Rgba32Float` pixel data)
/// into 6 cubemap faces, returned as a single contiguous `Vec<f32>` with
/// faces stacked vertically (each face is `face_size × face_size`).
///
/// `src_width` / `src_height` are the dimensions of the source panorama.
/// `src_data` is the RGBA f32 pixel data (4 floats per pixel).
pub fn equirect_to_cubemap_faces(
    src_data: &[f32],
    src_width: u32,
    src_height: u32,
    face_size: u32,
) -> Vec<f32> {
    let pixels_per_face = (face_size * face_size) as usize;
    let total_pixels = pixels_per_face * 6;
    let mut out = vec![0.0f32; total_pixels * 4];

    for (face_idx, (right, up, forward)) in CUBEMAP_FACES.iter().enumerate() {
        let face_offset = face_idx * pixels_per_face * 4;

        for y in 0..face_size {
            for x in 0..face_size {
                // Map pixel (x, y) to [-1, 1] range
                let fx = (2.0 * (x as f32 + 0.5) / face_size as f32) - 1.0;
                let fy = 1.0 - (2.0 * (y as f32 + 0.5) / face_size as f32);

                // Build direction vector: forward + fx * right + fy * up
                let dir = [
                    forward[0] + fx * right[0] + fy * up[0],
                    forward[1] + fx * right[1] + fy * up[1],
                    forward[2] + fx * right[2] + fy * up[2],
                ];

                let (u, v) = direction_to_equirect_uv(dir);

                // Bilinear sample from source
                let sx = u * (src_width as f32 - 1.0);
                let sy = v * (src_height as f32 - 1.0);

                let x0 = (sx.floor() as u32).min(src_width - 1);
                let y0 = (sy.floor() as u32).min(src_height - 1);
                let x1 = (x0 + 1).min(src_width - 1);
                let y1 = (y0 + 1).min(src_height - 1);

                let fx_frac = sx - sx.floor();
                let fy_frac = sy - sy.floor();

                let sample = |px: u32, py: u32| -> [f32; 4] {
                    let idx = ((py * src_width + px) * 4) as usize;
                    if idx + 3 < src_data.len() {
                        [src_data[idx], src_data[idx + 1], src_data[idx + 2], src_data[idx + 3]]
                    } else {
                        [0.0, 0.0, 0.0, 1.0]
                    }
                };

                let s00 = sample(x0, y0);
                let s10 = sample(x1, y0);
                let s01 = sample(x0, y1);
                let s11 = sample(x1, y1);

                let pixel_idx = face_offset + ((y * face_size + x) as usize) * 4;
                for c in 0..4 {
                    let top = s00[c] * (1.0 - fx_frac) + s10[c] * fx_frac;
                    let bot = s01[c] * (1.0 - fx_frac) + s11[c] * fx_frac;
                    out[pixel_idx + c] = top * (1.0 - fy_frac) + bot * fy_frac;
                }
            }
        }
    }

    out
}

/// Determine an appropriate cubemap face size from the source panorama
/// dimensions.  A 2:1 equirectangular panorama of width W maps well to
/// face_size ≈ W/4 (each face covers ~90°).  We clamp to powers of two
/// between 256 and 2048.
pub fn choose_face_size(src_width: u32) -> u32 {
    let ideal = src_width / 4;
    // Round to nearest power of two
    let pot = (ideal as f32).log2().round() as u32;
    (1u32 << pot).clamp(256, 2048)
}

// ───────────────────────────────────────────────────────────────────────────
// System: process loaded HDR/EXR into cubemaps
// ───────────────────────────────────────────────────────────────────────────

/// Watches for loaded HDR/EXR images and converts them to cubemap textures
/// in-place so they can be used with `Skybox` and `EnvironmentMapLight`.
pub fn process_pending_hdr_cubemaps(
    mut pending: ResMut<PendingHdrCubemaps>,
    asset_server: Res<AssetServer>,
    mut images: ResMut<Assets<Image>>,
) {
    if pending.pending.is_empty() {
        return;
    }

    debug!(
        "process_pending_hdr_cubemaps: {} pending, {} already converted",
        pending.pending.len(),
        pending.converted.len(),
    );

    // Collect work items first (immutable borrow of pending.pending) so we
    // can mutate pending.converted afterwards without a borrow conflict.
    let work: Vec<(usize, usize, Handle<Image>)> = pending
        .pending
        .iter()
        .enumerate()
        .filter_map(|(i, (catalog_idx, handle))| {
            if pending.converted.contains(catalog_idx) {
                return Some((i, *catalog_idx, handle.clone())); // will be removed as completed
            }
            if asset_server.load_state(handle).is_loaded() {
                debug!("  pending[{}] catalog_idx={} — asset loaded, ready to convert", i, catalog_idx);
                Some((i, *catalog_idx, handle.clone()))
            } else {
                debug!("  pending[{}] catalog_idx={} — still loading", i, catalog_idx);
                None
            }
        })
        .collect();

    if work.is_empty() {
        return;
    }

    let mut completed_indices = Vec::new();

    for (i, catalog_idx, handle) in &work {
        // Already converted — just mark for removal from pending list.
        if pending.converted.contains(catalog_idx) {
            debug!("  catalog_idx={} already converted, removing from pending", catalog_idx);
            completed_indices.push(*i);
            continue;
        }

        let Some(image) = images.get_mut(handle) else {
            debug!("  catalog_idx={} handle exists but image not in Assets — skipping", catalog_idx);
            continue;
        };

        // Only process 2D images (equirectangular panoramas).
        // If it already has 6 layers it's already a cubemap.
        if image.texture_descriptor.size.depth_or_array_layers > 1 {
            info!(
                "Environment map image already has {} layers, skipping conversion",
                image.texture_descriptor.size.depth_or_array_layers,
            );
            pending.converted.push(*catalog_idx);
            completed_indices.push(*i);
            continue;
        }

        let src_width = image.width();
        let src_height = image.height();

        info!(
            "Converting equirectangular HDR {}×{} to cubemap...",
            src_width, src_height
        );

        // We need Rgba32Float data for the conversion.  The HDR/EXR loaders
        // produce this format.
        let format = image.texture_descriptor.format;
        debug!(
            "  catalog_idx={} source format={:?}, dimensions={}×{}, data_len={}",
            catalog_idx,
            format,
            src_width,
            src_height,
            image.data.as_ref().map(|d| d.len()).unwrap_or(0),
        );
        let src_floats = match format {
            bevy::render::render_resource::TextureFormat::Rgba32Float => {
                // Data is already f32s — reinterpret the byte slice.
                debug!("  Rgba32Float — zero-copy reinterpret to &[f32]");
                let bytes = image.data.as_ref().expect("image has no data");
                bytemuck::cast_slice::<u8, f32>(bytes).to_vec()
            }
            bevy::render::render_resource::TextureFormat::Rgba16Float => {
                // Convert f16 → f32
                debug!("  Rgba16Float — converting f16 → f32");
                let bytes = image.data.as_ref().expect("image has no data");
                let halfs = bytemuck::cast_slice::<u8, u16>(bytes);
                halfs.iter().map(|&h| half_to_f32(h)).collect()
            }
            _ => {
                warn!(
                    "Environment map has unsupported texture format {:?}, skipping cubemap conversion",
                    format,
                );
                pending.converted.push(*catalog_idx);
                completed_indices.push(*i);
                continue;
            }
        };

        let face_size = choose_face_size(src_width);
        debug!(
            "  catalog_idx={} chose face_size={} (from src_width={}), converting {} source pixels → 6×{}×{} cubemap",
            catalog_idx, face_size, src_width, src_floats.len() / 4, face_size, face_size,
        );
        let cubemap_data = equirect_to_cubemap_faces(&src_floats, src_width, src_height, face_size);

        // Convert f32 data back to bytes
        let cubemap_bytes: Vec<u8> = bytemuck::cast_slice(&cubemap_data).to_vec();
        debug!(
            "  catalog_idx={} cubemap produced {} bytes ({} floats, {} pixels)",
            catalog_idx,
            cubemap_bytes.len(),
            cubemap_data.len(),
            cubemap_data.len() / 4,
        );

        // Overwrite the image in-place: 6 faces as array layers
        image.texture_descriptor.size.width = face_size;
        image.texture_descriptor.size.height = face_size;
        image.texture_descriptor.size.depth_or_array_layers = 6;
        image.texture_descriptor.format =
            bevy::render::render_resource::TextureFormat::Rgba32Float;
        image.data = Some(cubemap_bytes);

        // Tell Bevy to interpret this as a Cube texture
        image.texture_view_descriptor = Some(TextureViewDescriptor {
            dimension: Some(TextureViewDimension::Cube),
            ..default()
        });

        info!(
            "Environment map cubemap conversion complete: {}×{} per face, 6 faces",
            face_size, face_size
        );

        pending.converted.push(*catalog_idx);
        completed_indices.push(*i);
    }

    // Remove completed entries (iterate in reverse to preserve indices)
    completed_indices.sort_unstable();
    for i in completed_indices.into_iter().rev() {
        pending.pending.remove(i);
    }
}

/// Convert an IEEE 754 half-precision float (u16) to f32.
fn half_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) & 1) as u32;
    let exp = ((h >> 10) & 0x1f) as u32;
    let mant = (h & 0x3ff) as u32;

    if exp == 0 {
        if mant == 0 {
            // Zero
            f32::from_bits(sign << 31)
        } else {
            // Subnormal
            let mut m = mant;
            let mut e = 1u32;
            while (m & 0x400) == 0 {
                m <<= 1;
                e += 1;
            }
            let m = (m & 0x3ff) << 13;
            let e = (127u32 - 15 + 1).wrapping_sub(e);
            f32::from_bits((sign << 31) | (e << 23) | m)
        }
    } else if exp == 31 {
        // Inf / NaN
        let m = mant << 13;
        f32::from_bits((sign << 31) | (0xff << 23) | m)
    } else {
        // Normalised
        let e = exp + (127 - 15);
        let m = mant << 13;
        f32::from_bits((sign << 31) | (e << 23) | m)
    }
}

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
    mut pending: ResMut<PendingHdrCubemaps>,
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
            debug!(
                "sync_environment_map: config changed — entry='{}' idx={} intensity={} skybox_brightness={}",
                entry.name, idx, config.intensity, config.skybox_brightness,
            );
            match &entry.source {
                EnvironmentMapSource::Ktx2 {
                    diffuse_path,
                    specular_path,
                } => {
                    debug!("  KTX2 pair: diffuse='{}', specular='{}'", diffuse_path, specular_path);
                    let specular_handle: Handle<Image> = asset_server.load(specular_path);
                    let diffuse_handle: Handle<Image> = asset_server.load(diffuse_path);

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
                EnvironmentMapSource::Hdr { asset_path } => {
                    debug!("  HDR/EXR panorama: '{}'", asset_path);
                    let handle: Handle<Image> = asset_server.load(asset_path);

                    // Register for cubemap conversion if not already done
                    if !pending.converted.contains(&idx)
                        && !pending.pending.iter().any(|(ci, _)| *ci == idx)
                    {
                        debug!("  Queued for cubemap conversion (catalog_idx={})", idx);
                        pending.pending.push((idx, handle.clone()));
                    } else {
                        debug!(
                            "  Already queued or converted (converted={}, pending={})",
                            pending.converted.contains(&idx),
                            pending.pending.iter().any(|(ci, _)| *ci == idx),
                        );
                    }

                    // Use the same handle for both diffuse and specular.
                    // The image will be converted to a cubemap by
                    // process_pending_hdr_cubemaps once it finishes loading.
                    commands.entity(camera_entity).insert((
                        EnvironmentMapLight {
                            diffuse_map: handle.clone(),
                            specular_map: handle.clone(),
                            intensity: config.intensity,
                            ..default()
                        },
                        Skybox {
                            image: handle,
                            brightness: config.skybox_brightness,
                            ..default()
                        },
                        EnvironmentMapActive,
                    ));
                }
            }
        }
    } else if has_marker {
        debug!("sync_environment_map: removing env map components (enabled={}, selected={:?})", config.enabled, config.selected);
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
        match &catalog.entries[0].source {
            EnvironmentMapSource::Ktx2 {
                diffuse_path,
                specular_path,
            } => {
                assert_eq!(
                    diffuse_path,
                    "environment_maps/pisa_diffuse_rgb9e5_zstd.ktx2"
                );
                assert_eq!(
                    specular_path,
                    "environment_maps/pisa_specular_rgb9e5_zstd.ktx2"
                );
            }
            _ => panic!("Expected Ktx2 source"),
        }
    }

    #[test]
    fn catalog_register_hdr() {
        let mut catalog = EnvironmentMapCatalog::default();
        catalog.register_hdr("Sunset", "environment_maps/sunset.hdr");
        assert_eq!(catalog.entries.len(), 1);
        assert_eq!(catalog.entries[0].name, "Sunset");
        match &catalog.entries[0].source {
            EnvironmentMapSource::Hdr { asset_path } => {
                assert_eq!(asset_path, "environment_maps/sunset.hdr");
            }
            _ => panic!("Expected Hdr source"),
        }
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

    #[test]
    fn direction_to_equirect_uv_forward() {
        // Looking straight along +X should give u ≈ 0.5
        let (u, v) = direction_to_equirect_uv([1.0, 0.0, 0.0]);
        assert!((u - 0.5).abs() < 0.01, "u={}", u);
        assert!((v - 0.5).abs() < 0.01, "v={}", v);
    }

    #[test]
    fn direction_to_equirect_uv_up() {
        // Looking straight up (+Y) should give v ≈ 0
        let (_, v) = direction_to_equirect_uv([0.0, 1.0, 0.0]);
        assert!(v < 0.01, "v={}", v);
    }

    #[test]
    fn direction_to_equirect_uv_down() {
        // Looking straight down (-Y) should give v ≈ 1
        let (_, v) = direction_to_equirect_uv([0.0, -1.0, 0.0]);
        assert!(v > 0.99, "v={}", v);
    }

    #[test]
    fn choose_face_size_ranges() {
        // 4096 wide panorama → ideal 1024 → 1024
        assert_eq!(choose_face_size(4096), 1024);
        // 2048 wide → ideal 512 → 512
        assert_eq!(choose_face_size(2048), 512);
        // 8192 wide → ideal 2048 → 2048
        assert_eq!(choose_face_size(8192), 2048);
        // Very small → clamps to 256
        assert_eq!(choose_face_size(128), 256);
        // Very large → clamps to 2048
        assert_eq!(choose_face_size(32768), 2048);
    }

    #[test]
    fn equirect_to_cubemap_produces_correct_size() {
        // Tiny 4×2 panorama, face_size 2 → 6 faces × 2×2 × 4 channels
        let src = vec![1.0f32; 4 * 2 * 4]; // 4×2 RGBA
        let result = equirect_to_cubemap_faces(&src, 4, 2, 2);
        assert_eq!(result.len(), 6 * 2 * 2 * 4);
    }

    #[test]
    fn half_to_f32_zero() {
        assert_eq!(half_to_f32(0x0000), 0.0f32);
        assert_eq!(half_to_f32(0x8000), -0.0f32);
    }

    #[test]
    fn half_to_f32_one() {
        // Half-precision 1.0 = 0x3C00
        let val = half_to_f32(0x3C00);
        assert!((val - 1.0).abs() < 1e-6, "val={}", val);
    }

    #[test]
    fn half_to_f32_inf() {
        let val = half_to_f32(0x7C00);
        assert!(val.is_infinite() && val.is_sign_positive());
    }

    #[test]
    fn half_to_f32_nan() {
        let val = half_to_f32(0x7C01);
        assert!(val.is_nan());
    }

    #[test]
    fn mixed_catalog_entries() {
        let mut catalog = EnvironmentMapCatalog::default();
        catalog.register("Pisa", "pisa");
        catalog.register_hdr("Sunset", "environment_maps/sunset.hdr");
        catalog.register_hdr("Studio", "environment_maps/studio.exr");
        assert_eq!(catalog.entries.len(), 3);

        assert!(matches!(
            &catalog.entries[0].source,
            EnvironmentMapSource::Ktx2 { .. }
        ));
        assert!(matches!(
            &catalog.entries[1].source,
            EnvironmentMapSource::Hdr { .. }
        ));
        assert!(matches!(
            &catalog.entries[2].source,
            EnvironmentMapSource::Hdr { .. }
        ));

        assert_eq!(catalog.index_of("Sunset"), Some(1));
        assert_eq!(catalog.index_of("Studio"), Some(2));
    }
}
