//! Sequence management for mesh files
//!
//! This module provides functionality for discovering, loading, and playing back
//! sequences of mesh files (e.g., simulation timesteps).
//!
//! ## Architecture
//!
//! The core data model is split into pure library types and thin Bevy wrappers:
//!
//! - **[`LoadSection`]** — a `[start, end)` frame range. No UI state.
//! - **[`Timeline`]** — owns `total_frames`, `Vec<LoadSection>`, and `playhead`.
//!   All temporal logic lives here as plain methods.
//! - **[`SequenceManager`]** — playback controls (`is_playing`, `speed`,
//!   `loop_enabled`) and the loaded [`Sequence`] metadata.
//!
//! Pure functions:
//! - [`compute_spawn_window`] — given playhead + render window size + load
//!   section bounds, returns the `[lo, hi)` range of entities to keep spawned.
//! - [`distribute_sections`] — split a frame budget across N evenly-spaced
//!   load sections.
//!
//! Bevy systems (`advance_playback`, `update_timeline_budget`,
//! `sync_ui_from_sequence_manager`) are thin wrappers that read resources,
//! call the pure methods, and write results back.

pub mod discovery;
pub mod loader;
pub mod playback;
pub mod watch;

use bevy::prelude::*;
use std::path::PathBuf;

use crate::app::ui::state::UiState;
use crate::lib::memory_budget::MemoryBudget;

// Re-export commonly used types from loader
pub use loader::{
    FrameLoadedEvent, LoadSequenceRequest, LoadingStats, MeshFileFormat, SequenceAssets,
};
pub use watch::{LoadingPhase, LoadingPhaseState, WatchConfig, WatchPlugin};

// ---------------------------------------------------------------------------
// LoadSection — pure [start, end) range
// ---------------------------------------------------------------------------

/// A contiguous range of frames to keep loaded in memory.
///
/// This is a pure data struct — no UI state, no Bevy dependencies beyond the
/// derive. All mutation goes through the methods which enforce invariants.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadSection {
    /// Start frame index (inclusive)
    pub start: usize,
    /// End frame index (exclusive)
    pub end: usize,
}

impl Default for LoadSection {
    fn default() -> Self {
        Self {
            start: 0,
            end: usize::MAX, // Will be clamped to actual sequence length
        }
    }
}

