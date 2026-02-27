//! Multi-series architecture: manage multiple mesh sequences with independent timelines.
//!
//! A **Series** is the atomic loadable unit — one simulation run at one
//! resolution.  Each series owns its own [`Sequence`] metadata (frame paths,
//! name, base dir) and its own [`Timeline`] (playhead, load sections,
//! total_frames).
//!
//! [`SeriesManager`] holds all loaded series and tracks which one is
//! **active** (visible in the viewport).  It is pure data — no Bevy
//! dependencies, no ECS queries — and lives in `lib/` so it can be tested
//! without a running `App`.
//!
//! ## Design
//!
//! - One series is active at a time.  The active series' `Timeline` is
//!   synced into the global `Timeline` Bevy resource by an app-layer system.
//! - Other series can exist (for pre-loading, budget accounting, etc.) but
//!   are not rendered.
//! - The global `SequenceManager`, `Timeline`, and `FrameStore` Bevy
//!   resources remain for now — they represent the *active* series.  Phase 2
//!   will make `FrameStore` per-series.
//!
//! ## Related
//!
//! - `seaview-6pd` — multi-series architecture epic
//! - `seaview-5zf` — GPU visibility/spawn/despawn strategy (blocked on this)
//! - `seaview-w5t` — memory budget UI per-series allocation

use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

use crate::lib::environment_objects::Transform3D;
use crate::lib::sequence::{LoadSection, Sequence, Timeline};

// ---------------------------------------------------------------------------
// SeriesId — lightweight, copyable identifier
// ---------------------------------------------------------------------------

/// Unique identifier for a loaded series.
///
/// Internally a monotonically increasing `u64`.  Cheap to copy, hash, and
/// compare.  Not persisted across sessions — series are identified by name
/// in `seaview.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct SeriesId(u64);

impl SeriesId {
    /// The id value as a raw `u64` (for display / serialization).
    pub fn raw(self) -> u64 {
        self.0
    }

    /// Create a `SeriesId` from a raw `u64`.
    ///
    /// Intended for tests and settings restoration where IDs are
    /// reconstructed from persisted data.  Normal runtime code should
    /// let [`SeriesManager::add`] allocate IDs.
    pub fn from_raw(val: u64) -> Self {
        Self(val)
    }
}

impl fmt::Display for SeriesId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "series-{}", self.0)
    }
}

// ---------------------------------------------------------------------------
// SeriesState — one loaded series: metadata + timeline
// ---------------------------------------------------------------------------

/// The complete state for a single loaded series.
///
/// Owns the [`Sequence`] (frame paths, name, source orientation) and a
/// per-series [`Timeline`] (playhead, load sections, total_frames).
#[derive(Debug, Clone)]
pub struct SeriesState {
    /// Unique identifier (assigned by [`SeriesManager`]).
    pub id: SeriesId,

    /// The underlying sequence metadata (frame paths, name, base_dir, etc.).
    pub sequence: Sequence,

    /// Per-series timeline (playhead, load sections, total_frames).
    pub timeline: Timeline,

    /// Human-readable label.  Defaults to `sequence.name` but can be
    /// overridden by the user.
    pub label: String,

    /// Base directory for this series (convenience — mirrors `sequence.base_dir`).
    pub base_dir: PathBuf,

    /// Per-series transform (position, rotation, scale).
    /// Applied to all spawned frame entities for this series.
    /// Defaults to identity.
    pub transform: Transform3D,
}

impl SeriesState {
    /// Create a new series from a discovered [`Sequence`].
    ///
    /// The timeline is initialised with a single load section covering
    /// `[0, min(total_frames, max_frames))`.
    pub fn new(id: SeriesId, sequence: Sequence, max_frames: usize) -> Self {
        let total_frames = sequence.frame_count();
        let label = sequence.name.clone();
        let base_dir = sequence.base_dir.clone();

        let mut timeline = Timeline::default();
        timeline.init_sequence(total_frames, max_frames);

        Self {
            id,
            sequence,
            timeline,
            label,
            base_dir,
            transform: Transform3D::default(),
        }
    }

    /// Total number of frames in this series.
    pub fn frame_count(&self) -> usize {
        self.sequence.frame_count()
    }

    /// Whether this series has any frames.
    pub fn is_empty(&self) -> bool {
        self.sequence.frame_count() == 0
    }

    /// Get the current playhead position for this series.
    pub fn playhead(&self) -> usize {
        self.timeline.playhead
    }

    /// Get the total number of loaded frames across all active sections.
    pub fn total_loaded(&self) -> usize {
        self.timeline.total_loaded()
    }

    /// Apply saved load sections (from settings), replacing the default.
    ///
    /// Sections are clamped to valid bounds and empty sections are discarded.
    pub fn apply_saved_sections(&mut self, sections: Vec<LoadSection>, max_frames: usize) {
        let total = self.timeline.total_frames;
        let mut clamped: Vec<LoadSection> = sections
            .into_iter()
            .map(|mut s| {
                s.clamp(total, max_frames);
                s
            })
            .filter(|s| s.frame_count() > 0)
            .collect();

        if clamped.is_empty() {
            return; // keep existing sections
        }

        clamped.sort_by_key(|s| s.start);
        self.timeline.sections = clamped;
    }
}

// ---------------------------------------------------------------------------
// SeriesManager — owns all series, tracks active
// ---------------------------------------------------------------------------

