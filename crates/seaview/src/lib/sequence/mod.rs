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
use crate::lib::memory_budget::MemoryBudget;

// Re-export commonly used types from loader
pub use loader::{FrameLoadedEvent, LoadSequenceRequest, LoadingStats, SequenceAssets};
pub use watch::{LoadingPhase, LoadingPhaseState, WatchConfig, WatchPlugin};

// ---------------------------------------------------------------------------
// Load Section — runtime resource for frame range in memory
// ---------------------------------------------------------------------------

/// Resource defining which frames should be kept in memory.
///
/// This is the runtime representation that the loader uses to decide
/// which frames to spawn/despawn. It's synced with `LoadSectionSettings`
/// from the per-directory config.
#[derive(Resource, Debug, Clone)]
pub struct LoadSection {
    /// Start frame index (inclusive)
    pub start: usize,
    /// End frame index (exclusive)
    pub end: usize,
    /// Whether the user is currently dragging an edge
    pub dragging: Option<LoadSectionEdge>,
}

/// Which edge of the load section is being dragged
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadSectionEdge {
    Start,
    End,
}

impl Default for LoadSection {
    fn default() -> Self {
        Self {
            start: 0,
            end: usize::MAX, // Will be clamped to actual sequence length
            dragging: None,
        }
    }
}

impl LoadSection {
    /// Create a new load section
    pub fn new(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            dragging: None,
        }
    }

    /// Get the number of frames in this section
    pub fn frame_count(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Check if a frame index is within this section
    pub fn contains(&self, frame: usize) -> bool {
        frame >= self.start && frame < self.end
    }

    /// Clamp the section to valid bounds given total frames and max allowed
    pub fn clamp(&mut self, total_frames: usize, max_frames: usize) {
        // Ensure end doesn't exceed total
        self.end = self.end.min(total_frames);
        // Ensure start is before end
        self.start = self.start.min(self.end);
        // Ensure we don't exceed max frames
        let current_count = self.frame_count();
        if current_count > max_frames {
            // Shrink from the end
            self.end = self.start + max_frames;
        }
    }

    /// Set the start edge, respecting constraints
    pub fn set_start(&mut self, new_start: usize, max_frames: usize) {
        self.start = new_start.min(self.end.saturating_sub(1));
        // Ensure we don't exceed max frames
        if self.frame_count() > max_frames {
            self.end = self.start + max_frames;
        }
    }

    /// Set the end edge, respecting constraints
    pub fn set_end(&mut self, new_end: usize, total_frames: usize, max_frames: usize) {
        self.end = new_end.min(total_frames).max(self.start + 1);
        // Ensure we don't exceed max frames
        if self.frame_count() > max_frames {
            self.start = self.end.saturating_sub(max_frames);
        }
    }

    /// Center the section on a frame, keeping the same size
    pub fn center_on(&mut self, frame: usize, total_frames: usize) {
        let size = self.frame_count();
        let half = size / 2;
        let new_start = frame.saturating_sub(half);
        let new_end = new_start + size;

        if new_end <= total_frames {
            self.start = new_start;
            self.end = new_end;
        } else {
            self.end = total_frames;
            self.start = self.end.saturating_sub(size);
        }
    }
}

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
        .init_resource::<LoadSection>()
        .add_message::<SequenceEvent>()
        .add_systems(
            Update,
            (advance_playback, playback::handle_keyboard_playback)
                .before(sync_ui_from_sequence_manager),
        )
        .add_systems(
            Update,
            (
                sync_ui_from_sequence_manager,
                update_load_section_from_budget,
            ),
        );
    }
}

