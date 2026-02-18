//! UI state management for Seaview
//!
//! This module contains the global UI state that persists across frames
//! and is shared between different UI systems.

use bevy::prelude::*;
use uuid::Uuid;

use crate::lib::fade::FadeEffect;
use crate::lib::sequence::FrameStore;
use crate::lib::toast::ToastQueue;

/// Configuration for the rendering frame window (ring buffer).
///
/// Controls how many mesh entities are kept spawned around the current frame.
/// Only entities within the window have `Mesh3d` components; the rest are
/// despawned to avoid Bevy per-entity CPU overhead (`calculate_bounds`, etc.).
#[derive(Resource, Debug, Clone)]
pub struct RenderConfig {
    /// Number of frames to keep spawned around the current frame.
    /// The window spans `[current - window/2, current + window/2]`.
    /// A value of 1 means only the current frame is spawned (original behaviour).
    pub frame_window_size: usize,

    /// Whether wireframe overlay is enabled (shows mesh triangles on top of material).
    pub wireframe_enabled: bool,
}

impl Default for RenderConfig {
    fn default() -> Self {
        Self {
            frame_window_size: 5,
            wireframe_enabled: false,
        }
    }
}

/// Global UI state resource
#[derive(Resource, Default)]
pub struct UiState {
    /// Currently active session ID
    pub active_session: Option<Uuid>,

    /// Whether the environment panel is visible
    pub show_environment_panel: bool,

    /// Whether the network panel is visible
    pub show_network_panel: bool,

    /// Whether the playback controls are visible
    pub show_playback_controls: bool,

    /// Whether the performance panel is visible
    pub show_perf_panel: bool,

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

    /// Which load-section edge the user is currently dragging (if any)
    pub load_section_dragging: Option<LoadSectionEdge>,

    /// Toast notification queue
    pub toasts: ToastQueue,

    /// Monotonic clock for toast timestamps (seconds since app start).
    /// Updated each frame by a system with `Time` access.
    pub toast_clock: f64,

    /// Timestamp (elapsed seconds since app start) of the last successful save.
    /// `None` if no save has occurred yet this session.
    pub last_saved_at: Option<f64>,

    /// Fire-and-fade animation for the save indicator in the status bar.
    /// Set on each successful save; expires after its duration.
    pub save_fade: Option<FadeEffect>,
}

/// Which edge of the load section the user is dragging in the timeline UI
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadSectionEdge {
    Start,
    End,
    /// Drag the entire section body (move without resizing).
    Body,
}

/// Playback control state
#[derive(Debug, Clone)]
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

impl Default for PlaybackState {
    fn default() -> Self {
        Self {
            is_playing: false,
            current_frame: 0,
            total_frames: 0,
            speed: 1.0,
            loop_enabled: true,
        }
    }
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

    /// Update from [`FrameStore`] resource.
    pub fn update_from_frame_store(&mut self, store: &FrameStore) {
        self.is_loading = store.loading;
        self.loaded_frames = store.loaded_count;
        self.total_frames = store.total_frames;
    }
}

/// Sizes of UI panels for layout persistence
#[derive(Debug, Clone)]
pub struct PanelSizes {
    /// Width of the left environment panel
    pub environment_panel_width: f32,

    /// Height of the bottom playback panel
    pub playback_panel_height: f32,

    /// Height of the top menu bar
    pub menu_bar_height: f32,
}

impl Default for PanelSizes {
    fn default() -> Self {
        Self {
            environment_panel_width: 300.0,
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

    /// Whether the environment light section is open
    pub environment_light_open: bool,

    /// Whether the lighting rig (spot lights) section is open
    pub lighting_rig_open: bool,

    /// Whether the material section is open
    pub material_open: bool,

    /// Whether the performance section is open
    pub perf_open: bool,

    /// Whether the rendering section is open
    pub rendering_open: bool,

    /// Whether the spatial section is open
    pub spatial_open: bool,

    /// Whether the environment objects section is open
    pub environment_objects_open: bool,
}

impl Default for CollapsibleState {
    fn default() -> Self {
        Self {
            sessions_open: false,
            network_status_open: false,
            environment_light_open: false,
            lighting_rig_open: false,
            material_open: false,
            perf_open: false,
            rendering_open: false,
            spatial_open: false,
            environment_objects_open: false,
        }
    }
}

/// Resource that controls the mesh material properties
#[derive(Resource, Clone, Debug)]
pub struct MaterialConfig {
    /// Base color of the material
    pub base_color: Color,

    /// Opacity (0.0 = fully transparent, 1.0 = fully opaque)
    pub opacity: f32,

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
}

impl Default for MaterialConfig {
    fn default() -> Self {
        Self {
            base_color: Color::srgb(0.3, 0.5, 0.8),
            opacity: 1.0,
            perceptual_roughness: 0.4,
            metallic: 0.1,
            reflectance: 0.5,
            emissive: Color::BLACK,
            emissive_intensity: 0.0,
            double_sided: true,
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
            show_environment_panel: true,
            show_network_panel: false,
            show_playback_controls: true,
            show_perf_panel: false,
            playback: PlaybackState {
                speed: 1.0,
                total_frames: 0,
                ..Default::default()
            },
            loading: LoadingState::default(),
            toasts: ToastQueue::default(),
            toast_clock: 0.0,
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

    /// Show an error toast notification.
    pub fn show_error(&mut self, message: impl Into<String>) {
        let msg = message.into();
        error!("UI Error: {}", msg);
        // Push as toast — `now` will be 0.0 here since we don't have Time access,
        // but the toast_events system will set a proper timestamp on next drain.
        // Instead, use a sentinel: callers with Time access should use toasts directly.
        self.toasts.error(msg, self.toast_clock);
    }

    /// Show an info toast notification.
    pub fn show_info(&mut self, message: impl Into<String>) {
        let msg = message.into();
        debug!("UI Info: {}", msg);
        self.toasts.info(msg, self.toast_clock);
    }

    /// Show a success toast notification.
    pub fn show_success(&mut self, message: impl Into<String>) {
        let msg = message.into();
        debug!("UI Success: {}", msg);
        self.toasts.success(msg, self.toast_clock);
    }

    /// Show a warning toast notification.
    pub fn show_warning(&mut self, message: impl Into<String>) {
        let msg = message.into();
        warn!("UI Warning: {}", msg);
        self.toasts.warning(msg, self.toast_clock);
    }

    /// Clear all temporary messages (legacy — clears modal state).
    pub fn clear_messages(&mut self) {
        self.temp_state.error_message = None;
        self.temp_state.info_message = None;
    }

    /// Update the internal clock used for toast timestamps.
    /// Call once per frame from a system that has access to `Time`.
    pub fn update_toast_clock(&mut self, elapsed_secs: f64) {
        self.toast_clock = elapsed_secs;
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
            .init_resource::<RenderConfig>()
            .add_plugins(MaterialConfigPlugin)
            .add_message::<CreateSessionEvent>()
            .add_message::<DeleteSessionEvent>()
            .add_message::<SwitchSessionEvent>();
    }
}