/// Manages multiple loaded series and tracks which one is active.
///
/// Pure data — no Bevy resources, no ECS.  App-layer systems sync the
/// active series' timeline into the global `Timeline` resource.
#[derive(Debug, Clone)]
pub struct SeriesManager {
    /// All loaded series, keyed by their unique ID.
    series: HashMap<SeriesId, SeriesState>,

    /// Insertion-order list of series IDs (for stable UI ordering).
    order: Vec<SeriesId>,

    /// The currently active (visible) series, if any.
    active: Option<SeriesId>,

    /// Monotonic counter for generating unique [`SeriesId`]s.
    next_id: u64,
}

impl Default for SeriesManager {
    fn default() -> Self {
        Self::new()
    }
}

impl SeriesManager {
    /// Create an empty series manager.
    pub fn new() -> Self {
        Self {
            series: HashMap::new(),
            order: Vec::new(),
            active: None,
            next_id: 0,
        }
    }

    // -- ID generation ------------------------------------------------------

    fn alloc_id(&mut self) -> SeriesId {
        let id = SeriesId(self.next_id);
        self.next_id += 1;
        id
    }

    // -- Queries ------------------------------------------------------------

    /// Number of loaded series.
    pub fn len(&self) -> usize {
        self.series.len()
    }

    /// Whether there are no loaded series.
    pub fn is_empty(&self) -> bool {
        self.series.is_empty()
    }

    /// The active series ID, if any.
    pub fn active_id(&self) -> Option<SeriesId> {
        self.active
    }

    /// Get the active series state (immutable).
    pub fn active(&self) -> Option<&SeriesState> {
        self.active.and_then(|id| self.series.get(&id))
    }

    /// Get the active series state (mutable).
    pub fn active_mut(&mut self) -> Option<&mut SeriesState> {
        self.active.and_then(|id| self.series.get_mut(&id))
    }

    /// Get a series by ID (immutable).
    pub fn get(&self, id: SeriesId) -> Option<&SeriesState> {
        self.series.get(&id)
    }

    /// Get a series by ID (mutable).
    pub fn get_mut(&mut self, id: SeriesId) -> Option<&mut SeriesState> {
        self.series.get_mut(&id)
    }

    /// Find a series by name (first match).
    pub fn find_by_name(&self, name: &str) -> Option<&SeriesState> {
        self.series.values().find(|s| s.label == name)
    }

    /// Find a series by base directory (first match).
    pub fn find_by_dir(&self, dir: &std::path::Path) -> Option<&SeriesState> {
        self.series.values().find(|s| s.base_dir == dir)
    }

    /// Iterate over all series in insertion order.
    pub fn iter(&self) -> impl Iterator<Item = &SeriesState> {
        self.order.iter().filter_map(|id| self.series.get(id))
    }

    /// Collect all series (mutable) in insertion order.
    ///
    /// Returns a `Vec` of mutable references rather than an iterator because
    /// the borrow checker cannot prove that `filter_map` over a `HashMap`
    /// won't alias.  Callers can iterate the returned `Vec` normally.
    pub fn iter_mut(&mut self) -> Vec<&mut SeriesState> {
        let order = &self.order;
        // SAFETY-note: this is safe because each ID appears at most once in
        // `order` (enforced by add/remove), so we never alias.  We use
        // `unsafe` to work around the HashMap borrow-checker limitation.
        //
        // Alternative: use an IndexMap.  For now the series count is tiny
        // (single digits) so a Vec collect is fine.
        let mut result = Vec::with_capacity(order.len());
        for id in order {
            if let Some(state) = self.series.get_mut(id) {
                // Extend the lifetime — each id is unique so no aliasing.
                let ptr: *mut SeriesState = state;
                result.push(unsafe { &mut *ptr });
            }
        }
        result
    }

    /// Get all series IDs in insertion order.
    pub fn ids(&self) -> &[SeriesId] {
        &self.order
    }

    // -- Mutation ------------------------------------------------------------

    /// Add a series from a discovered [`Sequence`].
    ///
    /// Returns the assigned [`SeriesId`].  If this is the first series, it
    /// automatically becomes active.
    pub fn add(&mut self, sequence: Sequence, max_frames: usize) -> SeriesId {
        let id = self.alloc_id();
        let state = SeriesState::new(id, sequence, max_frames);
        self.series.insert(id, state);
        self.order.push(id);

        // First series auto-activates
        if self.active.is_none() {
            self.active = Some(id);
        }

        id
    }

    /// Add a series with pre-configured load sections (from settings).
    ///
    /// Returns the assigned [`SeriesId`].
    pub fn add_with_sections(
        &mut self,
        sequence: Sequence,
        max_frames: usize,
        sections: Vec<LoadSection>,
    ) -> SeriesId {
        let id = self.add(sequence, max_frames);
        if let Some(state) = self.series.get_mut(&id) {
            state.apply_saved_sections(sections, max_frames);
        }
        id
    }

    /// Remove a series by ID.
    ///
    /// If the removed series was active, the next series in order becomes
    /// active (or `None` if no series remain).
    ///
    /// Returns the removed [`SeriesState`] if it existed.
    pub fn remove(&mut self, id: SeriesId) -> Option<SeriesState> {
        let state = self.series.remove(&id)?;
        self.order.retain(|&oid| oid != id);

        if self.active == Some(id) {
            self.active = self.order.first().copied();
        }

        Some(state)
    }

