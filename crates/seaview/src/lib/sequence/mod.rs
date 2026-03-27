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
use crate::lib::actions::Action;
use crate::lib::memory_budget::MemoryBudget;
use crate::lib::series::{SeriesId, SeriesManager};

// Re-export commonly used types from loader
pub use loader::{
    FrameLoadedEvent, FrameStore, LoadSequenceRequest, LoaderConfig, LoadingStats, SpatialIndex,
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
    /// Whether this section is active (frames loaded into memory).
    /// Inactive sections are bookmarks — visible in the UI but greyed out,
    /// no memory cost.
    pub active: bool,
}

impl Default for LoadSection {
    fn default() -> Self {
        Self {
            start: 0,
            end: usize::MAX, // Will be clamped to actual sequence length
            active: true,
        }
    }
}

impl LoadSection {
    /// Create a new load section (active by default)
    pub fn new(start: usize, end: usize) -> Self {
        Self {
            start,
            end,
            active: true,
        }
    }

    /// Get the number of frames in this section
    pub fn frame_count(&self) -> usize {
        self.end.saturating_sub(self.start)
    }

    /// Check if a frame index is within this section's range.
    /// Note: this checks the *range* regardless of active status.
    pub fn contains(&self, frame: usize) -> bool {
        frame >= self.start && frame < self.end
    }