impl LoadSection {
    /// Create a new load section
    pub fn new(start: usize, end: usize) -> Self {
        Self { start, end }
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

// ---------------------------------------------------------------------------
// Timeline — owns temporal state: total_frames, sections, playhead
// ---------------------------------------------------------------------------

/// The temporal model for a loaded sequence.
///
/// Pure data structure. Owns the full timeline state:
/// - How many frames exist (`total_frames`)
/// - Which ranges are loaded into memory (`sections`)
/// - Where the playhead sits (`playhead`)
///
/// All logic is exposed as plain methods that take plain arguments and return
/// plain results — no Bevy queries, no side effects.
#[derive(Resource, Debug, Clone)]
pub struct Timeline {
    /// Total number of frames in the sequence
    pub total_frames: usize,
    /// Loaded frame ranges (budget layer). At least one section exists after
    /// a sequence is loaded.
    pub sections: Vec<LoadSection>,
    /// Current playhead position (frame index)
    pub playhead: usize,
}

impl Default for Timeline {
    fn default() -> Self {
        Self {
            total_frames: 0,
            sections: vec![LoadSection::default()],
            playhead: 0,
        }
    }
}

impl Timeline {
    /// Create a timeline for a sequence with `total_frames` frames and a
    /// single load section covering `[0, initial_loaded)`.
    pub fn new(total_frames: usize, initial_loaded: usize) -> Self {
        Self {
            total_frames,
            sections: vec![LoadSection::new(0, initial_loaded.min(total_frames))],
            playhead: 0,
        }
    }

    // -- Playhead control ---------------------------------------------------

    /// Move to the next frame. Returns `true` if the playhead moved.
    pub fn next_frame(&mut self) -> bool {
        if self.total_frames > 0 && self.playhead + 1 < self.total_frames {
            self.playhead += 1;
            true
        } else {
            false
        }
    }

    /// Move to the previous frame. Returns `true` if the playhead moved.
    pub fn previous_frame(&mut self) -> bool {
        if self.playhead > 0 {
            self.playhead -= 1;
            true
        } else {
            false
        }
    }

    /// Jump to a specific frame. Returns `true` if the frame is valid.
    pub fn jump_to_frame(&mut self, frame: usize) -> bool {
        if self.total_frames > 0 && frame < self.total_frames {
            self.playhead = frame;
            true
        } else {
            false
        }
    }

    // -- Section queries ----------------------------------------------------

    /// Get the first section (convenience for single-section mode).
    pub fn section(&self) -> &LoadSection {
        &self.sections[0]
    }

    /// Get the first section mutably (convenience for single-section mode).
    pub fn section_mut(&mut self) -> &mut LoadSection {
        &mut self.sections[0]
    }

    /// Find the section containing `frame`, or `None`.
    pub fn section_containing(&self, frame: usize) -> Option<&LoadSection> {
        self.sections.iter().find(|s| s.contains(frame))
    }

    /// Find the index + ref of the section containing `frame`, or `None`.
    pub fn section_containing_indexed(&self, frame: usize) -> Option<(usize, &LoadSection)> {
        self.sections
            .iter()
            .enumerate()
            .find(|(_, s)| s.contains(frame))
    }

    /// Total number of loaded frames across all sections.
    pub fn total_loaded(&self) -> usize {
        self.sections.iter().map(|s| s.frame_count()).sum()
    }

    // -- Multi-section management -------------------------------------------

    /// Add a new section at `center`, splitting the remaining budget evenly.
    ///
    /// Returns the index of the new section, or `None` if there is no room
    /// (budget exhausted or `total_frames == 0`).
    pub fn add_section(&mut self, center: usize, max_frames: usize) -> Option<usize> {
        if self.total_frames == 0 || max_frames == 0 {
            return None;
        }

        let used: usize = self.sections.iter().map(|s| s.frame_count()).sum();
        let remaining = max_frames.saturating_sub(used);
        if remaining < 2 {
            return None; // Need at least 2 frames for a useful section
        }

        // New section gets the remaining budget, centered on `center`
        let half = remaining / 2;
        let start = center.saturating_sub(half);
        let end = (start + remaining).min(self.total_frames);
        let start = end.saturating_sub(remaining); // re-anchor if clamped

        let mut new_section = LoadSection::new(start, end);
        new_section.clamp(self.total_frames, remaining);

        // Resolve overlaps: shrink any existing section that overlaps the new one
        for existing in &mut self.sections {
            // If the new section overlaps an existing one, shrink the existing
            if new_section.start < existing.end && new_section.end > existing.start {
                // Shrink whichever side has less overlap
                let overlap_from_left = existing.end.saturating_sub(new_section.start);
                let overlap_from_right = new_section.end.saturating_sub(existing.start);
                if overlap_from_left <= overlap_from_right {
                    existing.end = new_section.start;
                } else {
                    existing.start = new_section.end;
                }
                if existing.start >= existing.end {
                    existing.end = existing.start; // degenerate — will be cleaned up
                }
            }
        }

        // Remove degenerate sections (zero-length)
        self.sections.retain(|s| s.frame_count() > 0);

        self.sections.push(new_section);

        // Keep sections sorted by start frame
        self.sections.sort_by_key(|s| s.start);

        Some(self.sections.len() - 1)
    }

    /// Remove the section at `index`.
    ///
    /// If this was the last section, a new default section covering `[0, max)`
    /// is inserted so the vec is never empty.
    pub fn remove_section(&mut self, index: usize, max_frames: usize) {
        if index < self.sections.len() {
            self.sections.remove(index);
        }
        if self.sections.is_empty() {
            let loaded = self.total_frames.min(max_frames);
            self.sections.push(LoadSection::new(0, loaded));
        }
    }

    /// Return the next loaded frame at or after `frame`, searching across all
    /// sections.  Returns `None` if no section contains any frame ≥ `frame`.
    pub fn next_loaded_frame(&self, frame: usize) -> Option<usize> {
        // If the frame is inside a section, return it directly
        if self.section_containing(frame).is_some() {
            return Some(frame);
        }
        // Otherwise find the nearest section that starts after `frame`
        self.sections
            .iter()
            .filter(|s| s.start >= frame)
            .map(|s| s.start)
            .min()
    }

    /// Return the previous loaded frame at or before `frame`, searching
    /// across all sections.  Returns `None` if no section contains any
    /// frame ≤ `frame`.
    pub fn prev_loaded_frame(&self, frame: usize) -> Option<usize> {
        if self.section_containing(frame).is_some() {
            return Some(frame);
        }
        self.sections
            .iter()
            .filter(|s| s.end > 0 && s.end.saturating_sub(1) <= frame)
            .map(|s| s.end - 1)
            .max()
    }

    /// Check whether `frame` falls inside any loaded section.
    pub fn is_frame_loaded(&self, frame: usize) -> bool {
        self.sections.iter().any(|s| s.contains(frame))
    }

    // -- Budget enforcement -------------------------------------------------

    /// Clamp every section to fit within `total_frames` and `max_frames`
    /// budget. This is the pure logic behind the `update_timeline_budget`
    /// system.
    pub fn enforce_budget(&mut self, max_frames: usize) {
        if self.sections.len() == 1 {
            self.sections[0].clamp(self.total_frames, max_frames);
            return;
        }

        // Multi-section: give each section an equal share of the budget,
        // then clamp individually.
        let per_section = if self.sections.is_empty() {
            0
        } else {
            (max_frames / self.sections.len()).max(1)
        };
        for section in &mut self.sections {
            section.clamp(self.total_frames, per_section);
        }
    }

    // -- Initialisation helpers ---------------------------------------------

    /// Reset the timeline for a newly-loaded sequence.
    pub fn init_sequence(&mut self, total_frames: usize, max_frames: usize) {
        self.total_frames = total_frames;
        self.playhead = 0;
        let loaded = total_frames.min(max_frames);
        self.sections = vec![LoadSection::new(0, loaded)];
    }

    /// Reset the timeline to `num_sections` evenly-distributed sections.
    pub fn init_multi_section(
        &mut self,
        total_frames: usize,
        max_frames: usize,
        num_sections: usize,
    ) {
        self.total_frames = total_frames;
        self.playhead = 0;
        self.sections = distribute_sections(total_frames, max_frames, num_sections);
        if self.sections.is_empty() {
            let loaded = total_frames.min(max_frames);
            self.sections = vec![LoadSection::new(0, loaded)];
        }
    }
}

// ---------------------------------------------------------------------------
// Pure functions — no Bevy, fully testable
// ---------------------------------------------------------------------------

/// Compute the `[lo, hi)` range of entities to keep spawned, given the
/// playhead position, a render window size, and the active load section
/// bounds.
///
/// The spawn window is centred on `playhead` with radius `window_size / 2`,
/// then clamped to `[section_start, section_end)` ∩ `[0, total_frames)`.
pub fn compute_spawn_window(
    playhead: usize,
    window_size: usize,
    section_start: usize,
    section_end: usize,
    total_frames: usize,
) -> (usize, usize) {
    let half = window_size / 2;
    let raw_lo = playhead.saturating_sub(half);
    let raw_hi = (playhead + half + 1).min(total_frames);
    let clamped_end = section_end.min(total_frames);
    let lo = raw_lo.max(section_start);
    let hi = raw_hi.min(clamped_end);
    if lo >= hi {
        (0, 0)
    } else {
        (lo, hi)
    }
}

/// Compute spawn windows across **all** sections in a timeline.
///
/// For each section that overlaps the render window centred on `playhead`,
/// returns a `(lo, hi)` range.  The caller should spawn entities for the
/// union of all returned ranges and despawn everything else.
///
/// This is the multi-section generalisation of [`compute_spawn_window`].
pub fn compute_spawn_windows(
    playhead: usize,
    window_size: usize,
    sections: &[LoadSection],
    total_frames: usize,
) -> Vec<(usize, usize)> {
    let mut windows = Vec::with_capacity(sections.len());
    for section in sections {
        let (lo, hi) = compute_spawn_window(
            playhead,
            window_size,
            section.start,
            section.end,
            total_frames,
        );
        if lo < hi {
            windows.push((lo, hi));
        }
    }
    windows
}

/// Distribute `num_sections` load sections of equal size across
/// `total_frames`, consuming at most `max_frames` total loaded frames.
///
/// Sections are evenly spaced. Overlaps are resolved by pushing later
/// sections rightward.
pub fn distribute_sections(
    total_frames: usize,
    max_frames: usize,
    num_sections: usize,
) -> Vec<LoadSection> {
    if num_sections == 0 || total_frames == 0 || max_frames == 0 {
        return Vec::new();
    }

    let budget = max_frames.min(total_frames);
    let per_section = (budget / num_sections).max(1);

    if num_sections == 1 {
        return vec![LoadSection::new(0, per_section.min(total_frames))];
    }

    // Evenly space section centres across the timeline
    let mut sections = Vec::with_capacity(num_sections);
    let slot_size = total_frames as f64 / num_sections as f64;

    for i in 0..num_sections {
        let center = (slot_size * (i as f64 + 0.5)) as usize;
        let half = per_section / 2;
        let mut start = center.saturating_sub(half);
        let mut end = start + per_section;

        // Clamp to total
        if end > total_frames {
            end = total_frames;
            start = end.saturating_sub(per_section);
        }

        sections.push(LoadSection::new(start, end));
    }

    // Resolve overlaps: push later sections right
    for i in 1..sections.len() {
        if sections[i].start < sections[i - 1].end {
            let count = sections[i].frame_count();
            sections[i].start = sections[i - 1].end;
            sections[i].end = (sections[i].start + count).min(total_frames);
        }
    }

    sections
}

// ---------------------------------------------------------------------------
// Bevy plugin + systems (thin wrappers)
// ---------------------------------------------------------------------------

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
        .init_resource::<Timeline>()
        .add_message::<SequenceEvent>()
        .add_systems(
            Update,
            (advance_playback, playback::handle_keyboard_playback)
                .before(sync_ui_from_sequence_manager),
        )
        .add_systems(
            Update,
            (sync_ui_from_sequence_manager, update_timeline_budget),
        );
    }
}

