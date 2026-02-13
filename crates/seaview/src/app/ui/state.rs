//! UI state management for Seaview
//!
//! This module contains the global UI state that persists across frames
//! and is shared between different UI systems.

use bevy::prelude::*;
use uuid::Uuid;

use crate::lib::sequence::SequenceAssets;

/// Global UI state resource
#[derive(Resource, Default)]
pub struct UiState {
    /// Currently active session ID
    pub active_session: Option<Uuid>,

    /// Whether the session panel is visible
    pub show_session_panel: bool,

    /// Whether the network panel is visible
    pub show_network_panel: bool,

    /// Whether the playback controls are visible
    pub show_playback_controls: bool,

    /// Current playback state
    pub playback: PlaybackState,

    /// Current loading state
    pub loading: LoadingState,

    /// UI panel sizes for persistence
    pub panel_sizes: PanelSizes,

    /// Temporary UI state (dialogs, etc)
    pub temp_state: TempUiState,

    /// Collapsible section state for accordion UI
    pub collapsible: CollapsibleState,
}

/// Playback control state
#[derive(Default, Debug, Clone)]
pub struct PlaybackState {
    /// Whether playback is active
    pub is_playing: bool,

    /// Current frame index
    pub current_frame: usize,

    /// Total number of frames
    pub total_frames: usize,

    /// Playback speed multiplier
    pub speed: f32,

    /// Whether to loop at the end
    pub loop_enabled: bool,
}

/// Loading state for asset loading progress
#[derive(Default, Debug, Clone)]
pub struct LoadingState {
    /// Whether loading is in progress
    pub is_loading: bool,

    /// Number of frames loaded
    pub loaded_frames: usize,

    /// Total frames to load
    pub total_frames: usize,

    /// Name of the sequence being loaded
    pub sequence_name: Option<String>,
}

impl LoadingState {
    /// Get loading progress as a percentage (0.0 to 1.0)
    pub fn progress(&self) -> f32 {
        if self.total_frames == 0 {
            0.0
        } else {
            self.loaded_frames as f32 / self.total_frames as f32
        }
    }

    /// Check if loading is complete
    pub fn is_complete(&self) -> bool {
        self.total_frames > 0 && self.loaded_frames >= self.total_frames
    }

    /// Update from SequenceAssets resource
    pub fn update_from_assets(&mut self, assets: &SequenceAssets) {
        self.is_loading = assets.loading;
        self.loaded_frames = assets.loaded_count;
        self.total_frames = assets.total_frames;
    }
}

/// Sizes of UI panels for layout persistence
#[derive(Debug, Clone)]
pub struct PanelSizes {
    /// Width of the left session panel
    pub session_panel_width: f32,

    /// Height of the bottom playback panel
    pub playback_panel_height: f32,

    /// Height of the top menu bar
    pub menu_bar_height: f32,
}

impl Default for PanelSizes {
    fn default() -> Self {
        Self {
            session_panel_width: 300.0,
            playback_panel_height: 100.0,
            menu_bar_height: 25.0,
        }
    }
}

/// Temporary UI state for dialogs and transient interactions
#[derive(Default, Debug, Clone)]
pub struct TempUiState {
    /// Whether to show the new session dialog
    pub show_new_session_dialog: bool,

    /// Whether to show the delete confirmation dialog
    pub show_delete_confirmation: Option<Uuid>,

    /// Current error message to display
    pub error_message: Option<String>,

    /// Current info message to display
    pub info_message: Option<String>,
}

/// Collapsible section state for accordion UI
#[derive(Debug, Clone)]
pub struct CollapsibleState {
    /// Whether the sessions section is open
    pub sessions_open: bool,

    /// Whether the network status section is open
    pub network_status_open: bool,

    /// Whether the lighting rig section is open
    pub lighting_rig_open: bool,

    /// Whether the material section is open
    pub material_open: bool,
}

impl Default for CollapsibleState {
    fn default() -> Self {
        Self {
            sessions_open: true,
            network_status_open: true,
            lighting_rig_open: true,
            material_open: true,
        }
    }
}

/// Resource that controls the mesh material properties
#[derive(Resource, Clone, Debug)]
pub struct MaterialConfig {
    /// Base color of the material
    pub base_color: Color,

    /// Perceptual roughness (0.0 = mirror, 1.0 = fully rough)
    pub perceptual_roughness: f32,

    /// Metallic factor (0.0 = dielectric, 1.0 = metal)
    pub metallic: f32,

    /// Reflectance at normal incidence (0.0 - 1.0)
    pub reflectance: f32,

    /// Emissive color and intensity
    pub emissive: Color,

    /// Emissive intensity multiplier
    pub emissive_intensity: f32,

    /// Whether to render double-sided (no backface culling)
    pub double_sided: bool,

    /// Alpha mode for transparency
    pub alpha_mode: AlphaModeConfig,

    /// Alpha cutoff (used with AlphaModeConfig::Mask)
    pub alpha_cutoff: f32,
}