    /// Centre frame of this section.
    pub fn center(&self) -> usize {
        self.start + self.frame_count() / 2
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

    /// Set the start edge.  No budget constraint — sections can be any size.
    /// Only prevents start from going past end.
    pub fn set_start(&mut self, new_start: usize) {
        self.start = new_start.min(self.end.saturating_sub(1));
    }

    /// Set the end edge.  No budget constraint — sections can be any size.
    /// Only prevents end from going before start or past total_frames.
    pub fn set_end(&mut self, new_end: usize, total_frames: usize) {
        self.end = new_end.min(total_frames).max(self.start + 1);
    }

    /// Move the section so it starts at `new_start`, keeping the same size.
    /// Clamps to `[0, total_frames)`.
    pub fn move_to(&mut self, new_start: usize, total_frames: usize) {
        let size = self.frame_count();
        let clamped_start = new_start.min(total_frames.saturating_sub(size));
        self.start = clamped_start;
        self.end = clamped_start + size;
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
        self.sections.iter().find(|s| s.active && s.contains(frame))
    }

    /// Find the index + ref of the section containing `frame`, or `None`.
    pub fn section_containing_indexed(&self, frame: usize) -> Option<(usize, &LoadSection)> {
        self.sections
            .iter()
            .enumerate()
            .find(|(_, s)| s.active && s.contains(frame))
    }

    /// Like `section_containing_indexed` but matches ANY section (active or
    /// inactive).  Used by the UI for hit-testing.
    pub fn section_containing_indexed_any(&self, frame: usize) -> Option<(usize, &LoadSection)> {
        self.sections
            .iter()
            .enumerate()
            .find(|(_, s)| s.contains(frame))
    }

    /// Total number of loaded frames across all **active** sections.
    pub fn total_loaded(&self) -> usize {
        self.sections
            .iter()
            .filter(|s| s.active)
            .map(|s| s.frame_count())
            .sum()
    }

    // -- Multi-section management -------------------------------------------

    /// Add a new section centred on `center`.
    ///
    /// No budget constraints — the section is created at a reasonable default
    /// size and marked **active**.  The caller should follow up with
    /// [`enforce_budget`] to deactivate distant sections if needed.
    ///
    /// Returns the index of the new section, or `None` if `total_frames == 0`
    /// or there is no gap to place it in.
    pub fn add_section(&mut self, center: usize, max_frames: usize) -> Option<usize> {
        if self.total_frames == 0 {
            return None;
        }

        // Default size: average of existing sections, or 10% of total, at least 2.
        let avg_existing = if self.sections.is_empty() {
            (self.total_frames / 10).max(2)
        } else {
            let total_used: usize = self.sections.iter().map(|s| s.frame_count()).sum();
            (total_used / self.sections.len()).max(2)
        };
        let section_size = avg_existing.min(self.total_frames).max(2);

        let half = section_size / 2;
        let start = center.saturating_sub(half);
        let end = (start + section_size).min(self.total_frames);
        let start = end.saturating_sub(section_size); // re-anchor if clamped

        let new_section = LoadSection::new(start, end);

        // Minimum gap (in frames) to leave between sections so their edge
        // handles don't overlap in the UI.  Use ~0.5% of total frames,
        // clamped to [1, 20].
        let min_gap = (self.total_frames / 200).clamp(1, 20);

        // Find the tightest left and right bounds imposed by existing sections.
        let mut left_bound = 0usize;
        let mut right_bound = self.total_frames;
        for existing in &self.sections {
            if existing.end <= center {
                left_bound = left_bound.max(existing.end + min_gap);
            }
            if existing.start >= center {
                right_bound = right_bound.min(existing.start.saturating_sub(min_gap));
            }
            if existing.start < center && existing.end > center {
                return None;
            }
        }

        if left_bound >= right_bound || right_bound.saturating_sub(left_bound) < 2 {
            return None; // No room between neighbours
        }

        // Clamp new section into the available slot.
        let mut ns = new_section;
        ns.start = ns.start.max(left_bound);
        ns.end = ns.end.min(right_bound);
        if ns.start >= ns.end {
            ns.start = left_bound;
            ns.end = right_bound.min(left_bound + section_size);
        }
        if ns.frame_count() < 2 {
            return None;
        }

        let new_start = ns.start;
        let new_end = ns.end;

        self.sections.push(ns);
        self.sections.sort_by_key(|s| s.start);

        let idx = self
            .sections
            .iter()
            .position(|s| s.start == new_start && s.end == new_end)
            .unwrap_or(self.sections.len() - 1);

        // Auto-activate and enforce budget (may deactivate distant sections).
        self.sections[idx].active = true;
        self.enforce_budget(max_frames);

        Some(idx)
    }

    /// Add a minimal section starting at `start_frame`.
    ///
    /// Creates a section just 2 frames wide (or as small as the gap allows).
    /// Designed for the drag-to-paint gesture: the caller creates the section
    /// here, then immediately enters a drag on the `End` edge so the user
    /// can sweep rightward to size it.
    ///
    /// No budget constraints on size — budget is enforced via visibility.
    ///
    /// Returns the index of the new section in the (sorted) `sections` vec,
    /// or `None` if there is no room.
    pub fn add_section_at(&mut self, start_frame: usize, max_frames: usize) -> Option<usize> {
        if self.total_frames == 0 {
            return None;
        }

        // Minimum gap between sections (same as add_section).
        let min_gap = (self.total_frames / 200).clamp(1, 20);

        // Find the slot the new section must fit into.
        let mut left_bound = 0usize;
        let mut right_bound = self.total_frames;
        for existing in &self.sections {
            if existing.end <= start_frame {
                left_bound = left_bound.max(existing.end + min_gap);
            }
            if existing.start > start_frame {
                right_bound = right_bound.min(existing.start.saturating_sub(min_gap));
            }
            if existing.contains(start_frame) {
                return None;
            }
        }

        let clamped_start = start_frame.max(left_bound);
        if clamped_start >= right_bound || right_bound.saturating_sub(clamped_start) < 2 {
            return None;
        }

        // Minimal section: just 2 frames (user will drag the End edge).
        let initial_size = 2.min(right_bound - clamped_start);
        if initial_size < 2 {
            return None;
        }
        let new_section = LoadSection::new(clamped_start, clamped_start + initial_size);

        let new_start = new_section.start;
        let new_end = new_section.end;

        self.sections.push(new_section);
        self.sections.sort_by_key(|s| s.start);

        let idx = self
            .sections
            .iter()
            .position(|s| s.start == new_start && s.end == new_end)
            .unwrap_or(self.sections.len() - 1);

        // Auto-activate and enforce budget.
        self.sections[idx].active = true;
        self.enforce_budget(max_frames);

        Some(idx)
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
        // If the frame is inside an active section, return it directly
        if self.section_containing(frame).is_some() {
            return Some(frame);
        }
        // Otherwise find the nearest active section that starts after `frame`
        self.sections
            .iter()
            .filter(|s| s.active && s.start >= frame)
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
            .filter(|s| s.active && s.end > 0 && s.end.saturating_sub(1) <= frame)
            .map(|s| s.end - 1)
            .max()
    }

    /// Check whether `frame` falls inside any **active** loaded section.
    pub fn is_frame_loaded(&self, frame: usize) -> bool {
        self.sections.iter().any(|s| s.active && s.contains(frame))
    }

    /// Check whether `frame` falls inside ANY section (active or inactive).
    pub fn is_frame_in_any_section(&self, frame: usize) -> bool {
        self.sections.iter().any(|s| s.contains(frame))
    }

    // -- Budget enforcement -------------------------------------------------

    /// Enforce the memory budget by toggling section **active** flags.
    ///
    /// Sections are never resized — only deactivated (furthest from playhead
    /// first) until the total active frame count fits within `max_frames`.
    /// This is the pure logic behind the `update_timeline_budget` system.
    pub fn enforce_budget(&mut self, max_frames: usize) {
        // Clamp each section to total_frames (no section can extend past the
        // end of the sequence).
        for section in &mut self.sections {
            if section.end > self.total_frames {
                section.end = self.total_frames;
            }
            if section.start > section.end {
                section.start = section.end;
            }
        }

        // Check whether active sections exceed the budget.
        let active_loaded: usize = self
            .sections
            .iter()
            .filter(|s| s.active)
            .map(|s| s.frame_count())
            .sum();

        if active_loaded <= max_frames {
            return; // Within budget — nothing to do.
        }

        // Deactivate sections furthest from the playhead until we fit.
        // Build a list of active section indices sorted by distance from
        // playhead (furthest first).
        let playhead = self.playhead;
        let mut active_indices: Vec<usize> = self
            .sections
            .iter()
            .enumerate()
            .filter(|(_, s)| s.active)
            .map(|(i, _)| i)
            .collect();

        active_indices.sort_by(|&a, &b| {
            let da = self.sections[a].center().abs_diff(playhead);
            let db = self.sections[b].center().abs_diff(playhead);
            db.cmp(&da) // furthest first
        });

        let mut current_loaded = active_loaded;
        for &idx in &active_indices {
            if current_loaded <= max_frames {
                break;
            }
            // Don't deactivate the section the playhead is in.
            if self.sections[idx].contains(playhead) {
                continue;
            }
            current_loaded -= self.sections[idx].frame_count();
            self.sections[idx].active = false;
        }
    }

    /// Activate section at `index`, deactivating the furthest-from-playhead
    /// active section(s) if needed to stay within `max_frames` budget.
    pub fn activate_section(&mut self, index: usize, max_frames: usize) {
        if index >= self.sections.len() || self.sections[index].active {
            return;
        }
        self.sections[index].active = true;

        // Check if we've blown the budget and need to deactivate others.
        let active_loaded: usize = self
            .sections
            .iter()
            .filter(|s| s.active)
            .map(|s| s.frame_count())
            .sum();

        if active_loaded <= max_frames {
            return;
        }

        // Deactivate furthest-from-playhead active sections (excluding the
        // one we just activated and the one containing the playhead).
        let playhead = self.playhead;
        let mut candidates: Vec<usize> = self
            .sections
            .iter()
            .enumerate()
            .filter(|(i, s)| s.active && *i != index && !s.contains(playhead))
            .map(|(i, _)| i)
            .collect();

        candidates.sort_by(|&a, &b| {
            let da = self.sections[a].center().abs_diff(playhead);
            let db = self.sections[b].center().abs_diff(playhead);
            db.cmp(&da) // furthest first
        });

        let mut current_loaded = active_loaded;
        for &idx in &candidates {
            if current_loaded <= max_frames {
                break;
            }
            current_loaded -= self.sections[idx].frame_count();
            self.sections[idx].active = false;
        }
    }

    /// Deactivate section at `index`.
    pub fn deactivate_section(&mut self, index: usize) {
        if index < self.sections.len() {
            self.sections[index].active = false;
        }
    }

    // -- Watch / incremental growth -----------------------------------------

    /// Expand any active section whose `end` was pinned to the old sequence
    /// end so it grows to cover newly-arrived frames.
    ///
    /// A section is considered "pinned to the end" when its `end` equals
    /// `old_total` — i.e. it extended all the way to the last known frame.
    /// When the sequence grows (watch mode, incremental load), these sections
    /// should grow with it so the new frames become loadable without manual
    /// intervention.
    ///
    /// `self.total_frames` must already be set to the **new** total before
    /// calling this method.
    ///
    /// Returns the number of sections that were expanded (useful for logging).
    pub fn expand_trailing_sections(&mut self, old_total: usize) -> usize {
        if self.total_frames <= old_total {
            return 0; // Sequence didn't grow — nothing to do.
        }
        let new_total = self.total_frames;
        let mut expanded = 0;
        for section in &mut self.sections {
            if section.active && section.end == old_total {
                section.end = new_total;
                expanded += 1;
            }
        }
        expanded
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
        if !section.active {
            continue;
        }
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

/// Return the set of frame indices that should have assets loaded, based on
/// the timeline's loaded sections.  This is the **authoritative** answer to
/// "which frames should we ask the asset server to load?" and must be used
/// by the loader instead of blindly loading every frame in the sequence.
///
/// Indices are yielded in order, one per loaded section range.
pub fn frames_to_load(sections: &[LoadSection], total_frames: usize) -> Vec<usize> {
    let mut out = Vec::new();
    for section in sections {
        if !section.active {
            continue;
        }
        let end = section.end.min(total_frames);
        let start = section.start.min(end);
        out.extend(start..end);
    }
    out
}

/// Compute the **union** of needed frame indices across multiple series'
/// section lists.
///
/// Each entry in `all_sections` is the `(sections, total_frames)` pair for
/// one series.  The result is a sorted, deduplicated `Vec<usize>` of all
/// frame indices that any series needs loaded.
///
/// This is the multi-series generalisation of [`frames_to_load`] — the
/// loader calls this to determine which files [`MeshCache`] should hold.
///
/// ```ignore
/// // Series A wants [0..100], series B wants [80..200]
/// // Union: [0..200]
/// let union = union_frames_to_load(&[
///     (&series_a.sections, 300),
///     (&series_b.sections, 300),
/// ]);
/// ```
pub fn union_frames_to_load(all_sections: &[(&[LoadSection], usize)]) -> Vec<usize> {
    // For a single series, skip the dedup overhead.
    if all_sections.len() == 1 {
        let (sections, total) = all_sections[0];
        return frames_to_load(sections, total);
    }

    // Collect all frame indices, then sort + dedup.
    let mut out = Vec::new();
    for &(sections, total_frames) in all_sections {
        for section in sections {
            if !section.active {
                continue;
            }
            let end = section.end.min(total_frames);
            let start = section.start.min(end);
            out.extend(start..end);
        }
    }
    out.sort_unstable();
    out.dedup();
    out
}

// ---------------------------------------------------------------------------
// Bevy plugin + systems (thin wrappers)
// ---------------------------------------------------------------------------

/// Bevy resource wrapper for [`SeriesManager`] (which is a pure lib type
/// without Bevy derives).
///
/// Access the inner manager via `Deref` / `DerefMut`:
/// ```ignore
/// fn my_system(mgr: Res<SeriesManagerRes>) {
///     for series in mgr.iter() { /* ... */ }
/// }
/// ```
#[derive(Resource, Debug, Clone)]
pub struct SeriesManagerRes(pub SeriesManager);

impl Default for SeriesManagerRes {
    fn default() -> Self {
        Self(SeriesManager::new())
    }
}

impl std::ops::Deref for SeriesManagerRes {
    type Target = SeriesManager;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl std::ops::DerefMut for SeriesManagerRes {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.0
    }
}

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
        .init_resource::<SeriesManagerRes>()
        .add_message::<SequenceEvent>()
        .add_systems(
            Update,
            (advance_playback, playback::handle_keyboard_playback).before(sync_series_manager),
        )
        .add_systems(
            Update,
            (
                sync_series_manager,
                sync_ui_from_sequence_manager,
                update_timeline_budget,
            )
                .chain(),
        );
    }
}

/// System: enforce memory budget on the timeline's load sections.
///
/// Thin wrapper — reads `MemoryBudget` + `FrameStore`, calls
/// `timeline.enforce_budget()`.
fn update_timeline_budget(
    mut timeline: ResMut<Timeline>,
    memory_budget: Res<MemoryBudget>,
    frame_store: Res<FrameStore>,
    mut last_max_frames: Local<usize>,
) {
    if frame_store.total_frames == 0 {
        return;
    }

    let max_frames = if memory_budget.max_frames < usize::MAX {
        memory_budget.max_frames
    } else {
        frame_store.total_frames
    };

    // Check whether the timeline actually needs updating before taking a
    // mutable borrow — avoids triggering Bevy change-detection every frame
    // which would cause `sync_section_loads` to re-evaluate needlessly.
    let needs_total_sync = timeline.total_frames != frame_store.total_frames;

    let any_past_end = timeline
        .sections
        .iter()
        .any(|s| s.end > frame_store.total_frames);

    // Only auto-enforce budget when the budget itself has DECREASED or
    // sections extend past total_frames.  We must not re-enforce every
    // frame just because active_loaded > max_frames — the user may have
    // explicitly activated a section via the UI, and enforce_budget would
    // immediately undo that choice (furthest-from-playhead gets
    // deactivated).  The user-triggered paths (activate_section,
    // add_section, add_section_at) already call enforce_budget internally.
    let budget_decreased = max_frames < *last_max_frames;
    let needs_budget_enforce = budget_decreased || any_past_end;

    if !needs_total_sync && !needs_budget_enforce {
        *last_max_frames = max_frames;
        return; // Nothing to do — skip DerefMut to avoid change-detection churn.
    }

    // Now take the mutable borrow.
    let old_total = timeline.total_frames;
    timeline.total_frames = frame_store.total_frames;

    // When the sequence grows (watch mode / incremental load), auto-expand
    // any active section that was pinned to the old end so the new frames
    // become visible without manual slider adjustment.
    if frame_store.total_frames > old_total {
        let expanded = timeline.expand_trailing_sections(old_total);
        if expanded > 0 {
            debug!(
                "Auto-expanded {} trailing section(s): {} → {} total frames",
                expanded, old_total, frame_store.total_frames,
            );
        }
    }

    let old_sections = timeline.sections.clone();

    if needs_budget_enforce {
        timeline.enforce_budget(max_frames);
    }

    *last_max_frames = max_frames;

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
///
/// When multiple load sections exist the playhead skips over gaps between
/// sections — it jumps from the end of one section to the start of the
/// next.  If the playhead is past the last section and looping is enabled
/// it wraps to the first loaded frame.
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

        let next_candidate = timeline.playhead + 1;

        if next_candidate >= timeline.total_frames {
            // Past the end of the whole sequence
            if sm.loop_enabled {
                // Wrap to the first loaded frame (start of first section)
                timeline.playhead = timeline.next_loaded_frame(0).unwrap_or(0);
            } else {
                sm.is_playing = false;
                *accumulator = 0.0;
                break;
            }
        } else if timeline.is_frame_loaded(next_candidate) {
            // Next frame is inside a loaded section — simple advance
            timeline.playhead = next_candidate;
        } else {
            // Next frame is in a gap — skip to the start of the next section
            if let Some(next) = timeline.next_loaded_frame(next_candidate) {
                timeline.playhead = next;
            } else if sm.loop_enabled {
                // No section ahead — wrap around
                timeline.playhead = timeline.next_loaded_frame(0).unwrap_or(0);
            } else {
                sm.is_playing = false;
                *accumulator = 0.0;
                break;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Series ↔ global resource sync
// ---------------------------------------------------------------------------

/// Tracks which series was active last frame so we can detect switches.
#[derive(Default)]
struct SeriesSyncState {
    /// The active series ID on the previous frame.
    prev_active: Option<SeriesId>,
}

/// Bidirectional sync between [`SeriesManagerRes`] and the global
/// [`Timeline`] / [`SequenceManager`] resources.
///
/// **Active-series switch** (detected by comparing `prev_active`):
///   - Copies the new active series' `Timeline` into the global `Timeline`.
///   - Copies the new active series' `Sequence` into `SequenceManager`.
///   - Fires a [`LoadSequenceRequest`] so the loader picks up the new frames.
///
/// **Steady state** (same active series, no switch):
///   - Syncs the global `Timeline` (mutated by playback, keyboard, UI drag)
///     **back** into the active series' per-series timeline so the
///     `SeriesManager` stays up to date.
fn sync_series_manager(
    mut series_mgr: ResMut<SeriesManagerRes>,
    mut timeline: ResMut<Timeline>,
    mut sequence_manager: ResMut<SequenceManager>,
    mut load_events: MessageWriter<LoadSequenceRequest>,
    mut sync_state: Local<SeriesSyncState>,
    actions: Res<crate::app::input::ActionStateRes>,
) {
    // -- Keyboard-driven series switching (Ctrl+PageDown / Ctrl+PageUp) ------
    if actions.0.just_activated(Action::NextSeries) {
        series_mgr.cycle_next();
        debug!(
            "▶ NextSeries action — cycled to {:?}",
            series_mgr.active_id()
        );
    }
    if actions.0.just_activated(Action::PrevSeries) {
        series_mgr.cycle_prev();
        debug!(
            "◀ PrevSeries action — cycled to {:?}",
            series_mgr.active_id()
        );
    }

    let current_active = series_mgr.active_id();

    if current_active != sync_state.prev_active {
        // --- Active series changed -------------------------------------------

        // When the very first series is registered (None → Some), the legacy
        // discovery path has already fired a LoadSequenceRequest and populated
        // SequenceManager + Timeline.  Firing another LoadSequenceRequest here
        // would cause a double-load.  Skip the reload in that case — only
        // trigger it for genuine user-initiated switches (Some(a) → Some(b)).
        let is_initial_registration = sync_state.prev_active.is_none();

        if let Some(active) = series_mgr.active() {
            // On initial registration (None → Some), the legacy discovery
            // path already loaded the *first* sequence into SequenceManager
            // + Timeline.  If the restored active series IS that first
            // sequence we can skip the reload.  But if settings restoration
            // changed the active to a *different* series, we must reload.
            let legacy_matches_active = is_initial_registration
                && sequence_manager
                    .current_sequence
                    .as_ref()
                    .map(|seq| seq.base_dir == active.base_dir)
                    .unwrap_or(false);

            if legacy_matches_active {
                // Active series matches what the legacy path already loaded.
                // Just sync the restored timeline (playhead, sections) into
                // the global Timeline so the UI reflects saved state.
                info!(
                    "Initial series registration: {} '{}' (syncing timeline, skipping reload)",
                    active.id, active.label,
                );
                *timeline = active.timeline.clone();
            } else if is_initial_registration && sequence_manager.current_sequence.is_none() {
                // No legacy sequence loaded yet — skip reload entirely.
                info!(
                    "Initial series registration: {} '{}' (no legacy sequence, skipping reload)",
                    active.id, active.label,
                );
            } else if is_initial_registration {
                // Active series differs from what legacy path loaded — reload.
                info!(
                    "Initial series registration: {} '{}' (differs from legacy-loaded sequence, reloading)",
                    active.id,
                    active.label,
                );

                *timeline = active.timeline.clone();
                sequence_manager.load_sequence(active.sequence.clone());

                let frame_paths: Vec<_> = active
                    .sequence
                    .frames
                    .iter()
                    .map(|f| f.path.clone())
                    .collect();
                if !frame_paths.is_empty() {
                    info!(
                        "Loading {} frames for series '{}' ({})",
                        frame_paths.len(),
                        active.label,
                        active.id,
                    );
                    load_events.write(LoadSequenceRequest {
                        series_id: active.id,
                        frame_paths,
                    });
                }
            } else {
                info!(
                    "Series switch: {:?} → {} '{}'",
                    sync_state.prev_active, active.id, active.label,
                );

                // Push active series' timeline into the global resource.
                *timeline = active.timeline.clone();

                // Push sequence metadata into SequenceManager.
                sequence_manager.load_sequence(active.sequence.clone());

                // Trigger reload of the new series' frames.
                let frame_paths: Vec<_> = active
                    .sequence
                    .frames
                    .iter()
                    .map(|f| f.path.clone())
                    .collect();
                if !frame_paths.is_empty() {
                    info!(
                        "Loading {} frames for series '{}' ({})",
                        frame_paths.len(),
                        active.label,
                        active.id,
                    );
                    load_events.write(LoadSequenceRequest {
                        series_id: active.id,
                        frame_paths,
                    });
                }
            }
        }
        sync_state.prev_active = current_active;
    } else if let Some(id) = current_active {
        // --- Steady state: sync global Timeline back to active series --------
        // Only write if something actually changed (avoid needless clones).
        if let Some(active) = series_mgr.get(id) {
            let global_changed = active.timeline.playhead != timeline.playhead
                || active.timeline.sections != timeline.sections
                || active.timeline.total_frames != timeline.total_frames;

            if global_changed {
                if let Some(active_mut) = series_mgr.get_mut(id) {
                    active_mut.timeline = (*timeline).clone();
                }
            }
        }
    }
}

/// One-way sync: `SequenceManager` + `Timeline` → `UiState` for display.
fn sync_ui_from_sequence_manager(
    sm: Res<SequenceManager>,
    timeline: Res<Timeline>,
    mut ui_state: ResMut<UiState>,
    frame_store: Res<FrameStore>,
    series_mgr: Res<SeriesManagerRes>,
) {
    ui_state.playback.total_frames = timeline.total_frames;
    ui_state.playback.current_frame = timeline.playhead;
    ui_state.playback.is_playing = sm.is_playing;
    ui_state.playback.speed = sm.speed;
    ui_state.playback.loop_enabled = sm.loop_enabled;

    // Sync loading state — use total_loaded() (load-section target) as denominator
    ui_state
        .loading
        .update_from_frame_store(&frame_store, timeline.total_loaded());

    // Sync series state — UI reads from these instead of SeriesManagerRes directly
    ui_state.series = series_mgr.summary();
    ui_state.active_series_id = series_mgr.active_id();
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
        ls.set_start(20);
        assert_eq!(ls.start, 20);
        assert_eq!(ls.end, 100);
    }

    #[test]
    fn test_set_start_past_end() {
        let mut ls = LoadSection::new(10, 50);
        ls.set_start(80);
        // start clamped to end - 1 = 49
        assert_eq!(ls.start, 49);
    }

    #[test]
    fn test_set_start_free_sizing() {
        // No budget constraint — start can go as low as 0
        let mut ls = LoadSection::new(50, 200);
        ls.set_start(0);
        assert_eq!(ls.start, 0);
        assert_eq!(ls.end, 200);
    }

    #[test]
    fn test_set_end_normal() {
        let mut ls = LoadSection::new(10, 100);
        ls.set_end(200, 500);
        assert_eq!(ls.start, 10);
        assert_eq!(ls.end, 200);
    }

    #[test]
    fn test_set_end_past_total() {
        let mut ls = LoadSection::new(10, 100);
        ls.set_end(600, 500);
        assert_eq!(ls.end, 500);
    }

    #[test]
    fn test_set_end_before_start() {
        let mut ls = LoadSection::new(50, 100);
        ls.set_end(30, 500);
        // end clamped to max(30, start + 1) = 51
        assert_eq!(ls.end, 51);
    }

    #[test]
    fn test_set_end_free_sizing() {
        // No budget constraint — end can go up to total_frames
        let mut ls = LoadSection::new(10, 100);
        ls.set_end(400, 500);
        assert_eq!(ls.end, 400);
        assert_eq!(ls.start, 10);
    }

    // -----------------------------------------------------------------------
    // LoadSection::move_to
    // -----------------------------------------------------------------------

    #[test]
    fn test_move_to_normal() {
        let mut ls = LoadSection::new(0, 100);
        ls.move_to(50, 2133);
        assert_eq!(ls.start, 50);
        assert_eq!(ls.end, 150);
    }

    #[test]
    fn test_move_to_clamps_to_end() {
        let mut ls = LoadSection::new(0, 100);
        ls.move_to(2100, 2133);
        assert_eq!(ls.start, 2033);
        assert_eq!(ls.end, 2133);
    }

    #[test]
    fn test_move_to_preserves_size() {
        let mut ls = LoadSection::new(500, 776);
        let size = ls.frame_count();
        ls.move_to(200, 2133);
        assert_eq!(ls.frame_count(), size);
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
        // Section contains the playhead (frame 0), so it can't be deactivated.
        // It keeps its original size — budget is best-effort when the only
        // section holds the playhead.
        assert_eq!(tl.sections[0], LoadSection::new(0, 1000));
        assert!(tl.sections[0].active);
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
        // Playhead is 0, so section 0 contains it and stays active.
        // Section 1 is furthest from playhead → deactivated.
        assert_eq!(tl.sections.len(), 2);
        assert!(tl.sections[0].active);
        assert!(!tl.sections[1].active);
        // Active total = 500 (section 0 alone; can't deactivate playhead section).
        assert_eq!(tl.total_loaded(), 500);
    }

    #[test]
    fn test_enforce_budget_within_budget_untouched() {
        // Two sections that together are within budget should NOT be modified.
        let mut tl = Timeline::new(1000, 100);
        tl.sections = vec![LoadSection::new(0, 30), LoadSection::new(500, 570)];
        // Total = 100, budget = 100 → within budget.
        let before = tl.sections.clone();
        tl.enforce_budget(100);
        assert_eq!(
            tl.sections, before,
            "Sections within budget should not be touched"
        );
    }

    #[test]
    fn test_enforce_budget_asymmetric_proportional() {
        // One big section and one small section. Budget enforced via
        // deactivation, not resizing.
        let mut tl = Timeline::new(1000, 100);
        tl.sections = vec![LoadSection::new(0, 300), LoadSection::new(500, 600)];
        // Total = 400, budget = 200 → deactivate furthest from playhead (0).
        tl.enforce_budget(200);
        // Section 0 contains playhead → stays active.
        // Section 1 (center 550) is furthest → deactivated.
        assert!(tl.sections[0].active);
        assert!(!tl.sections[1].active);
        // Sections keep their original sizes — no resizing.
        assert_eq!(tl.sections[0].frame_count(), 300);
        assert_eq!(tl.sections[1].frame_count(), 100);
        // Active total = 300 (playhead section can't be deactivated).
        assert_eq!(tl.total_loaded(), 300);
    }

    #[test]
    fn test_enforce_budget_section_past_total_frames() {
        // Section extending past total_frames should be clamped even if
        // within frame-count budget.
        let mut tl = Timeline::new(100, 100);
        tl.sections = vec![LoadSection::new(80, 150)];
        tl.enforce_budget(200);
        assert!(
            tl.sections[0].end <= 100,
            "Section end {} should be <= total_frames 100",
            tl.sections[0].end,
        );
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
        // Section is always created; budget enforced by deactivating distant
        // sections rather than refusing creation.
        let idx = tl.add_section(70, 30);
        assert_eq!(idx, Some(1));
        // New section exists but was deactivated to stay within budget
        // (playhead=0 is in section 0, so section 1 is deactivated).
        assert!(tl.sections[0].active);
        assert!(!tl.sections[1].active);
    }

    #[test]
    fn test_add_section_tiny_budget() {
        let mut tl = Timeline::new(100, 10);
        // Section is created even when budget is tight — enforce_budget
        // deactivates it since playhead (0) is in section 0.
        let idx = tl.add_section(50, 10);
        assert_eq!(idx, Some(1));
        assert!(tl.sections[0].active);
        assert!(!tl.sections[1].active);
        // Second call at same center lands inside the new section → None.
        assert_eq!(tl.add_section(50, 11), None);
    }

    // ===================================================================
    // add_section_at — drag-to-paint gesture
    // ===================================================================

    #[test]
    fn test_add_section_at_basic() {
        let mut tl = Timeline::new(1000, 200);
        // Section 0: [0, 200). Budget=400 → remaining=200.
        let idx = tl.add_section_at(500, 400);
        assert!(idx.is_some(), "Should create section at frame 500");
        let idx = idx.unwrap();
        // New section should start at (or very near) 500
        assert_eq!(tl.sections[idx].start, 500);
        // Should be minimal (2 frames)
        assert_eq!(tl.sections[idx].frame_count(), 2);
    }

    #[test]
    fn test_add_section_at_no_budget() {
        let mut tl = Timeline::new(100, 30);
        // Section is always created; enforce_budget deactivates it since
        // playhead (0) is in section 0.
        let idx = tl.add_section_at(70, 30);
        assert_eq!(idx, Some(1));
        assert!(tl.sections[0].active);
        assert!(!tl.sections[1].active);
    }

    #[test]
    fn test_add_section_at_inside_existing() {
        let mut tl = Timeline::new(100, 100);
        // Section covers [0, 100). Can't place inside it.
        assert_eq!(tl.add_section_at(50, 200), None);
    }

    #[test]
    fn test_add_section_at_respects_gap() {
        let mut tl = Timeline::new(1000, 200);
        // Section 0: [0, 200). Try to place right at the boundary.
        let idx = tl.add_section_at(200, 400);
        assert!(idx.is_some());
        let idx = idx.unwrap();
        // Must leave a gap — start should be > 200
        assert!(
            tl.sections[idx].start > 200,
            "Should leave a gap after existing section, got start={}",
            tl.sections[idx].start,
        );
    }

    #[test]
    fn test_add_section_at_near_next_section() {
        let mut tl = Timeline::new(1000, 100);
        tl.sections = vec![LoadSection::new(0, 100), LoadSection::new(200, 300)];
        // Place between the two sections
        let idx = tl.add_section_at(150, 500);
        assert!(idx.is_some());
        let idx = idx.unwrap();
        // Must not overlap either neighbor
        assert!(tl.sections[idx].start >= tl.sections[0].end);
        assert!(tl.sections[idx].end <= tl.sections[2].start);
    }

    #[test]
    fn test_add_section_at_no_room_between_neighbors() {
        let mut tl = Timeline::new(1000, 100);
        // Two sections with only a tiny gap (3 frames, but min_gap eats most of it)
        tl.sections = vec![LoadSection::new(0, 100), LoadSection::new(103, 200)];
        // min_gap = (1000/200).clamp(1,20) = 5, so slot is 103-5=98..100+5=105
        // left_bound = 100+5=105, right_bound = 103-5=98 → left >= right → None
        assert_eq!(tl.add_section_at(101, 500), None);
    }

    // ===================================================================
    // add_section — centred placement
    // ===================================================================

    #[test]
    fn test_add_section_leaves_gap() {
        // When adding a section near an existing one, there must be a gap
        // between them so the UI edge handles don't overlap.
        let mut tl = Timeline::new(1000, 200);
        // Section 0 is [0, 200)
        let idx = tl.add_section(500, 400);
        assert!(idx.is_some(), "Should be able to add with remaining budget");
        assert_eq!(tl.sections.len(), 2);

        // The two sections must NOT be adjacent — there must be a gap.
        let gap = tl.sections[1].start.saturating_sub(tl.sections[0].end);
        assert!(
            gap >= 1,
            "Sections should have a gap between them, got gap={} sections={:?}",
            gap,
            tl.sections,
        );
    }

    #[test]
    fn test_add_section_no_overlap_with_existing() {
        let mut tl = Timeline::new(1000, 300);
        // Section 0 is [0, 300). Add two more.
        let _ = tl.add_section(600, 600);
        let _ = tl.add_section(900, 900);
        // No pair of sections should overlap
        for i in 1..tl.sections.len() {
            assert!(
                tl.sections[i].start >= tl.sections[i - 1].end,
                "Overlap between section {} ({:?}) and {} ({:?})",
                i - 1,
                tl.sections[i - 1],
                i,
                tl.sections[i],
            );
        }
    }

    #[test]
    fn test_add_section_inside_existing_returns_none() {
        let mut tl = Timeline::new(100, 100);
        // Section covers [0, 100) — entire timeline
        // Clicking inside should not add a section (is_frame_loaded would
        // catch this in the UI, but add_section should also handle it).
        assert_eq!(tl.add_section(50, 200), None);
    }

    #[test]
    fn test_add_section_caps_size() {
        // With a large remaining budget, the new section should NOT consume
        // all of it — it should be capped so the user has room to resize.
        let mut tl = Timeline::new(1000, 100);
        // Section 0: [0, 100). Budget=500 → remaining=400.
        let idx = tl.add_section(700, 500);
        assert!(idx.is_some());
        let new = &tl.sections[1];
        // New section should be significantly smaller than 400 (the remaining).
        assert!(
            new.frame_count() <= 200,
            "New section should be capped, not consume all remaining budget. Got {} frames",
            new.frame_count(),
        );
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

    // -- expand_trailing_sections -------------------------------------------

    #[test]
    fn test_expand_trailing_single_section_pinned() {
        // Section [0, 200) pinned to end, sequence grows to 210
        let mut tl = Timeline::new(200, 200);
        assert_eq!(tl.sections[0].end, 200);
        tl.total_frames = 210;
        let expanded = tl.expand_trailing_sections(200);
        assert_eq!(expanded, 1);
        assert_eq!(tl.sections[0].end, 210);
    }

    #[test]
    fn test_expand_trailing_section_not_pinned() {
        // Section [0, 100) in a 200-frame sequence — not pinned to end
        let mut tl = Timeline::new(200, 100);
        tl.total_frames = 210;
        let expanded = tl.expand_trailing_sections(200);
        assert_eq!(expanded, 0);
        assert_eq!(tl.sections[0].end, 100); // unchanged
    }

    #[test]
    fn test_expand_trailing_multi_section() {
        // Two sections: [0, 100) and [150, 200). Only the second is pinned.
        let mut tl = Timeline {
            total_frames: 220,
            sections: vec![LoadSection::new(0, 100), LoadSection::new(150, 200)],
            playhead: 0,
        };
        let expanded = tl.expand_trailing_sections(200);
        assert_eq!(expanded, 1);
        assert_eq!(tl.sections[0].end, 100); // unchanged
        assert_eq!(tl.sections[1].end, 220); // expanded
    }

    #[test]
    fn test_expand_trailing_inactive_section_ignored() {
        // Inactive section pinned to end should NOT expand
        let mut tl = Timeline {
            total_frames: 210,
            sections: vec![LoadSection {
                start: 0,
                end: 200,
                active: false,
            }],
            playhead: 0,
        };
        let expanded = tl.expand_trailing_sections(200);
        assert_eq!(expanded, 0);
        assert_eq!(tl.sections[0].end, 200); // unchanged
    }

    #[test]
    fn test_expand_trailing_no_growth() {
        // Sequence didn't grow — nothing should change
        let mut tl = Timeline::new(200, 200);
        tl.total_frames = 200;
        let expanded = tl.expand_trailing_sections(200);
        assert_eq!(expanded, 0);
    }

    #[test]
    fn test_expand_trailing_sequence_shrank() {
        // Sequence shrank — nothing should change
        let mut tl = Timeline::new(200, 200);
        tl.total_frames = 190;
        let expanded = tl.expand_trailing_sections(200);
        assert_eq!(expanded, 0);
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

    // ===================================================================
    // frames_to_load — budget-aware asset loading
    // ===================================================================

    #[test]
    fn test_frames_to_load_single_section() {
        let sections = vec![LoadSection::new(0, 98)];
        let indices = frames_to_load(&sections, 2133);
        assert_eq!(indices.len(), 98);
        assert_eq!(indices[0], 0);
        assert_eq!(indices[97], 97);
    }

    #[test]
    fn test_frames_to_load_respects_budget_not_total() {
        // This is the core regression: with 2133 total frames and a budget
        // that fits 98, we must only return 98 indices, NOT 2133.
        let mut timeline = Timeline::default();
        timeline.init_sequence(2133, 98);
        let indices = frames_to_load(&timeline.sections, timeline.total_frames);
        assert_eq!(
            indices.len(),
            98,
            "Should load only budget-limited frames, not all {}",
            timeline.total_frames
        );
    }

    #[test]
    fn test_frames_to_load_multi_section() {
        let sections = vec![
            LoadSection::new(0, 32),
            LoadSection::new(500, 532),
            LoadSection::new(1000, 1032),
        ];
        let indices = frames_to_load(&sections, 2133);
        assert_eq!(indices.len(), 96);
        assert_eq!(indices[0], 0);
        assert_eq!(indices[31], 31);
        assert_eq!(indices[32], 500);
        assert_eq!(indices[63], 531);
        assert_eq!(indices[64], 1000);
        assert_eq!(indices[95], 1031);
    }

    #[test]
    fn test_frames_to_load_clamps_to_total() {
        let sections = vec![LoadSection::new(2100, 2200)];
        let indices = frames_to_load(&sections, 2133);
        assert_eq!(indices.len(), 33); // 2100..2133
        assert_eq!(*indices.last().unwrap(), 2132);
    }

    #[test]
    fn test_frames_to_load_empty_sections() {
        let indices = frames_to_load(&[], 2133);
        assert!(indices.is_empty());
    }

    #[test]
    fn test_frames_to_load_zero_total() {
        let sections = vec![LoadSection::new(0, 100)];
        let indices = frames_to_load(&sections, 0);
        assert!(indices.is_empty());
    }

    // ── union_frames_to_load ──────────────────────────────────────────

    #[test]
    fn test_union_single_series() {
        let sections = vec![LoadSection::new(0, 10)];
        let result = union_frames_to_load(&[(&sections, 100)]);
        assert_eq!(result, (0..10).collect::<Vec<_>>());
    }

    #[test]
    fn test_union_two_series_overlapping() {
        // Series A: [0..100], Series B: [80..200]
        // Union should be [0..200] with no duplicates.
        let a = vec![LoadSection::new(0, 100)];
        let b = vec![LoadSection::new(80, 200)];
        let result = union_frames_to_load(&[(&a, 300), (&b, 300)]);
        assert_eq!(result.len(), 200);
        assert_eq!(result[0], 0);
        assert_eq!(*result.last().unwrap(), 199);
        // Verify no duplicates
        let mut deduped = result.clone();
        deduped.dedup();
        assert_eq!(result, deduped);
    }

    #[test]
    fn test_union_two_series_disjoint() {
        let a = vec![LoadSection::new(0, 50)];
        let b = vec![LoadSection::new(100, 150)];
        let result = union_frames_to_load(&[(&a, 200), (&b, 200)]);
        assert_eq!(result.len(), 100);
        assert_eq!(result[0], 0);
        assert_eq!(result[49], 49);
        assert_eq!(result[50], 100);
        assert_eq!(result[99], 149);
    }

    #[test]
    fn test_union_two_series_identical() {
        let a = vec![LoadSection::new(10, 20)];
        let b = vec![LoadSection::new(10, 20)];
        let result = union_frames_to_load(&[(&a, 100), (&b, 100)]);
        assert_eq!(result.len(), 10);
        assert_eq!(result, (10..20).collect::<Vec<_>>());
    }

    #[test]
    fn test_union_empty_sections() {
        let result = union_frames_to_load(&[]);
        assert!(result.is_empty());
    }

    #[test]
    fn test_union_one_series_empty_sections() {
        let a = vec![LoadSection::new(0, 50)];
        let b: Vec<LoadSection> = vec![];
        let result = union_frames_to_load(&[(&a, 100), (&b, 100)]);
        assert_eq!(result.len(), 50);
    }

    #[test]
    fn test_union_clamps_to_total_frames() {
        // Series A has total_frames=50 but section goes to 100
        let a = vec![LoadSection::new(0, 100)];
        let result = union_frames_to_load(&[(&a, 50)]);
        assert_eq!(result.len(), 50);
        assert_eq!(*result.last().unwrap(), 49);
    }

    #[test]
    fn test_union_different_total_frames() {
        // Series A: 100 frames, section [0..100]
        // Series B: 200 frames, section [50..200]
        // Union: [0..200] but A clamps to 100, B clamps to 200
        let a = vec![LoadSection::new(0, 100)];
        let b = vec![LoadSection::new(50, 200)];
        let result = union_frames_to_load(&[(&a, 100), (&b, 200)]);
        assert_eq!(result.len(), 200);
        assert_eq!(result[0], 0);
        assert_eq!(*result.last().unwrap(), 199);
    }

    #[test]
    fn test_union_inactive_sections_skipped() {
        let a = vec![LoadSection::new(0, 50)];
        let mut inactive = LoadSection::new(50, 100);
        inactive.active = false;
        let b = vec![inactive];
        let result = union_frames_to_load(&[(&a, 200), (&b, 200)]);
        assert_eq!(result.len(), 50);
    }

    #[test]
    fn test_union_multi_section_per_series() {
        let a = vec![LoadSection::new(0, 30), LoadSection::new(70, 100)];
        let b = vec![LoadSection::new(20, 80)];
        let result = union_frames_to_load(&[(&a, 200), (&b, 200)]);
        // Union: 0..100
        assert_eq!(result.len(), 100);
        assert_eq!(result[0], 0);
        assert_eq!(*result.last().unwrap(), 99);
    }
}