/// System: enforce memory budget on the timeline's load sections.
///
/// Thin wrapper — reads `MemoryBudget` + `SequenceAssets`, calls
/// `timeline.enforce_budget()`.
fn update_timeline_budget(
    mut timeline: ResMut<Timeline>,
    memory_budget: Res<MemoryBudget>,
    sequence_assets: Res<SequenceAssets>,
) {
    if sequence_assets.total_frames == 0 {
        return;
    }

    // Keep timeline.total_frames in sync (watch mode can add frames)
    timeline.total_frames = sequence_assets.total_frames;

    let max_frames = if memory_budget.max_frames < usize::MAX {
        memory_budget.max_frames
    } else {
        sequence_assets.total_frames
    };

    // Snapshot for change detection
    let old_sections = timeline.sections.clone();

    timeline.enforce_budget(max_frames);

    if timeline.sections != old_sections {
        debug!(
            "Timeline budget enforced: {:?} → {:?} (total: {}, max: {})",
            old_sections, timeline.sections, timeline.total_frames, max_frames,
        );
    }
}

// ---------------------------------------------------------------------------
// SequenceManager — playback controls + sequence metadata
// ---------------------------------------------------------------------------

/// Resource that manages playback state and the loaded sequence metadata.
///
/// **Not** the source of truth for the playhead — that lives in [`Timeline`].
/// This resource owns `is_playing`, `speed`, `loop_enabled`, and the
/// `Option<Sequence>` data.
#[derive(Resource)]
pub struct SequenceManager {
    /// Currently loaded sequence, if any
    pub current_sequence: Option<Sequence>,
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
            is_playing: false,
            speed: 1.0,
            loop_enabled: true,
        }
    }
}

