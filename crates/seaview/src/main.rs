use bevy::asset::io::AssetSourceBuilder;
use bevy::diagnostic::FrameTimeDiagnosticsPlugin;
use bevy::pbr::wireframe::WireframePlugin;
use bevy::pbr::{DefaultOpaqueRendererMethod, ScreenSpaceReflections};
use bevy::prelude::*;
use bevy::window::{CursorGrabMode, CursorOptions, PrimaryWindow};
use seaview::lib::lighting::{EnvironmentLightConfig, GlobalLight};
use seaview::{
    AssetLoadersPlugin, MemoryBudgetPlugin, MeshDimensions, MeshInfoPlugin, NightLightingConfig,
    NightLightingPlugin, SeaviewUiPlugin, SessionPlugin,
};

use seaview::app::cli::Args;
use seaview::app::systems::camera::{
    camera_controller, cursor_grab_system, handle_center_on_mesh, CenterOnMeshEvent, FpsCamera,
};
use seaview::app::systems::diagnostics::RenderingDiagnosticsPlugin;
use seaview::app::ui::state::MaterialConfig;

use seaview::lib::coordinates::SourceOrientation;
use seaview::lib::global_settings::GlobalSettings;
use seaview::lib::sequence::{
    discovery::DiscoverSequenceRequest, LoadSection, LoadSequenceRequest, LoadingPhase,
    LoadingPhaseState, SequencePlugin, Timeline, WatchConfig,
};
use seaview::lib::settings::{resolve_settings_dir, SaveViewEvent, Settings, SettingsResource};

/// Resource that holds saved load sections from `seaview.toml` until the
/// sequence is loaded and the Timeline is ready to accept them.
///
/// A one-shot system consumes this after `timeline.total_frames > 0`.
#[derive(Resource)]
struct PendingSavedSections(Vec<LoadSection>);