    /// Switch the active series.
    ///
    /// Returns `true` if the active series changed (i.e. `id` exists and
    /// was not already active).
    pub fn set_active(&mut self, id: SeriesId) -> bool {
        if !self.series.contains_key(&id) {
            return false;
        }
        if self.active == Some(id) {
            return false;
        }
        self.active = Some(id);
        true
    }

    /// Cycle to the next series in order.  Wraps around.
    ///
    /// Returns the newly active [`SeriesId`], or `None` if empty.
    pub fn cycle_next(&mut self) -> Option<SeriesId> {
        if self.order.is_empty() {
            return None;
        }
        let current_idx = self
            .active
            .and_then(|id| self.order.iter().position(|&oid| oid == id))
            .unwrap_or(0);
        let next_idx = (current_idx + 1) % self.order.len();
        let next_id = self.order[next_idx];
        self.active = Some(next_id);
        Some(next_id)
    }

    /// Cycle to the previous series in order.  Wraps around.
    ///
    /// Returns the newly active [`SeriesId`], or `None` if empty.
    pub fn cycle_prev(&mut self) -> Option<SeriesId> {
        if self.order.is_empty() {
            return None;
        }
        let current_idx = self
            .active
            .and_then(|id| self.order.iter().position(|&oid| oid == id))
            .unwrap_or(0);
        let prev_idx = if current_idx == 0 {
            self.order.len() - 1
        } else {
            current_idx - 1
        };
        let prev_id = self.order[prev_idx];
        self.active = Some(prev_id);
        Some(prev_id)
    }

    /// Rename a series.
    pub fn rename(&mut self, id: SeriesId, new_label: impl Into<String>) -> bool {
        if let Some(state) = self.series.get_mut(&id) {
            state.label = new_label.into();
            true
        } else {
            false
        }
    }

    /// Total loaded frames across *all* series (for global budget display).
    pub fn total_loaded_all(&self) -> usize {
        self.series.values().map(|s| s.total_loaded()).sum()
    }