/// System to update load section when memory budget changes
fn update_load_section_from_budget(
    mut load_section: ResMut<LoadSection>,
    memory_budget: Res<MemoryBudget>,
    sequence_assets: Res<SequenceAssets>,
) {
    // Only update if we have valid data and the section needs clamping
    if sequence_assets.total_frames == 0 {
        return;
    }

    let max_frames = if memory_budget.max_frames < usize::MAX {
        memory_budget.max_frames
    } else {
        sequence_assets.total_frames
    };

    let old_start = load_section.start;
    let old_end = load_section.end;

    // Clamp the load section to valid bounds
    load_section.clamp(sequence_assets.total_frames, max_frames);

    // Log if section changed
    if load_section.start != old_start || load_section.end != old_end {
        debug!(
            "LoadSection clamped: [{}..{}) → [{}..{}) (total: {}, max: {})",
            old_start,
            old_end,
            load_section.start,
            load_section.end,
            sequence_assets.total_frames,
            max_frames
        );
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
fn advance_playback(mut sm: ResMut<SequenceManager>, time: Res<Time>, mut accumulator: Local<f32>) {
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

#[cfg(test)]
mod tests {
    use super::*;

    // -----------------------------------------------------------------------
    // LoadSection basics
    // -----------------------------------------------------------------------

    #[test]
    fn test_load_section_new() {
        let ls = LoadSection::new(10, 50);
        assert_eq!(ls.start, 10);
        assert_eq!(ls.end, 50);
        assert_eq!(ls.frame_count(), 40);
        assert!(ls.dragging.is_none());
    }

    #[test]
    fn test_load_section_default_covers_everything() {
        let ls = LoadSection::default();
        assert_eq!(ls.start, 0);
        assert_eq!(ls.end, usize::MAX);
    }

    #[test]
    fn test_load_section_frame_count() {
        assert_eq!(LoadSection::new(0, 100).frame_count(), 100);
        assert_eq!(LoadSection::new(50, 50).frame_count(), 0);
        assert_eq!(LoadSection::new(50, 30).frame_count(), 0); // saturating_sub
    }

    #[test]
    fn test_load_section_contains() {
        let ls = LoadSection::new(10, 20);
        assert!(!ls.contains(9));
        assert!(ls.contains(10));
        assert!(ls.contains(15));
        assert!(ls.contains(19));
        assert!(!ls.contains(20)); // exclusive end
        assert!(!ls.contains(100));
    }

    // -----------------------------------------------------------------------
    // LoadSection::clamp
    // -----------------------------------------------------------------------

    #[test]
    fn test_clamp_end_to_total_frames() {
        let mut ls = LoadSection::new(0, 500);
        ls.clamp(200, 1000);
        assert_eq!(ls.start, 0);
        assert_eq!(ls.end, 200);
    }

    #[test]
    fn test_clamp_start_past_end() {
        let mut ls = LoadSection::new(300, 200);
        ls.clamp(500, 1000);
        // start clamped to min(300, 200) = 200
        assert_eq!(ls.start, 200);
        assert_eq!(ls.end, 200);
    }

    #[test]
    fn test_clamp_exceeds_max_frames() {
        let mut ls = LoadSection::new(0, 500);
        ls.clamp(500, 100);
        assert_eq!(ls.start, 0);
        assert_eq!(ls.end, 100);
        assert_eq!(ls.frame_count(), 100);
    }

    #[test]
    fn test_clamp_default_to_real_sequence() {
        let mut ls = LoadSection::default(); // 0..usize::MAX
        ls.clamp(812, 812);
        assert_eq!(ls.start, 0);
        assert_eq!(ls.end, 812);
    }

    #[test]
    fn test_clamp_default_with_budget() {
        let mut ls = LoadSection::default();
        ls.clamp(812, 200);
        assert_eq!(ls.start, 0);
        assert_eq!(ls.end, 200);
    }

    #[test]
    fn test_clamp_already_within_bounds() {
        let mut ls = LoadSection::new(10, 50);
        ls.clamp(500, 100);
        assert_eq!(ls.start, 10);
        assert_eq!(ls.end, 50);
    }

    // -----------------------------------------------------------------------
    // LoadSection::set_start / set_end
    // -----------------------------------------------------------------------

    #[test]
    fn test_set_start_normal() {
        let mut ls = LoadSection::new(10, 100);
        ls.set_start(20, 200);
        assert_eq!(ls.start, 20);
        assert_eq!(ls.end, 100);
    }

    #[test]
    fn test_set_start_past_end() {
        let mut ls = LoadSection::new(10, 50);
        ls.set_start(80, 200);
        // start clamped to end - 1 = 49
        assert_eq!(ls.start, 49);
    }

    #[test]
    fn test_set_start_exceeds_max_frames() {
        let mut ls = LoadSection::new(50, 200);
        ls.set_start(0, 50);
        assert_eq!(ls.start, 0);
        // end shrunk to start + max = 50
        assert_eq!(ls.end, 50);
    }

    #[test]
    fn test_set_end_normal() {
        let mut ls = LoadSection::new(10, 100);
        ls.set_end(200, 500, 300);
        assert_eq!(ls.start, 10);
        assert_eq!(ls.end, 200);
    }

    #[test]
    fn test_set_end_past_total() {
        let mut ls = LoadSection::new(10, 100);
        ls.set_end(600, 500, 1000);
        assert_eq!(ls.end, 500);
    }

    #[test]
    fn test_set_end_before_start() {
        let mut ls = LoadSection::new(50, 100);
        ls.set_end(30, 500, 1000);
        // end clamped to max(30, start + 1) = 51
        assert_eq!(ls.end, 51);
    }

    #[test]
    fn test_set_end_exceeds_max_frames() {
        let mut ls = LoadSection::new(10, 100);
        ls.set_end(400, 500, 50);
        assert_eq!(ls.end, 400);
        // start pushed forward: end - max = 350
        assert_eq!(ls.start, 350);
    }

    // -----------------------------------------------------------------------
    // LoadSection::center_on
    // -----------------------------------------------------------------------

    #[test]
    fn test_center_on_middle() {
        let mut ls = LoadSection::new(0, 100);
        ls.center_on(250, 500);
        assert_eq!(ls.start, 200);
        assert_eq!(ls.end, 300);
        assert_eq!(ls.frame_count(), 100);
    }

    #[test]
    fn test_center_on_near_start() {
        let mut ls = LoadSection::new(0, 100);
        ls.center_on(10, 500);
        // Would go to -40..60, but saturating_sub clamps to 0
        assert_eq!(ls.start, 0);
        assert_eq!(ls.end, 100);
    }

    #[test]
    fn test_center_on_near_end() {
        let mut ls = LoadSection::new(0, 100);
        ls.center_on(480, 500);
        // Would go to 430..530, but clamps to end of sequence
        assert_eq!(ls.end, 500);
        assert_eq!(ls.start, 400);
        assert_eq!(ls.frame_count(), 100);
    }

    #[test]
    fn test_center_on_preserves_size() {
        let mut ls = LoadSection::new(0, 50);
        ls.center_on(200, 1000);
        assert_eq!(ls.frame_count(), 50);
    }
}