fn main() {
    // Parse command line arguments
    let args = Args::parse_args();

    if args.verbose {
        debug!("Starting Seaview mesh viewer...");
        if let Some(ref path) = args.path {
            debug!("Path provided: {:?}", path);
        }
    }

    // Load global settings first
    let global_settings = match GlobalSettings::load() {
        Ok(settings) => {
            debug!("Loaded global settings");
            settings
        }
        Err(e) => {
            warn!("Failed to load global settings: {}, using defaults", e);
            GlobalSettings::default()
        }
    };

    // Load per-directory settings if a path was provided
    let settings_dir = args.path.as_ref().and_then(|p| resolve_settings_dir(p));
    let dir_settings = settings_dir
        .as_ref()
        .and_then(|dir| match Settings::load_from_dir(dir) {
            Ok(Some(s)) => {
                debug!("Loaded seaview.toml from {:?}", dir);
                Some(s)
            }
            Ok(None) => {
                debug!("No seaview.toml found in {:?}, using defaults", dir);
                None
            }
            Err(e) => {
                warn!("Failed to load seaview.toml: {}", e);
                None
            }
        });

    // Determine source coordinates: CLI flag > per-directory settings > global settings > default
    // clap sets default_value = "yup", so we check if the user explicitly passed it
    let coord_string = {
        let cli_explicit = std::env::args().any(|a| a.starts_with("--source-coordinates"));
        if cli_explicit {
            args.source_coordinates.clone()
        } else if let Some(ref s) = dir_settings {
            s.sequence
                .as_ref()
                .and_then(|seq| seq.source_coordinates.clone())
                .or_else(|| {
                    global_settings
                        .defaults
                        .as_ref()
                        .and_then(|d| d.source_coordinates.clone())
                })
                .unwrap_or_else(|| args.source_coordinates.clone())
        } else {
            global_settings
                .defaults
                .as_ref()
                .and_then(|d| d.source_coordinates.clone())
                .unwrap_or_else(|| args.source_coordinates.clone())
        }
    };

    // Parse source coordinate system
    let source_orientation = match SourceOrientation::from_str(&coord_string) {
        Ok(orientation) => {
            if args.verbose {
                debug!("Using coordinate system: {}", orientation.description());
            }
            orientation
        }
        Err(e) => {
            error!("{}", e);
            std::process::exit(1);
        }
    };

    // Build settings resource
    let settings_resource = SettingsResource {
        settings: dir_settings.clone().unwrap_or_default(),
        directory: settings_dir,
    };

    let mut app = App::new();

    // Register asset source for the sequence directory if provided
    if let Some(ref path) = args.path {
        // Canonicalize to get absolute path
        let canonical_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());

        if canonical_path.is_dir() {
            let path_str = canonical_path.to_string_lossy().to_string();
            debug!(
                "Registering asset source 'seq' for sequence directory: {:?}",
                canonical_path
            );
            debug!("Asset source path: {}", path_str);
            app.register_asset_source("seq", AssetSourceBuilder::platform_default(&path_str, None));
            debug!("Asset source 'seq' registered successfully");
        } else if canonical_path.is_file() {
            // Register parent directory for single file loads
            if let Some(parent) = canonical_path.parent() {
                let parent_str = parent.to_string_lossy().to_string();
                debug!(
                    "Registering asset source 'seq' for file directory: {:?}",
                    parent
                );
                debug!("Asset source path: {}", parent_str);
                app.register_asset_source(
                    "seq",
                    AssetSourceBuilder::platform_default(&parent_str, None),
                );
                debug!("Asset source 'seq' registered successfully");
            } else {
                error!(
                    "Could not get parent directory for file: {:?}",
                    canonical_path
                );
            }
        }
    } else {
        warn!("No path provided - asset source 'seq' not registered");
        warn!("Provide a path via CLI to load meshes");
    }

    app.insert_resource(DefaultOpaqueRendererMethod::deferred())
        .add_plugins(DefaultPlugins)
        .add_plugins(WireframePlugin::default())
        .add_plugins(AssetLoadersPlugin)
        .add_plugins(FrameTimeDiagnosticsPlugin::default())
        .add_plugins(RenderingDiagnosticsPlugin)
        .add_plugins(SequencePlugin)
        // .add_plugins(BrpExtrasPlugin)
        .add_plugins(SessionPlugin)
        .add_plugins(MemoryBudgetPlugin)
        .add_plugins(SeaviewUiPlugin)
        .add_plugins(NightLightingPlugin)
        .add_plugins(MeshInfoPlugin)
        .add_plugins(seaview::app::systems::AppSystemsPlugin)
        .insert_resource(args)
        .insert_resource(source_orientation)
        .insert_resource(settings_resource)
        .insert_resource(global_settings)
        .add_message::<CenterOnMeshEvent>()
        .add_message::<SaveViewEvent>()
        .add_systems(
            Startup,
            (setup, handle_input_path, setup_cursor, init_watch_config),
        )
        .add_systems(Update, (restore_saved_sections,))
        .add_systems(
            Update,
            (
                camera_controller,
                cursor_grab_system,
                handle_center_on_mesh,
                handle_save_view,
                // debug_mesh_cache_status,
            ),
        )
        .run();
}

/// Initialize watch config from CLI args, per-directory settings, and global settings
fn init_watch_config(
    args: Res<Args>,
    source_orientation: Res<SourceOrientation>,
    settings_res: Res<SettingsResource>,
    global_settings: Res<GlobalSettings>,
    mut watch_config: ResMut<WatchConfig>,
    mut loading_phase: ResMut<LoadingPhase>,
) {
    // Priority: CLI flag > per-directory settings > global settings > default (false)
    let watch_from_dir_settings = settings_res
        .settings
        .playback
        .as_ref()
        .and_then(|pb| pb.watch);

    let watch_from_global = global_settings.defaults.as_ref().and_then(|d| d.watch);

    let watch_enabled = watch_from_dir_settings
        .or(watch_from_global)
        .unwrap_or(false);

    // CLI --watch flag takes precedence if specified
    watch_config.enabled = if args.watch { true } else { watch_enabled };
    watch_config.source_orientation = *source_orientation;

    // Set directory to watch
    if let Some(ref path) = args.path {
        let canonical = path.canonicalize().unwrap_or_else(|_| path.clone());
        if canonical.is_dir() {
            watch_config.directory = Some(canonical);
        } else if let Some(parent) = canonical.parent() {
            watch_config.directory = Some(parent.to_path_buf());
        }
    }

    // If watch mode enabled, start in watching state after initial load
    if watch_config.enabled {
        if args.watch {
            debug!("Watch mode enabled via --watch flag");
        } else {
            debug!("Watch mode enabled via seaview.toml");
        }
        loading_phase.state = LoadingPhaseState::InitialDiscovery;
    }
}

