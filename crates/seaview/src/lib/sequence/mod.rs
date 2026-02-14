//! Sequence management for mesh files
//!
//! This module provides functionality for discovering, loading, and playing back
//! sequences of mesh files (e.g., simulation timesteps).

pub mod discovery;
pub mod loader;
pub mod playback;
pub mod watch;

use bevy::prelude::*;
use std::path::PathBuf;

use crate::app::ui::state::UiState;

// Re-export commonly used types from loader
pub use loader::{FrameLoadedEvent, LoadSequenceRequest, LoadingStats, SequenceAssets};
pub use watch::{LoadingPhase, LoadingPhaseState, WatchConfig, WatchPlugin};

/// Plugin for mesh sequence management
pub struct SequencePlugin;

impl Plugin for SequencePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            discovery::SequenceDiscoveryPlugin,
            playback::SequencePlaybackPlugin,
            loader::SequenceLoaderPlugin,
            watch::WatchPlugin,
        ))
        .init_resource::<SequenceManager>()
        .add_message::<SequenceEvent>()
        .add_systems(
            Update,
            (advance_playback, playback::handle_keyboard_playback)
                .before(sync_ui_from_sequence_manager),
        )
        .add_systems(Update, sync_ui_from_sequence_manager);
    }
}

/// Resource that manages the current sequence state.
///
/// **Single source of truth** for playback. All systems that need to change
/// frame, speed, or play/pause write here. `UiState::PlaybackState` is a
/// read-only mirror populated by `sync_ui_from_sequence_manager`.
#[derive(Resource)]
pub struct SequenceManager {
    /// Currently loaded sequence, if any
    pub current_sequence: Option<Sequence>,
    /// Current frame index in the sequence
    pub current_frame: usize,
    /// Whether playback is active
    pub is_playing: bool,
    /// Playback speed multiplier (1.0 = normal, 2.0 = double speed)
    pub speed: f32,
    /// Whether to loop at the end of the sequence
    pub loop_enabled: bool,
}

impl SequenceManager {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            current_sequence: None,
            current_frame: 0,
            is_playing: false,
            speed: 1.0,
            loop_enabled: true,
        }
    }
}

impl Default for SequenceManager {
    fn default() -> Self {
        Self {
            current_sequence: None,
            current_frame: 0,
            is_playing: false,
            speed: 1.0,
            loop_enabled: true,
        }
    }
}

impl SequenceManager {
    /// Load a new sequence
    pub fn load_sequence(&mut self, sequence: Sequence) {
        self.current_sequence = Some(sequence);
        self.current_frame = 0;
        self.is_playing = false;
    }

    /// Get the current sequence
    pub fn current_sequence(&self) -> Option<&Sequence> {
        self.current_sequence.as_ref()
    }

    /// Get the current frame path
    pub fn current_frame_path(&self) -> Option<&PathBuf> {
        self.current_sequence
            .as_ref()?
            .frame_path(self.current_frame)
    }

    /// Move to the next frame
    pub fn next_frame(&mut self) -> bool {
        if let Some(sequence) = &self.current_sequence {
            if self.current_frame + 1 < sequence.frame_count() {
                self.current_frame += 1;
                return true;
            }
        }
        false
    }

    /// Move to the previous frame
    pub fn previous_frame(&mut self) -> bool {
        if self.current_frame > 0 {
            self.current_frame -= 1;
            return true;
        }
        false
    }

    /// Jump to a specific frame
    pub fn jump_to_frame(&mut self, frame: usize) -> bool {
        if let Some(sequence) = &self.current_sequence {
            if frame < sequence.frame_count() {
                self.current_frame = frame;
                return true;
            }
        }
        false
    }

    /// Toggle playback
    pub fn toggle_playback(&mut self) {
        self.is_playing = !self.is_playing;
    }

