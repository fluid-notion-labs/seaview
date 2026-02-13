//! Sequence management for mesh files
//!
//! This module provides functionality for discovering, loading, and playing back
//! sequences of mesh files (e.g., simulation timesteps).

pub mod discovery;
pub mod loader;
pub mod playback;

use bevy::prelude::*;
use std::path::PathBuf;

// Re-export commonly used types from loader
pub use loader::{FrameLoadedEvent, LoadSequenceRequest, LoadingStats, SequenceAssets};

/// Plugin for mesh sequence management
pub struct SequencePlugin;

impl Plugin for SequencePlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins((
            discovery::SequenceDiscoveryPlugin,
            playback::SequencePlaybackPlugin,
            loader::SequenceLoaderPlugin,
        ))
        .init_resource::<SequenceManager>()
        .add_message::<SequenceEvent>();
    }
}

/// Resource that manages the current sequence state
#[derive(Resource)]
pub struct SequenceManager {
    /// Currently loaded sequence, if any
    pub current_sequence: Option<Sequence>,
    /// Current frame index in the sequence
    pub current_frame: usize,
    /// Whether playback is active
    pub is_playing: bool,
}

impl SequenceManager {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self {
            current_sequence: None,
            current_frame: 0,
            is_playing: false,
        }
    }
}

impl Default for SequenceManager {
    fn default() -> Self {
        Self {
            current_sequence: None,
            current_frame: 0,
            is_playing: false,
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