    /// Collect a snapshot of series info for UI display.
    pub fn summary(&self) -> Vec<SeriesSummary> {
        self.order
            .iter()
            .filter_map(|id| {
                let s = self.series.get(id)?;
                Some(SeriesSummary {
                    id: s.id,
                    label: s.label.clone(),
                    total_frames: s.frame_count(),
                    loaded_frames: s.total_loaded(),
                    playhead: s.playhead(),
                    is_active: self.active == Some(*id),
                })
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// SavedSeriesEntry — settings restoration input
// ---------------------------------------------------------------------------

/// Input struct for restoring series state from saved settings.
///
/// This is a plain-data struct with no Bevy dependencies, keeping the
/// restoration logic testable in `lib/`.  Callers (e.g. the discovery
/// system) convert from [`SeriesSettings`] into this before calling
/// [`SeriesManager::restore_from_saved`].
#[derive(Debug, Clone, Default)]
pub struct SavedSeriesEntry {
    /// Human-readable label (matched against `SeriesState::label` or
    /// `Sequence::name`).
    pub name: String,
    /// Base directory path (matched against `SeriesState::base_dir`).
    pub path: Option<PathBuf>,
    /// Whether this was the active series when the session was saved.
    pub active: bool,
    /// Saved playhead position.
    pub playhead: usize,
    /// Saved per-series load sections.
    pub load_sections: Vec<LoadSection>,
    /// Per-series transform (position, rotation, scale).
    pub transform: Transform3D,
}

impl SeriesManager {
    /// Restore per-series state from saved settings entries.
    ///
    /// For each saved entry, finds a matching series (by base directory
    /// first, then by name) and applies:
    /// - Label (rename)
    /// - Load sections (via [`SeriesState::apply_saved_sections`])
    /// - Playhead position
    /// - Active flag (the last entry with `active = true` wins)
    ///
    /// Returns the number of entries that were successfully matched.
    pub fn restore_from_saved(&mut self, saved: &[SavedSeriesEntry], max_frames: usize) -> usize {
        let mut matched = 0usize;
        let mut active_id: Option<SeriesId> = None;

        for entry in saved {
            // Try to match by path first (most reliable), then by name.
            let found_id = entry
                .path
                .as_deref()
                .and_then(|p| self.find_by_dir(p).map(|s| s.id))
                .or_else(|| {
                    if !entry.name.is_empty() {
                        self.find_by_name(&entry.name).map(|s| s.id)
                    } else {
                        None
                    }
                });

            let Some(id) = found_id else {
                continue;
            };

            let Some(state) = self.get_mut(id) else {
                continue;
            };

            // Apply label
            if !entry.name.is_empty() {
                state.label = entry.name.clone();
            }

            // Apply saved load sections
            if !entry.load_sections.is_empty() {
                state.apply_saved_sections(entry.load_sections.clone(), max_frames);
            }

            // Apply transform
            state.transform = entry.transform.clone();

            // Apply playhead (clamped to valid range)
            if entry.playhead > 0 {
                let max_playhead = state.timeline.total_frames.saturating_sub(1);
                state.timeline.playhead = entry.playhead.min(max_playhead);
            }

            if entry.active {
                active_id = Some(id);
            }

            matched += 1;
        }

        // Set the active series from saved state (if found).
        if let Some(id) = active_id {
            self.set_active(id);
        }

        matched
    }
}

// ---------------------------------------------------------------------------
// SeriesSummary — lightweight snapshot for UI
// ---------------------------------------------------------------------------

/// Lightweight read-only snapshot of a series' state for UI rendering.
#[derive(Debug, Clone)]
pub struct SeriesSummary {
    pub id: SeriesId,
    pub label: String,
    pub total_frames: usize,
    pub loaded_frames: usize,
    pub playhead: usize,
    pub is_active: bool,
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lib::coordinates::SourceOrientation;
    use std::path::Path;

    /// Helper: create a dummy sequence with `n` frames in a specific directory.
    fn dummy_sequence_in_dir(name: &str, n: usize, base_dir: &str) -> Sequence {
        use crate::lib::sequence::FrameInfo;
        let base = PathBuf::from(base_dir);
        let frames: Vec<FrameInfo> = (0..n)
            .map(|i| {
                let path = base.join(format!("{name}_{i:04}.obj"));
                FrameInfo {
                    filename: path.file_name().unwrap().to_string_lossy().into_owned(),
                    path,
                    frame_number: i,
                    file_size: 1024,
                }
            })
            .collect();
        Sequence {
            name: name.to_string(),
            frames,
            base_dir: base,
            source_orientation: SourceOrientation::default(),
            pattern: format!("{name}_*.obj"),
        }
    }

    fn dummy_sequence(name: &str, n: usize) -> Sequence {
        use crate::lib::sequence::FrameInfo;
        let mut seq = Sequence::new(
            name.to_string(),
            PathBuf::from(format!("/tmp/{}", name)),
            "frame_*.stl".to_string(),
            SourceOrientation::default(),
        );
        for i in 0..n {
            let path = PathBuf::from(format!("/tmp/{}/frame_{:04}.stl", name, i));
            seq.frames.push(FrameInfo {
                path,
                frame_number: i,
                filename: format!("frame_{:04}.stl", i),
                file_size: 1024,
            });
        }
        seq
    }

    // -- SeriesId -----------------------------------------------------------

    #[test]
    fn series_id_display() {
        let id = SeriesId(42);
        assert_eq!(format!("{}", id), "series-42");
    }

    #[test]
    fn series_id_equality() {
        let a = SeriesId(1);
        let b = SeriesId(1);
        let c = SeriesId(2);
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    // -- SeriesState --------------------------------------------------------

    #[test]
    fn series_state_new() {
        let seq = dummy_sequence("splash", 100);
        let state = SeriesState::new(SeriesId(0), seq, 50);

        assert_eq!(state.id, SeriesId(0));
        assert_eq!(state.label, "splash");
        assert_eq!(state.frame_count(), 100);
        assert!(!state.is_empty());
        assert_eq!(state.playhead(), 0);
        assert_eq!(state.timeline.total_frames, 100);
        assert_eq!(state.timeline.sections.len(), 1);
        assert_eq!(state.timeline.sections[0].start, 0);
        assert_eq!(state.timeline.sections[0].end, 50);
    }

    #[test]
    fn series_state_new_budget_exceeds_total() {
        let seq = dummy_sequence("small", 30);
        let state = SeriesState::new(SeriesId(0), seq, 1000);

        assert_eq!(state.timeline.sections[0].end, 30);
    }

    #[test]
    fn series_state_empty_sequence() {
        let seq = dummy_sequence("empty", 0);
        let state = SeriesState::new(SeriesId(0), seq, 100);

        assert!(state.is_empty());
        assert_eq!(state.frame_count(), 0);
    }

    #[test]
    fn series_state_apply_saved_sections() {
        let seq = dummy_sequence("splash", 200);
        let mut state = SeriesState::new(SeriesId(0), seq, 200);

        let sections = vec![LoadSection::new(10, 50), LoadSection::new(100, 150)];
        state.apply_saved_sections(sections, 200);

        assert_eq!(state.timeline.sections.len(), 2);
        assert_eq!(state.timeline.sections[0].start, 10);
        assert_eq!(state.timeline.sections[0].end, 50);
        assert_eq!(state.timeline.sections[1].start, 100);
        assert_eq!(state.timeline.sections[1].end, 150);
    }

    #[test]
    fn series_state_apply_saved_sections_empty_keeps_existing() {
        let seq = dummy_sequence("splash", 100);
        let mut state = SeriesState::new(SeriesId(0), seq, 50);

        let original_sections = state.timeline.sections.clone();
        state.apply_saved_sections(vec![], 50);

        assert_eq!(state.timeline.sections, original_sections);
    }

    #[test]
    fn series_state_apply_saved_sections_out_of_range_filtered() {
        let seq = dummy_sequence("splash", 100);
        let mut state = SeriesState::new(SeriesId(0), seq, 100);

        // Section entirely past total_frames — after clamp, start == end → filtered
        let sections = vec![LoadSection::new(200, 300)];
        let original = state.timeline.sections.clone();
        state.apply_saved_sections(sections, 100);

        // Should keep existing because all saved sections were out of range
        assert_eq!(state.timeline.sections, original);
    }

    #[test]
    fn series_state_apply_saved_sections_sorted() {
        let seq = dummy_sequence("splash", 200);
        let mut state = SeriesState::new(SeriesId(0), seq, 200);

        // Provide sections out of order
        let sections = vec![LoadSection::new(100, 150), LoadSection::new(10, 50)];
        state.apply_saved_sections(sections, 200);

        assert_eq!(state.timeline.sections[0].start, 10);
        assert_eq!(state.timeline.sections[1].start, 100);
    }

    // -- SeriesManager: basics ----------------------------------------------

    #[test]
    fn manager_starts_empty() {
        let mgr = SeriesManager::new();
        assert!(mgr.is_empty());
        assert_eq!(mgr.len(), 0);
        assert!(mgr.active_id().is_none());
        assert!(mgr.active().is_none());
    }

    #[test]
    fn manager_default_is_empty() {
        let mgr = SeriesManager::default();
        assert!(mgr.is_empty());
    }

    #[test]
    fn manager_add_first_series_becomes_active() {
        let mut mgr = SeriesManager::new();
        let seq = dummy_sequence("splash", 100);
        let id = mgr.add(seq, 50);

        assert_eq!(mgr.len(), 1);
        assert_eq!(mgr.active_id(), Some(id));

        let active = mgr.active().unwrap();
        assert_eq!(active.label, "splash");
        assert_eq!(active.frame_count(), 100);
    }

    #[test]
    fn manager_add_second_does_not_change_active() {
        let mut mgr = SeriesManager::new();
        let id1 = mgr.add(dummy_sequence("first", 100), 50);
        let _id2 = mgr.add(dummy_sequence("second", 200), 100);

        assert_eq!(mgr.len(), 2);
        assert_eq!(mgr.active_id(), Some(id1));
    }

    #[test]
    fn manager_ids_in_insertion_order() {
        let mut mgr = SeriesManager::new();
        let id1 = mgr.add(dummy_sequence("a", 10), 10);
        let id2 = mgr.add(dummy_sequence("b", 20), 20);
        let id3 = mgr.add(dummy_sequence("c", 30), 30);

        assert_eq!(mgr.ids(), &[id1, id2, id3]);
    }

    #[test]
    fn manager_iter_in_insertion_order() {
        let mut mgr = SeriesManager::new();
        mgr.add(dummy_sequence("a", 10), 10);
        mgr.add(dummy_sequence("b", 20), 20);
        mgr.add(dummy_sequence("c", 30), 30);

        let labels: Vec<&str> = mgr.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, vec!["a", "b", "c"]);
    }

    // -- SeriesManager: get/find --------------------------------------------

    #[test]
    fn manager_get_by_id() {
        let mut mgr = SeriesManager::new();
        let id = mgr.add(dummy_sequence("splash", 100), 50);

        assert!(mgr.get(id).is_some());
        assert_eq!(mgr.get(id).unwrap().label, "splash");

        let bogus = SeriesId(999);
        assert!(mgr.get(bogus).is_none());
    }

    #[test]
    fn manager_get_mut_by_id() {
        let mut mgr = SeriesManager::new();
        let id = mgr.add(dummy_sequence("splash", 100), 50);

        mgr.get_mut(id).unwrap().label = "renamed".to_string();
        assert_eq!(mgr.get(id).unwrap().label, "renamed");
    }

    #[test]
    fn manager_find_by_name() {
        let mut mgr = SeriesManager::new();
        mgr.add(dummy_sequence("alpha", 10), 10);
        mgr.add(dummy_sequence("beta", 20), 20);

        assert!(mgr.find_by_name("beta").is_some());
        assert_eq!(mgr.find_by_name("beta").unwrap().frame_count(), 20);
        assert!(mgr.find_by_name("gamma").is_none());
    }

    #[test]
    fn manager_find_by_dir() {
        let mut mgr = SeriesManager::new();
        mgr.add(dummy_sequence("alpha", 10), 10);

        let dir = PathBuf::from("/tmp/alpha");
        assert!(mgr.find_by_dir(&dir).is_some());
        assert!(mgr.find_by_dir(std::path::Path::new("/tmp/nope")).is_none());
    }

    // -- SeriesManager: set_active ------------------------------------------

    #[test]
    fn manager_set_active() {
        let mut mgr = SeriesManager::new();
        let id1 = mgr.add(dummy_sequence("a", 10), 10);
        let id2 = mgr.add(dummy_sequence("b", 20), 20);

        assert_eq!(mgr.active_id(), Some(id1));

        let changed = mgr.set_active(id2);
        assert!(changed);
        assert_eq!(mgr.active_id(), Some(id2));
    }

    #[test]
    fn manager_set_active_same_returns_false() {
        let mut mgr = SeriesManager::new();
        let id = mgr.add(dummy_sequence("a", 10), 10);

        let changed = mgr.set_active(id);
        assert!(!changed);
    }

    #[test]
    fn manager_set_active_nonexistent_returns_false() {
        let mut mgr = SeriesManager::new();
        mgr.add(dummy_sequence("a", 10), 10);

        let changed = mgr.set_active(SeriesId(999));
        assert!(!changed);
    }

    // -- SeriesManager: remove ----------------------------------------------

    #[test]
    fn manager_remove() {
        let mut mgr = SeriesManager::new();
        let id1 = mgr.add(dummy_sequence("a", 10), 10);
        let id2 = mgr.add(dummy_sequence("b", 20), 20);

        let removed = mgr.remove(id1);
        assert!(removed.is_some());
        assert_eq!(removed.unwrap().label, "a");
        assert_eq!(mgr.len(), 1);

        // Active should have switched to id2
        assert_eq!(mgr.active_id(), Some(id2));
    }

    #[test]
    fn manager_remove_last_clears_active() {
        let mut mgr = SeriesManager::new();
        let id = mgr.add(dummy_sequence("only", 10), 10);

        mgr.remove(id);
        assert!(mgr.is_empty());
        assert!(mgr.active_id().is_none());
    }

    #[test]
    fn manager_remove_inactive_keeps_active() {
        let mut mgr = SeriesManager::new();
        let id1 = mgr.add(dummy_sequence("a", 10), 10);
        let id2 = mgr.add(dummy_sequence("b", 20), 20);

        mgr.remove(id2);
        assert_eq!(mgr.active_id(), Some(id1));
    }

    #[test]
    fn manager_remove_nonexistent() {
        let mut mgr = SeriesManager::new();
        mgr.add(dummy_sequence("a", 10), 10);

        assert!(mgr.remove(SeriesId(999)).is_none());
    }

    // -- SeriesManager: cycle -----------------------------------------------

    #[test]
    fn manager_cycle_next() {
        let mut mgr = SeriesManager::new();
        let id1 = mgr.add(dummy_sequence("a", 10), 10);
        let id2 = mgr.add(dummy_sequence("b", 20), 20);
        let id3 = mgr.add(dummy_sequence("c", 30), 30);

        assert_eq!(mgr.active_id(), Some(id1));

        assert_eq!(mgr.cycle_next(), Some(id2));
        assert_eq!(mgr.active_id(), Some(id2));

        assert_eq!(mgr.cycle_next(), Some(id3));
        assert_eq!(mgr.active_id(), Some(id3));

        // Wrap around
        assert_eq!(mgr.cycle_next(), Some(id1));
        assert_eq!(mgr.active_id(), Some(id1));
    }

    #[test]
    fn manager_cycle_prev() {
        let mut mgr = SeriesManager::new();
        let id1 = mgr.add(dummy_sequence("a", 10), 10);
        let id2 = mgr.add(dummy_sequence("b", 20), 20);
        let id3 = mgr.add(dummy_sequence("c", 30), 30);

        assert_eq!(mgr.active_id(), Some(id1));

        // Wrap around backward
        assert_eq!(mgr.cycle_prev(), Some(id3));
        assert_eq!(mgr.active_id(), Some(id3));

        assert_eq!(mgr.cycle_prev(), Some(id2));
        assert_eq!(mgr.active_id(), Some(id2));

        assert_eq!(mgr.cycle_prev(), Some(id1));
    }

    #[test]
    fn manager_cycle_empty() {
        let mut mgr = SeriesManager::new();
        assert_eq!(mgr.cycle_next(), None);
        assert_eq!(mgr.cycle_prev(), None);
    }

    #[test]
    fn manager_cycle_single() {
        let mut mgr = SeriesManager::new();
        let id = mgr.add(dummy_sequence("only", 10), 10);

        assert_eq!(mgr.cycle_next(), Some(id));
        assert_eq!(mgr.cycle_prev(), Some(id));
    }

    // -- SeriesManager: rename ----------------------------------------------

    #[test]
    fn manager_rename() {
        let mut mgr = SeriesManager::new();
        let id = mgr.add(dummy_sequence("old_name", 10), 10);

        assert!(mgr.rename(id, "new_name"));
        assert_eq!(mgr.get(id).unwrap().label, "new_name");
    }

    #[test]
    fn manager_rename_nonexistent() {
        let mut mgr = SeriesManager::new();
        assert!(!mgr.rename(SeriesId(999), "whatever"));
    }

    // -- SeriesManager: add_with_sections -----------------------------------

    #[test]
    fn manager_add_with_sections() {
        let mut mgr = SeriesManager::new();
        let seq = dummy_sequence("splash", 200);
        let sections = vec![LoadSection::new(10, 50), LoadSection::new(100, 150)];

        let id = mgr.add_with_sections(seq, 200, sections);
        let state = mgr.get(id).unwrap();

        assert_eq!(state.timeline.sections.len(), 2);
        assert_eq!(state.timeline.sections[0].start, 10);
        assert_eq!(state.timeline.sections[1].start, 100);
    }

    // -- SeriesManager: aggregate queries -----------------------------------

    #[test]
    fn manager_total_loaded_all() {
        let mut mgr = SeriesManager::new();
        mgr.add(dummy_sequence("a", 100), 30);
        mgr.add(dummy_sequence("b", 200), 50);

        // a: section [0, 30) = 30 loaded, b: section [0, 50) = 50 loaded
        assert_eq!(mgr.total_loaded_all(), 80);
    }

    #[test]
    fn manager_summary() {
        let mut mgr = SeriesManager::new();
        let id1 = mgr.add(dummy_sequence("alpha", 100), 40);
        let id2 = mgr.add(dummy_sequence("beta", 200), 60);

        let summaries = mgr.summary();
        assert_eq!(summaries.len(), 2);

        assert_eq!(summaries[0].id, id1);
        assert_eq!(summaries[0].label, "alpha");
        assert_eq!(summaries[0].total_frames, 100);
        assert_eq!(summaries[0].loaded_frames, 40);
        assert!(summaries[0].is_active);

        assert_eq!(summaries[1].id, id2);
        assert_eq!(summaries[1].label, "beta");
        assert_eq!(summaries[1].total_frames, 200);
        assert_eq!(summaries[1].loaded_frames, 60);
        assert!(!summaries[1].is_active);
    }

    // -- SeriesManager: active_mut ------------------------------------------

    #[test]
    fn manager_active_mut() {
        let mut mgr = SeriesManager::new();
        let id = mgr.add(dummy_sequence("test", 100), 50);

        // Mutate the active series' timeline
        let active = mgr.active_mut().unwrap();
        active.timeline.playhead = 25;

        assert_eq!(mgr.get(id).unwrap().playhead(), 25);
    }

    // -- SeriesManager: iter_mut --------------------------------------------

    #[test]
    fn manager_iter_mut() {
        let mut mgr = SeriesManager::new();
        mgr.add(dummy_sequence("a", 10), 10);
        mgr.add(dummy_sequence("b", 20), 20);

        for state in mgr.iter_mut() {
            state.label = format!("{}_modified", state.label);
        }

        let labels: Vec<&str> = mgr.iter().map(|s| s.label.as_str()).collect();
        assert_eq!(labels, vec!["a_modified", "b_modified"]);
    }

    // -- ID uniqueness / monotonic ------------------------------------------

    #[test]
    fn manager_ids_monotonic() {
        let mut mgr = SeriesManager::new();
        let id1 = mgr.add(dummy_sequence("a", 10), 10);
        let id2 = mgr.add(dummy_sequence("b", 10), 10);
        let id3 = mgr.add(dummy_sequence("c", 10), 10);

        assert!(id1.raw() < id2.raw());
        assert!(id2.raw() < id3.raw());
    }

    #[test]
    fn manager_ids_unique_after_remove() {
        let mut mgr = SeriesManager::new();
        let id1 = mgr.add(dummy_sequence("a", 10), 10);
        mgr.remove(id1);
        let id2 = mgr.add(dummy_sequence("b", 10), 10);

        // The removed ID is never reused
        assert_ne!(id1, id2);
        assert!(id2.raw() > id1.raw());
    }

    // -----------------------------------------------------------------------
    // restore_from_saved tests
    // -----------------------------------------------------------------------

    #[test]
    fn restore_matches_by_path() {
        let mut mgr = SeriesManager::new();
        let seq_a = dummy_sequence_in_dir("alpha", 100, "/data/alpha");
        let seq_b = dummy_sequence_in_dir("beta", 200, "/data/beta");
        mgr.add(seq_a, 100);
        mgr.add(seq_b, 200);

        let saved = vec![
            SavedSeriesEntry {
                name: "Alpha Renamed".into(),
                path: Some(PathBuf::from("/data/alpha")),
                active: false,
                playhead: 42,
                load_sections: vec![LoadSection::new(10, 50)],
                ..Default::default()
            },
            SavedSeriesEntry {
                name: "Beta Renamed".into(),
                path: Some(PathBuf::from("/data/beta")),
                active: true,
                playhead: 99,
                load_sections: vec![LoadSection::new(0, 80)],
                ..Default::default()
            },
        ];

        let matched = mgr.restore_from_saved(&saved, 200);
        assert_eq!(matched, 2);

        // Check alpha
        let alpha = mgr.find_by_dir(Path::new("/data/alpha")).unwrap();
        assert_eq!(alpha.label, "Alpha Renamed");
        assert_eq!(alpha.timeline.playhead, 42);
        assert_eq!(alpha.timeline.sections.len(), 1);
        assert_eq!(alpha.timeline.sections[0].start, 10);
        assert_eq!(alpha.timeline.sections[0].end, 50);

        // Check beta is active
        let beta = mgr.find_by_dir(Path::new("/data/beta")).unwrap();
        assert_eq!(beta.label, "Beta Renamed");
        assert_eq!(beta.timeline.playhead, 99);
        assert_eq!(mgr.active_id(), Some(beta.id));
    }

    #[test]
    fn restore_matches_by_name_when_no_path() {
        let mut mgr = SeriesManager::new();
        let seq = dummy_sequence("splash_v3", 50);
        mgr.add(seq, 50);

        let saved = vec![SavedSeriesEntry {
            name: "splash_v3".into(),
            path: None,
            active: true,
            playhead: 25,
            load_sections: vec![],
            ..Default::default()
        }];

        let matched = mgr.restore_from_saved(&saved, 50);
        assert_eq!(matched, 1);
        let found = mgr.find_by_name("splash_v3").unwrap();
        assert_eq!(found.timeline.playhead, 25);
    }

    #[test]
    fn restore_skips_unmatched_entries() {
        let mut mgr = SeriesManager::new();
        let seq = dummy_sequence_in_dir("alpha", 100, "/data/alpha");
        mgr.add(seq, 100);

        let saved = vec![
            SavedSeriesEntry {
                name: "alpha".into(),
                path: Some(PathBuf::from("/data/alpha")),
                active: false,
                playhead: 10,
                load_sections: vec![],
                ..Default::default()
            },
            SavedSeriesEntry {
                name: "nonexistent".into(),
                path: Some(PathBuf::from("/data/gone")),
                active: true,
                playhead: 0,
                load_sections: vec![],
                ..Default::default()
            },
        ];

        let matched = mgr.restore_from_saved(&saved, 100);
        assert_eq!(matched, 1);
        // Active should NOT have changed because the "active" entry didn't match
        let alpha = mgr.find_by_name("alpha").unwrap();
        assert_eq!(alpha.timeline.playhead, 10);
    }

    #[test]
    fn restore_clamps_playhead_to_bounds() {
        let mut mgr = SeriesManager::new();
        let seq = dummy_sequence_in_dir("tiny", 10, "/data/tiny");
        mgr.add(seq, 10);

        let saved = vec![SavedSeriesEntry {
            name: "tiny".into(),
            path: Some(PathBuf::from("/data/tiny")),
            active: true,
            playhead: 9999, // way out of range
            load_sections: vec![],
            ..Default::default()
        }];

        let matched = mgr.restore_from_saved(&saved, 10);
        assert_eq!(matched, 1);
        let found = mgr.find_by_name("tiny").unwrap();
        assert_eq!(found.timeline.playhead, 9); // clamped to total_frames - 1
    }

    #[test]
    fn restore_empty_saved_is_noop() {
        let mut mgr = SeriesManager::new();
        let seq = dummy_sequence("alpha", 50);
        let id = mgr.add(seq, 50);

        let matched = mgr.restore_from_saved(&[], 50);
        assert_eq!(matched, 0);
        // Active should still be the first series
        assert_eq!(mgr.active_id(), Some(id));
    }

    #[test]
    fn restore_active_last_wins() {
        let mut mgr = SeriesManager::new();
        let seq_a = dummy_sequence_in_dir("a", 100, "/data/a");
        let seq_b = dummy_sequence_in_dir("b", 100, "/data/b");
        mgr.add(seq_a, 100);
        let id_b = mgr.add(seq_b, 100);

        // Both marked active — last one wins
        let saved = vec![
            SavedSeriesEntry {
                name: "a".into(),
                path: Some(PathBuf::from("/data/a")),
                active: true,
                playhead: 0,
                load_sections: vec![],
                ..Default::default()
            },
            SavedSeriesEntry {
                name: "b".into(),
                path: Some(PathBuf::from("/data/b")),
                active: true,
                playhead: 0,
                load_sections: vec![],
                ..Default::default()
            },
        ];

        mgr.restore_from_saved(&saved, 100);
        assert_eq!(mgr.active_id(), Some(id_b));
    }

    #[test]
    fn restore_preserves_sections_when_saved_empty() {
        let mut mgr = SeriesManager::new();
        let seq = dummy_sequence_in_dir("alpha", 100, "/data/alpha");
        mgr.add(seq, 100);

        // The default section should exist from add()
        let original_sections = mgr
            .find_by_dir(Path::new("/data/alpha"))
            .unwrap()
            .timeline
            .sections
            .clone();
        assert!(!original_sections.is_empty());

        let saved = vec![SavedSeriesEntry {
            name: "alpha".into(),
            path: Some(PathBuf::from("/data/alpha")),
            active: false,
            playhead: 0,
            load_sections: vec![], // empty — should keep existing
            ..Default::default()
        }];

        mgr.restore_from_saved(&saved, 100);
        let after = mgr.find_by_dir(Path::new("/data/alpha")).unwrap();
        assert_eq!(after.timeline.sections, original_sections);
    }

    #[test]
    fn restore_zero_playhead_not_applied() {
        // playhead=0 is the default, so we skip applying it to avoid
        // overwriting a previously-computed default playhead.
        let mut mgr = SeriesManager::new();
        let seq = dummy_sequence_in_dir("alpha", 100, "/data/alpha");
        mgr.add(seq, 100);

        // Manually set playhead to something nonzero
        mgr.find_by_dir(Path::new("/data/alpha"))
            .map(|s| s.id)
            .and_then(|id| mgr.get_mut(id))
            .unwrap()
            .timeline
            .playhead = 55;

        let saved = vec![SavedSeriesEntry {
            name: "alpha".into(),
            path: Some(PathBuf::from("/data/alpha")),
            active: false,
            playhead: 0, // zero — should NOT overwrite
            load_sections: vec![],
            ..Default::default()
        }];

        mgr.restore_from_saved(&saved, 100);
        let after = mgr.find_by_dir(Path::new("/data/alpha")).unwrap();
        assert_eq!(after.timeline.playhead, 55);
    }

    #[test]
    fn restore_applies_transform() {
        use crate::lib::environment_objects::Transform3D;

        let mut mgr = SeriesManager::new();
        let seq = dummy_sequence_in_dir("splash", 100, "/data/splash");
        mgr.add(seq, 100);

        // Verify default is identity
        let before = mgr.find_by_dir(Path::new("/data/splash")).unwrap();
        assert!(before.transform.is_identity());

        let saved = vec![SavedSeriesEntry {
            name: "splash".into(),
            path: Some(PathBuf::from("/data/splash")),
            active: true,
            playhead: 0,
            load_sections: vec![],
            transform: Transform3D::default()
                .with_position(5.0, 10.0, -3.0)
                .with_rotation_degrees(0.0, 90.0, 0.0)
                .with_scale(2.0, 2.0, 2.0),
        }];

        let matched = mgr.restore_from_saved(&saved, 100);
        assert_eq!(matched, 1);

        let after = mgr.find_by_dir(Path::new("/data/splash")).unwrap();
        assert_eq!(after.transform.position, [5.0, 10.0, -3.0]);
        assert_eq!(after.transform.rotation_degrees, [0.0, 90.0, 0.0]);
        assert_eq!(after.transform.scale, [2.0, 2.0, 2.0]);
    }

    #[test]
    fn restore_identity_transform_stays_identity() {
        let mut mgr = SeriesManager::new();
        let seq = dummy_sequence_in_dir("alpha", 50, "/data/alpha");
        mgr.add(seq, 50);

        let saved = vec![SavedSeriesEntry {
            name: "alpha".into(),
            path: Some(PathBuf::from("/data/alpha")),
            active: true,
            playhead: 10,
            load_sections: vec![],
            ..Default::default()
        }];

        mgr.restore_from_saved(&saved, 50);
        let after = mgr.find_by_dir(Path::new("/data/alpha")).unwrap();
        assert!(after.transform.is_identity());
    }
}