/// Simplified alpha mode for UI selection
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AlphaModeConfig {
    Opaque,
    Mask,
    Blend,
}

impl AlphaModeConfig {
    pub fn all() -> &'static [AlphaModeConfig] {
        &[
            AlphaModeConfig::Opaque,
            AlphaModeConfig::Mask,
            AlphaModeConfig::Blend,
        ]
    }

    pub fn name(&self) -> &'static str {
        match self {
            AlphaModeConfig::Opaque => "Opaque",
            AlphaModeConfig::Mask => "Mask",
            AlphaModeConfig::Blend => "Blend",
        }
    }

    pub fn to_alpha_mode(&self, cutoff: f32) -> AlphaMode {
        match self {
            AlphaModeConfig::Opaque => AlphaMode::Opaque,
            AlphaModeConfig::Mask => AlphaMode::Mask(cutoff),
            AlphaModeConfig::Blend => AlphaMode::Blend,
        }
    }
}

impl Default for MaterialConfig {
    fn default() -> Self {
        Self {
            base_color: Color::srgb(0.3, 0.5, 0.8),
            perceptual_roughness: 0.4,
            metallic: 0.1,
            reflectance: 0.5,
            emissive: Color::BLACK,
            emissive_intensity: 0.0,
            double_sided: true,
            alpha_mode: AlphaModeConfig::Opaque,
            alpha_cutoff: 0.5,
        }
    }
}

/// Plugin for material configuration
pub struct MaterialConfigPlugin;

impl Plugin for MaterialConfigPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(MaterialConfig::default());
    }
}

impl UiState {
    /// Create a new UI state with default values
    /// Create a new UI state with sensible defaults
    pub fn new() -> Self {
        Self {
            show_session_panel: true,
            show_network_panel: true,
            show_playback_controls: true,
            playback: PlaybackState {
                speed: 1.0,
                total_frames: 0,
                ..Default::default()
            },
            loading: LoadingState::default(),
            ..Default::default()
        }
    }

    /// Set the active session
    pub fn set_active_session(&mut self, session_id: Option<Uuid>) {
        self.active_session = session_id;
        // Reset playback when switching sessions
        self.playback.current_frame = 0;
        self.playback.is_playing = false;
    }

    /// Toggle playback state
    pub fn toggle_playback(&mut self) {
        self.playback.is_playing = !self.playback.is_playing;
    }

    /// Advance to the next frame
    pub fn next_frame(&mut self) {
        if self.playback.current_frame < self.playback.total_frames.saturating_sub(1) {
            self.playback.current_frame += 1;
        } else if self.playback.loop_enabled {
            self.playback.current_frame = 0;
        } else {
            self.playback.is_playing = false;
        }
    }

    /// Go to the previous frame
    pub fn previous_frame(&mut self) {
        if self.playback.current_frame > 0 {
            self.playback.current_frame -= 1;
        } else if self.playback.loop_enabled && self.playback.total_frames > 0 {
            self.playback.current_frame = self.playback.total_frames - 1;
        }
    }

    /// Jump to a specific frame
    pub fn seek_to_frame(&mut self, frame: usize) {
        self.playback.current_frame = frame.min(self.playback.total_frames.saturating_sub(1));
    }

    /// Show an error message
    pub fn show_error(&mut self, message: impl Into<String>) {
        self.temp_state.error_message = Some(message.into());
        error!(
            "UI Error: {}",
            self.temp_state.error_message.as_ref().unwrap()
        );
    }

    /// Show an info message
    pub fn show_info(&mut self, message: impl Into<String>) {
        self.temp_state.info_message = Some(message.into());
        debug!(
            "UI Info: {}",
            self.temp_state.info_message.as_ref().unwrap()
        );
    }

    /// Clear all temporary messages
    pub fn clear_messages(&mut self) {
        self.temp_state.error_message = None;
        self.temp_state.info_message = None;
    }
}

/// Event sent when the user requests to create a new session
#[derive(Message)]
pub struct CreateSessionEvent {
    pub name: String,
    pub source_type: SessionSourceType,
}

/// Event sent when the user requests to delete a session
#[derive(Message)]
pub struct DeleteSessionEvent {
    pub session_id: Uuid,
}

/// Event sent when the user requests to switch sessions
#[derive(Message)]
pub struct SwitchSessionEvent {
    pub session_id: Uuid,
}

/// Types of session sources
#[derive(Debug, Clone, PartialEq)]
pub enum SessionSourceType {
    Network { port: u16 },
    File { path: std::path::PathBuf },
    DataLake { connection_string: String },
}

/// Plugin for UI state management
pub struct UiStatePlugin;

impl Plugin for UiStatePlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(UiState::new())
            .add_plugins(MaterialConfigPlugin)
            .add_message::<CreateSessionEvent>()
            .add_message::<DeleteSessionEvent>()
            .add_message::<SwitchSessionEvent>();
    }
}