fn setup(
    mut commands: Commands,
    _meshes: ResMut<Assets<Mesh>>,
    _materials: ResMut<Assets<StandardMaterial>>,
    settings_res: Res<SettingsResource>,
    global_settings: Res<GlobalSettings>,
    mut sequence_manager: ResMut<seaview::sequence::SequenceManager>,
) {
    // -- Cameras: look up active_camera in the named map -----------------------
    let camera_transform = settings_res
        .settings
        .active_camera_settings()
        .map(|cam| {
            debug!(
                "Applying camera '{}' from seaview.toml: pos={:?} rot={:?}",
                settings_res.settings.active_camera_name(),
                cam.position,
                cam.rotation,
            );
            cam.to_transform()
        })
        .unwrap_or_else(|| {
            Transform::from_xyz(100.0, 100.0, 100.0)
                .looking_at(Vec3::new(37.0, 37.0, 27.5), Vec3::Y)
        });

    // Store all named cameras for runtime switching (Phase 4 UI)
    commands.insert_resource(settings_res.settings.to_saved_cameras());

    // Spawn the FPS camera with HDR, MSAA off, and SSR enabled (deferred rendering)
    commands.spawn((
        Camera3d::default(),
        camera_transform,
        FpsCamera::default(),
        Msaa::Off,
        ScreenSpaceReflections::default(),
    ));

    // Apply cached mesh bounds from seaview.toml
    if let Some(ref mesh_bounds) = settings_res.settings.mesh {
        let mut dims = MeshDimensions::from_settings(mesh_bounds);
        debug!(
            "Loaded cached mesh bounds from seaview.toml: {:.2} × {:.2} × {:.2} m",
            dims.dimensions.unwrap().x,
            dims.dimensions.unwrap().y,
            dims.dimensions.unwrap().z,
        );
        // Mark as computed so auto-compute doesn't overwrite until recompute is requested
        dims.computed = true;
        commands.insert_resource(dims);
    }

    // Apply playback settings: per-directory > global > defaults
    // Speed
    let speed = settings_res
        .settings
        .playback
        .as_ref()
        .and_then(|pb| pb.speed)
        .or_else(|| global_settings.defaults.as_ref().and_then(|d| d.speed));
    if let Some(speed) = speed {
        sequence_manager.speed = speed;
        debug!("Applied playback speed: {}", speed);
    }

    // Loop
    let loop_enabled = settings_res
        .settings
        .playback
        .as_ref()
        .and_then(|pb| pb.loop_enabled)
        .or_else(|| {
            global_settings
                .defaults
                .as_ref()
                .and_then(|d| d.loop_enabled)
        });
    if let Some(loop_enabled) = loop_enabled {
        sequence_manager.loop_enabled = loop_enabled;
        debug!("Applied loop setting: {}", loop_enabled);
    }

    // -- Load sections: stash for deferred application -------------------------
    let saved_sections = settings_res.settings.get_load_sections();
    if !saved_sections.is_empty() {
        debug!(
            "Stashing {} saved load section(s) for deferred restore",
            saved_sections.len()
        );
        commands.insert_resource(PendingSavedSections(saved_sections));
    }

    // Add a directional light for overall illumination
    commands.spawn((
        DirectionalLight {
            illuminance: 10000.0,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(0.0, 1.0, 0.0).looking_at(Vec3::new(-0.3, -1.0, -0.5), Vec3::Y),
        GlobalLight,
    ));

    // Add a point light from above
    commands.spawn((
        PointLight {
            intensity: 50000.0,
            shadows_enabled: false,
            range: 1000.0,
            ..default()
        },
        Transform::from_xyz(50.0, 150.0, 50.0),
        GlobalLight,
    ));

    // Add another point light from a different angle for better surface visibility
    commands.spawn((
        PointLight {
            intensity: 30000.0,
            shadows_enabled: false,
            range: 1000.0,
            ..default()
        },
        Transform::from_xyz(-50.0, 100.0, -50.0),
        GlobalLight,
    ));

    // Add ambient light for overall brightness
    commands.insert_resource(GlobalAmbientLight {
        color: Color::WHITE,
        brightness: 500.0,
        ..default()
    });

    // -- Environment: restore lighting + materials from seaview.toml -----------
    if let Some(ref env) = settings_res.settings.environment {
        // Environment (global) light
        if let Some(ref env_light_settings) = env.environment_light {
            let env_light_cfg = EnvironmentLightConfig::from(env_light_settings);
            debug!(
                "Restored environment light from seaview.toml: enabled={}, ambient_brightness={}",
                env_light_cfg.enabled, env_light_cfg.ambient_brightness,
            );
            commands.insert_resource(env_light_cfg);
        } else if let Some(ref lighting_settings) = env.lighting {
            // Migration: old files stored global_lighting_enabled inside [environment.lighting].
            // Transfer it to EnvironmentLightConfig so the toggle isn't lost.
            let mut env_light_cfg = EnvironmentLightConfig::default();
            env_light_cfg.enabled = lighting_settings.global_lighting_enabled;
            debug!(
                "Migrated global_lighting_enabled={} → EnvironmentLightConfig",
                env_light_cfg.enabled,
            );
            commands.insert_resource(env_light_cfg);
        }

        // Spot-light rig
        if let Some(ref lighting_settings) = env.lighting {
            let lighting_cfg = NightLightingConfig::from(lighting_settings);
            debug!(
                "Restored lighting from seaview.toml: {} lights, height={}, algo={}",
                lighting_cfg.num_lights, lighting_cfg.height, lighting_settings.placement_algorithm,
            );
            commands.insert_resource(lighting_cfg);
        }

        // Material for the fluid mesh
        if let Some(material_settings) = env.materials.get("fluid") {
            let material_cfg = MaterialConfig::from(material_settings);
            debug!(
                "Restored material 'fluid' from seaview.toml: base_color={:?}, roughness={}",
                material_settings.base_color, material_cfg.perceptual_roughness,
            );
            commands.insert_resource(material_cfg);
        }
    }
}

/// System to ensure cursor is visible on startup
fn setup_cursor(mut cursor_query: Query<&mut CursorOptions, With<PrimaryWindow>>) {
    if let Ok(mut cursor) = cursor_query.single_mut() {
        cursor.visible = true;
        cursor.grab_mode = CursorGrabMode::None;
    }
}

/// One-shot system: once the sequence is loaded (`total_frames > 0`), apply
/// the saved load sections from `seaview.toml` and remove the resource so
/// this never runs again.
fn restore_saved_sections(
    mut commands: Commands,
    pending: Option<Res<PendingSavedSections>>,
    mut timeline: ResMut<Timeline>,
    memory_budget: Res<seaview::lib::memory_budget::MemoryBudget>,
) {
    let Some(pending) = pending else { return };
    if timeline.total_frames == 0 {
        return; // sequence not loaded yet — wait
    }

    let max_frames = if memory_budget.max_frames < usize::MAX {
        memory_budget.max_frames
    } else {
        timeline.total_frames
    };

    // Replace the default single section with the saved layout
    let mut sections: Vec<LoadSection> = pending
        .0
        .iter()
        .cloned()
        .map(|mut s| {
            s.clamp(timeline.total_frames, max_frames);
            s
        })
        .filter(|s| s.frame_count() > 0)
        .collect();

    if sections.is_empty() {
        // All saved sections were out of range — keep the default
        debug!("Saved sections out of range for current sequence, keeping default");
    } else {
        sections.sort_by_key(|s| s.start);
        info!(
            "Restored {} load section(s) from seaview.toml: {:?}",
            sections.len(),
            sections
                .iter()
                .map(|s| format!("[{}..{})", s.start, s.end))
                .collect::<Vec<_>>()
        );
        timeline.sections = sections;
    }

    // Consume the resource so this system becomes a no-op
    commands.remove_resource::<PendingSavedSections>();
}

/// System that handles save view requests.
///
/// Persists camera (under the active name), playback, coordinates,
/// and environment (lighting + materials) to `seaview.toml`.
fn handle_save_view(
    mut save_events: MessageReader<SaveViewEvent>,
    mut settings_res: ResMut<SettingsResource>,
    camera_query: Query<&Transform, (With<Camera3d>, With<FpsCamera>)>,
    sequence_manager: Res<seaview::sequence::SequenceManager>,
    source_orientation: Res<SourceOrientation>,
    lighting_cfg: Res<NightLightingConfig>,
    env_light_cfg: Res<EnvironmentLightConfig>,
    material_cfg: Res<MaterialConfig>,
    watch_config: Res<WatchConfig>,
    timeline: Res<Timeline>,
) {
    for _event in save_events.read() {
        if let Ok(transform) = camera_query.single() {
            // Camera — save under the active name (defaults to "default")
            settings_res.settings.set_camera_from_transform(transform);

            // Playback + coordinates + watch
            settings_res.settings.set_playback(
                sequence_manager.speed,
                sequence_manager.loop_enabled,
                watch_config.enabled,
            );
            settings_res.settings.sequence = Some(seaview::lib::settings::SequenceSettings {
                source_coordinates: Some(source_orientation.to_string()),
            });

            // Environment — lighting + material
            settings_res
                .settings
                .set_environment(&lighting_cfg, &env_light_cfg, &material_cfg);

            // Load sections — persist multi-section layout
            settings_res.settings.set_load_sections(&timeline.sections);

            match settings_res.save() {
                Ok(()) => {
                    debug!(
                        "💾 View saved to seaview.toml (camera='{}', environment included)",
                        settings_res.settings.active_camera_name()
                    );
                }
                Err(e) => {
                    error!("Failed to save view: {}", e);
                }
            }
        } else {
            warn!("No camera found to save");
        }
    }
}

/// System that handles the input path and decides whether to load a single file or discover a sequence
fn handle_input_path(
    mut commands: Commands,
    args: Res<Args>,
    source_orientation: Res<SourceOrientation>,
    mut load_events: MessageWriter<LoadSequenceRequest>,
    mut sequence_manager: ResMut<seaview::sequence::SequenceManager>,
) {
    if let Some(path) = &args.path {
        if path.is_dir() {
            // It's a directory - trigger sequence discovery
            debug!("Discovering sequences in directory: {:?}", path);
            commands.spawn(DiscoverSequenceRequest {
                directory: path.clone(),
                recursive: false,
                source_orientation: *source_orientation,
            });
        } else if path.is_file() {
            // It's a single file - load it directly using the sequence loader
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_lowercase())
                .unwrap_or_default();

            match ext.as_str() {
                "glb" | "gltf" | "stl" => {
                    debug!("Loading single mesh file: {:?}", path);

                    // Create a minimal sequence so coordinate transform is applied
                    let filename = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("mesh")
                        .to_string();
                    let base_dir = path.parent().unwrap_or(path.as_path()).to_path_buf();

                    let mut sequence = seaview::sequence::Sequence::new(
                        filename.clone(),
                        base_dir,
                        "single_file".to_string(),
                        *source_orientation,
                    );
                    sequence.add_frame(seaview::sequence::FrameInfo::new(path.clone(), 0));

                    debug!(
                        "Using coordinate system: {}",
                        source_orientation.description()
                    );
                    sequence_manager.load_sequence(sequence);

                    // Load as a single-frame "sequence"
                    // Use the seq:// asset source that was registered at startup
                    load_events.write(LoadSequenceRequest {
                        frame_paths: vec![path.clone()],
                    });
                }
                _ => {
                    warn!("Unsupported file type '{}': {:?}", ext, path);
                }
            }
        } else {
            error!("Path does not exist: {:?}", path);
        }
    }
}