    /// Set playback speed (clamped to 0.1..=5.0)
    pub fn set_speed(&mut self, speed: f32) {
        self.speed = speed.clamp(0.1, 5.0);
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// Timer-based auto-advance. Reads `sequence_manager.speed` and
/// `sequence_manager.loop_enabled`. Writes `sequence_manager.current_frame`.
fn advance_playback(
    mut sm: ResMut<SequenceManager>,
    time: Res<Time>,
    mut accumulator: Local<f32>,
) {
    if !sm.is_playing || sm.current_sequence.is_none() {
        *accumulator = 0.0;
        return;
    }

    let frame_duration = 1.0 / (30.0 * sm.speed);
    *accumulator += time.delta_secs();

    while *accumulator >= frame_duration {
        *accumulator -= frame_duration;

        let total = sm.current_sequence().map(|s| s.frame_count()).unwrap_or(0);
        if sm.current_frame + 1 < total {
            sm.current_frame += 1;
        } else if sm.loop_enabled {
            sm.current_frame = 0;
        } else {
            sm.is_playing = false;
            *accumulator = 0.0;
            break;
        }
    }
}

/// One-way sync: `SequenceManager` → `UiState::PlaybackState` for display.
///
/// Also syncs loading state from `SequenceAssets` so the UI panel has
/// everything it needs without borrowing resources directly.
fn sync_ui_from_sequence_manager(
    sm: Res<SequenceManager>,
    mut ui_state: ResMut<UiState>,
    sequence_assets: Res<SequenceAssets>,
) {
    if let Some(seq) = sm.current_sequence() {
        ui_state.playback.total_frames = seq.frame_count();
    }
    ui_state.playback.current_frame = sm.current_frame;
    ui_state.playback.is_playing = sm.is_playing;
    ui_state.playback.speed = sm.speed;
    ui_state.playback.loop_enabled = sm.loop_enabled;

    // Sync loading state
    ui_state.loading.update_from_assets(&sequence_assets);
}

/// Represents a sequence of mesh files
#[derive(Debug, Clone)]
pub struct Sequence {
    /// Name of the sequence
    pub name: String,
    /// Base directory containing the sequence
    pub base_dir: PathBuf,
    /// List of frame files in order
    pub frames: Vec<FrameInfo>,
    /// Source coordinate system orientation
    pub source_orientation: crate::lib::coordinates::SourceOrientation,
    /// Pattern that matches the sequence files
    pub pattern: String,
}

impl Sequence {
    pub fn new(
        name: String,
        base_dir: PathBuf,
        pattern: String,
        source_orientation: crate::lib::coordinates::SourceOrientation,
    ) -> Self {
        Self {
            name,
            base_dir,
            frames: Vec::new(),
            source_orientation,
            pattern,
        }
    }

    /// Get the number of frames in the sequence
    pub fn frame_count(&self) -> usize {
        self.frames.len()
    }

    /// Get the path for a specific frame
    pub fn frame_path(&self, index: usize) -> Option<&PathBuf> {
        self.frames.get(index).map(|f| &f.path)
    }

    /// Get frame info for a specific frame
    #[allow(dead_code)]
    pub fn frame_info(&self, index: usize) -> Option<&FrameInfo> {
        self.frames.get(index)
    }

    /// Add a frame to the sequence
    pub fn add_frame(&mut self, frame: FrameInfo) {
        self.frames.push(frame);
    }

    /// Sort frames by their frame number
    #[allow(dead_code)]
    pub fn sort_frames(&mut self) {
        self.frames.sort_by_key(|frame| frame.frame_number);
    }
}

/// Information about a single frame in a sequence
#[derive(Debug, Clone)]
pub struct FrameInfo {
    /// Path to the frame file
    pub path: PathBuf,
    /// Frame number extracted from filename
    #[allow(dead_code)]
    pub frame_number: usize,
    /// Original filename
    #[allow(dead_code)]
    pub filename: String,
    /// File size in bytes
    #[allow(dead_code)]
    pub file_size: u64,
}

impl FrameInfo {
    pub fn new(path: PathBuf, frame_number: usize) -> Self {
        let filename = path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        let file_size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);

        Self {
            path,
            frame_number,
            filename,
            file_size,
        }
    }
}

/// Events related to sequence operations
#[derive(Message, Debug)]
pub enum SequenceEvent {
    /// A new sequence was loaded
    #[allow(dead_code)]
    SequenceLoaded(String),
    /// Frame changed
    #[allow(dead_code)]
    FrameChanged(usize),
    /// Playback started
    PlaybackStarted,
    /// Playback stopped
    PlaybackStopped,
    /// Sequence discovery started
    #[allow(dead_code)]
    DiscoveryStarted(PathBuf),
    /// Sequence discovery completed
    #[allow(dead_code)]
    DiscoveryCompleted(usize),
    /// Error occurred
    #[allow(dead_code)]
    Error(String),
}