impl SequenceManager {
    /// Load a new sequence (playhead reset is handled by Timeline)
    pub fn load_sequence(&mut self, sequence: Sequence) {
        self.current_sequence = Some(sequence);
        self.is_playing = false;
    }

    /// Get the current sequence
    pub fn current_sequence(&self) -> Option<&Sequence> {
        self.current_sequence.as_ref()
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

/// Timer-based auto-advance. Reads `SequenceManager` for playback state,
/// writes `Timeline.playhead`.
fn advance_playback(
    mut sm: ResMut<SequenceManager>,
    mut timeline: ResMut<Timeline>,
    time: Res<Time>,
    mut accumulator: Local<f32>,
) {
    if !sm.is_playing || timeline.total_frames == 0 {
        *accumulator = 0.0;
        return;
    }

    let frame_duration = 1.0 / (30.0 * sm.speed);
    *accumulator += time.delta_secs();

    while *accumulator >= frame_duration {
        *accumulator -= frame_duration;

        if timeline.playhead + 1 < timeline.total_frames {
            timeline.playhead += 1;
        } else if sm.loop_enabled {
            timeline.playhead = 0;
        } else {
            sm.is_playing = false;
            *accumulator = 0.0;
            break;
        }
    }
}

/// One-way sync: `SequenceManager` + `Timeline` → `UiState` for display.
fn sync_ui_from_sequence_manager(
    sm: Res<SequenceManager>,
    timeline: Res<Timeline>,
    mut ui_state: ResMut<UiState>,
    sequence_assets: Res<SequenceAssets>,
) {
    ui_state.playback.total_frames = timeline.total_frames;
    ui_state.playback.current_frame = timeline.playhead;
    ui_state.playback.is_playing = sm.is_playing;
    ui_state.playback.speed = sm.speed;
    ui_state.playback.loop_enabled = sm.loop_enabled;

    // Sync loading state
    ui_state.loading.update_from_assets(&sequence_assets);
}

// ---------------------------------------------------------------------------
// Sequence + FrameInfo (unchanged)
// ---------------------------------------------------------------------------

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
            .and_then(|n| n.to_str())
            .unwrap_or("unknown")
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

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Events emitted by the sequence system
#[derive(Message, Debug, Clone)]
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

// ===========================================================================
// Tests
// ===========================================================================

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

    // -----------------------------------------------------------------------
    // Timeline basics
    // -----------------------------------------------------------------------

    #[test]
    fn test_timeline_new() {
        let tl = Timeline::new(100, 50);
        assert_eq!(tl.total_frames, 100);
        assert_eq!(tl.playhead, 0);
        assert_eq!(tl.sections.len(), 1);
        assert_eq!(tl.sections[0], LoadSection::new(0, 50));
    }

    #[test]
    fn test_timeline_new_clamps_to_total() {
        let tl = Timeline::new(30, 100);
        assert_eq!(tl.sections[0], LoadSection::new(0, 30));
    }

    #[test]
    fn test_timeline_default() {
        let tl = Timeline::default();
        assert_eq!(tl.total_frames, 0);
        assert_eq!(tl.playhead, 0);
        assert_eq!(tl.sections.len(), 1);
    }

    // -----------------------------------------------------------------------
    // Timeline playhead
    // -----------------------------------------------------------------------

    #[test]
    fn test_next_frame() {
        let mut tl = Timeline::new(10, 10);
        assert!(tl.next_frame());
        assert_eq!(tl.playhead, 1);
    }

    #[test]
    fn test_next_frame_at_end() {
        let mut tl = Timeline::new(10, 10);
        tl.playhead = 9;
        assert!(!tl.next_frame());
        assert_eq!(tl.playhead, 9);
    }

    #[test]
    fn test_previous_frame() {
        let mut tl = Timeline::new(10, 10);
        tl.playhead = 5;
        assert!(tl.previous_frame());
        assert_eq!(tl.playhead, 4);
    }

    #[test]
    fn test_previous_frame_at_start() {
        let mut tl = Timeline::new(10, 10);
        assert!(!tl.previous_frame());
        assert_eq!(tl.playhead, 0);
    }

    #[test]
    fn test_jump_to_frame() {
        let mut tl = Timeline::new(100, 100);
        assert!(tl.jump_to_frame(50));
        assert_eq!(tl.playhead, 50);
    }

    #[test]
    fn test_jump_to_frame_out_of_bounds() {
        let mut tl = Timeline::new(100, 100);
        assert!(!tl.jump_to_frame(100));
        assert_eq!(tl.playhead, 0);
        assert!(!tl.jump_to_frame(200));
        assert_eq!(tl.playhead, 0);
    }

    #[test]
    fn test_jump_to_frame_empty() {
        let mut tl = Timeline::default();
        assert!(!tl.jump_to_frame(0));
    }

    // -----------------------------------------------------------------------
    // Timeline section helpers
    // -----------------------------------------------------------------------

    #[test]
    fn test_section_convenience() {
        let tl = Timeline::new(100, 50);
        assert_eq!(tl.section(), &LoadSection::new(0, 50));
    }

    #[test]
    fn test_section_containing() {
        let mut tl = Timeline::new(1000, 100);
        tl.sections = vec![LoadSection::new(0, 100), LoadSection::new(500, 600)];
        assert!(tl.section_containing(50).is_some());
        assert!(tl.section_containing(550).is_some());
        assert!(tl.section_containing(300).is_none());
    }

    #[test]
    fn test_total_loaded() {
        let mut tl = Timeline::new(1000, 100);
        tl.sections = vec![LoadSection::new(0, 100), LoadSection::new(500, 600)];
        assert_eq!(tl.total_loaded(), 200);
    }

    // -----------------------------------------------------------------------
    // Timeline::enforce_budget
    // -----------------------------------------------------------------------

    #[test]
    fn test_enforce_budget_single_section() {
        let mut tl = Timeline::new(1000, 1000);
        tl.enforce_budget(100);
        assert_eq!(tl.sections[0], LoadSection::new(0, 100));
    }

    #[test]
    fn test_enforce_budget_already_within() {
        let mut tl = Timeline::new(1000, 50);
        tl.enforce_budget(100);
        assert_eq!(tl.sections[0], LoadSection::new(0, 50));
    }

    #[test]
    fn test_enforce_budget_multi_section() {
        let mut tl = Timeline::new(1000, 100);
        tl.sections = vec![LoadSection::new(0, 500), LoadSection::new(500, 1000)];
        tl.enforce_budget(200);
        // Each section gets 100
        assert_eq!(tl.sections[0].frame_count(), 100);
        assert_eq!(tl.sections[1].frame_count(), 100);
    }

    // -----------------------------------------------------------------------
    // Timeline::init_sequence
    // -----------------------------------------------------------------------

    #[test]
    fn test_init_sequence() {
        let mut tl = Timeline::default();
        tl.playhead = 42;
        tl.init_sequence(500, 100);
        assert_eq!(tl.total_frames, 500);
        assert_eq!(tl.playhead, 0);
        assert_eq!(tl.sections.len(), 1);
        assert_eq!(tl.sections[0], LoadSection::new(0, 100));
    }

    #[test]
    fn test_init_sequence_budget_exceeds_total() {
        let mut tl = Timeline::default();
        tl.init_sequence(50, 200);
        assert_eq!(tl.sections[0], LoadSection::new(0, 50));
    }

    // -----------------------------------------------------------------------
    // compute_spawn_window
    // -----------------------------------------------------------------------

    #[test]
    fn test_spawn_window_basic() {
        // window_size=5 centered on frame 50, section [0, 100), total 100
        let (lo, hi) = compute_spawn_window(50, 5, 0, 100, 100);
        assert_eq!(lo, 48); // 50 - 2
        assert_eq!(hi, 53); // 50 + 2 + 1
    }

    #[test]
    fn test_spawn_window_clamps_to_section() {
        // Playhead at 5, window_size=20, but section only [0, 10)
        let (lo, hi) = compute_spawn_window(5, 20, 0, 10, 100);
        assert_eq!(lo, 0);
        assert_eq!(hi, 10);
    }

    #[test]
    fn test_spawn_window_playhead_outside_section() {
        // Playhead at 200, section [0, 100)
        let (lo, hi) = compute_spawn_window(200, 5, 0, 100, 500);
        // raw_lo=198, raw_hi=203, clamped to [0,100) → lo=198 > hi=100 → empty
        assert_eq!((lo, hi), (0, 0));
    }

    #[test]
    fn test_spawn_window_near_start() {
        let (lo, hi) = compute_spawn_window(1, 10, 0, 100, 100);
        assert_eq!(lo, 0);
        // half = 5, raw_lo = 1-5 = 0 (saturating), raw_hi = 1+5+1 = 7
        // clamped: lo = max(0, 0) = 0, hi = min(7, 100) = 7
        assert_eq!(hi, 7);
    }

    #[test]
    fn test_spawn_window_near_end() {
        let (lo, hi) = compute_spawn_window(98, 10, 0, 100, 100);
        // half=5, raw_lo=93, raw_hi=104 → clamped to 100
        assert_eq!(lo, 93);
        assert_eq!(hi, 100);
    }

    #[test]
    fn test_spawn_window_section_offset() {
        // Section starts at 50
        let (lo, hi) = compute_spawn_window(52, 5, 50, 100, 100);
        // raw_lo=50, raw_hi=55, section_start=50 → lo=50, hi=55
        assert_eq!(lo, 50);
        assert_eq!(hi, 55);
    }

    // -----------------------------------------------------------------------
    // distribute_sections
    // -----------------------------------------------------------------------

    #[test]
    fn test_distribute_single() {
        let sections = distribute_sections(100, 50, 1);
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0], LoadSection::new(0, 50));
    }

    #[test]
    fn test_distribute_two() {
        let sections = distribute_sections(100, 50, 2);
        assert_eq!(sections.len(), 2);
        let total: usize = sections.iter().map(|s| s.frame_count()).sum();
        assert!(total <= 50);
        // Both sections should be within [0, 100)
        for s in &sections {
            assert!(s.end <= 100);
        }
        // No overlap
        assert!(sections[0].end <= sections[1].start);
    }

    #[test]
    fn test_distribute_budget_larger_than_total() {
        let sections = distribute_sections(30, 100, 2);
        assert_eq!(sections.len(), 2);
        let total: usize = sections.iter().map(|s| s.frame_count()).sum();
        assert!(total <= 30);
    }

    #[test]
    fn test_distribute_zero_sections() {
        assert!(distribute_sections(100, 50, 0).is_empty());
    }

    #[test]
    fn test_distribute_zero_frames() {
        assert!(distribute_sections(0, 50, 2).is_empty());
    }

    #[test]
    fn test_distribute_zero_budget() {
        assert!(distribute_sections(100, 0, 2).is_empty());
    }

    #[test]
    fn test_distribute_many_sections() {
        let sections = distribute_sections(1000, 200, 4);
        assert_eq!(sections.len(), 4);
        let total: usize = sections.iter().map(|s| s.frame_count()).sum();
        assert!(total <= 200);
        // No overlaps
        for i in 1..sections.len() {
            assert!(
                sections[i].start >= sections[i - 1].end,
                "Overlap between section {} and {}: {:?} vs {:?}",
                i - 1,
                i,
                sections[i - 1],
                sections[i],
            );
        }
    }

    // ===================================================================
    // Multi-section management
    // ===================================================================

    #[test]
    fn test_add_section_basic() {
        let mut tl = Timeline::new(100, 30);
        // First section is [0, 30)
        assert_eq!(tl.sections.len(), 1);

        // Add a section centred on frame 70, with 50 budget total
        let idx = tl.add_section(70, 50);
        assert!(idx.is_some());
        assert_eq!(tl.sections.len(), 2);
        // New section should be near frame 70
        let new = &tl.sections[1];
        assert!(new.contains(70));
    }

    #[test]
    fn test_add_section_no_budget() {
        let mut tl = Timeline::new(100, 30);
        // Section 0 uses all 30 frames of budget
        assert_eq!(tl.add_section(70, 30), None);
    }

    #[test]
    fn test_add_section_tiny_budget() {
        let mut tl = Timeline::new(100, 10);
        // Budget is 10, section uses 10, remaining is 0
        assert_eq!(tl.add_section(50, 10), None);
        // Even with budget=11, remaining=1 which is < 2
        assert_eq!(tl.add_section(50, 11), None);
    }

    #[test]
    fn test_remove_section() {
        let mut tl = Timeline::new(100, 100);
        tl.sections = vec![LoadSection::new(0, 30), LoadSection::new(50, 80)];
        tl.remove_section(0, 100);
        assert_eq!(tl.sections.len(), 1);
        assert_eq!(tl.sections[0].start, 50);
    }

    #[test]
    fn test_remove_last_section_restores_default() {
        let mut tl = Timeline::new(100, 40);
        tl.sections = vec![LoadSection::new(10, 20)];
        tl.remove_section(0, 40);
        // Should restore a default section
        assert_eq!(tl.sections.len(), 1);
        assert_eq!(tl.sections[0].start, 0);
        assert_eq!(tl.sections[0].end, 40);
    }

    #[test]
    fn test_next_loaded_frame() {
        let mut tl = Timeline::new(100, 100);
        tl.sections = vec![LoadSection::new(10, 20), LoadSection::new(50, 60)];
        // Inside a section → same frame
        assert_eq!(tl.next_loaded_frame(15), Some(15));
        // In a gap → jump to next section start
        assert_eq!(tl.next_loaded_frame(25), Some(50));
        // Before all sections → first section start
        assert_eq!(tl.next_loaded_frame(5), Some(10));
        // After all sections → None
        assert_eq!(tl.next_loaded_frame(60), None);
    }

    #[test]
    fn test_prev_loaded_frame() {
        let mut tl = Timeline::new(100, 100);
        tl.sections = vec![LoadSection::new(10, 20), LoadSection::new(50, 60)];
        // Inside a section → same frame
        assert_eq!(tl.prev_loaded_frame(55), Some(55));
        // In a gap → jump to prev section end-1
        assert_eq!(tl.prev_loaded_frame(30), Some(19));
        // Before all sections → None
        assert_eq!(tl.prev_loaded_frame(5), None);
    }

    #[test]
    fn test_is_frame_loaded() {
        let mut tl = Timeline::new(100, 100);
        tl.sections = vec![LoadSection::new(10, 20), LoadSection::new(50, 60)];
        assert!(tl.is_frame_loaded(15));
        assert!(tl.is_frame_loaded(55));
        assert!(!tl.is_frame_loaded(30));
        assert!(!tl.is_frame_loaded(5));
    }

    #[test]
    fn test_init_multi_section() {
        let mut tl = Timeline::default();
        tl.init_multi_section(100, 40, 2);
        assert_eq!(tl.sections.len(), 2);
        let total: usize = tl.sections.iter().map(|s| s.frame_count()).sum();
        assert!(total <= 40);
    }

    #[test]
    fn test_section_containing_indexed() {
        let mut tl = Timeline::new(100, 100);
        tl.sections = vec![LoadSection::new(0, 30), LoadSection::new(50, 80)];
        assert_eq!(tl.section_containing_indexed(10).map(|(i, _)| i), Some(0));
        assert_eq!(tl.section_containing_indexed(60).map(|(i, _)| i), Some(1));
        assert_eq!(tl.section_containing_indexed(35), None);
    }

    // ===================================================================
    // compute_spawn_windows (multi-section)
    // ===================================================================

    #[test]
    fn test_spawn_windows_single_section() {
        let sections = vec![LoadSection::new(0, 100)];
        let wins = compute_spawn_windows(50, 10, &sections, 100);
        assert_eq!(wins.len(), 1);
        assert_eq!(wins[0], (45, 56));
    }

    #[test]
    fn test_spawn_windows_multi_section_playhead_in_first() {
        let sections = vec![LoadSection::new(0, 30), LoadSection::new(60, 90)];
        let wins = compute_spawn_windows(15, 10, &sections, 100);
        // Only the first section overlaps the render window around frame 15
        assert_eq!(wins.len(), 1);
        assert!(wins[0].1 <= 30);
    }

    #[test]
    fn test_spawn_windows_playhead_in_gap() {
        let sections = vec![LoadSection::new(0, 30), LoadSection::new(60, 90)];
        // Playhead at 45 (in the gap) with a huge window should catch both sections
        let wins = compute_spawn_windows(45, 100, &sections, 100);
        assert_eq!(wins.len(), 2);
    }

    #[test]
    fn test_spawn_windows_empty_sections() {
        let sections: Vec<LoadSection> = vec![];
        let wins = compute_spawn_windows(50, 10, &sections, 100);
        assert!(wins.is_empty());
    }
}
