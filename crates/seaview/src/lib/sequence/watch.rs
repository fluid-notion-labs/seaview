//! Watch mode for auto-detecting and loading new frames
//!
//! This module provides functionality for monitoring a directory for new mesh
//! files and incrementally loading them into the current sequence.

use bevy::prelude::*;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::Duration;

use super::{FrameInfo, Sequence, SequenceAssets};

/// Plugin for watch mode functionality
pub struct WatchPlugin;

impl Plugin for WatchPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WatchConfig>()
            .init_resource::<LoadingPhase>()
            .init_resource::<WatchState>()
            .add_systems(Update, (poll_for_new_frames, handle_keyboard_watch_toggle));
    }
}

/// Configuration for watch mode
#[derive(Resource)]
pub struct WatchConfig {
    /// Whether watch mode is enabled
    pub enabled: bool,
    /// Directory being watched
    pub directory: Option<PathBuf>,
    /// How often to poll for new files
    pub poll_interval: Duration,
    /// Minimum file age before considering it stable (avoids partial reads)
    pub min_file_age: Duration,
    /// Source orientation for coordinate transforms
    pub source_orientation: crate::lib::coordinates::SourceOrientation,
}

impl Default for WatchConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            directory: None,
            poll_interval: Duration::from_millis(500),
            min_file_age: Duration::from_secs(1),
            source_orientation: crate::lib::coordinates::SourceOrientation::default(),
        }
    }
}

/// State machine for the loading lifecycle
#[derive(Resource, Default)]
pub struct LoadingPhase {
    /// Current phase state
    pub state: LoadingPhaseState,
}

/// Phases of the loading lifecycle
#[derive(Default, Clone, Copy, PartialEq, Eq, Debug)]
pub enum LoadingPhaseState {
    /// No loading activity
    #[default]
    Idle,
    /// Initial directory scan on startup
    InitialDiscovery,
    /// Loading frames discovered at startup
    InitialLoading,
    /// Actively watching for new frames
    Watching,
    /// Loading newly discovered frames
    IncrementalLoad,
}

impl LoadingPhaseState {
    /// Human-readable description of the current state
    pub fn description(&self) -> &'static str {
        match self {
            Self::Idle => "Idle",
            Self::InitialDiscovery => "Discovering...",
            Self::InitialLoading => "Loading...",
            Self::Watching => "Watching",
            Self::IncrementalLoad => "Loading new frames...",
        }
    }

    /// Whether we're in a loading state
    pub fn is_loading(&self) -> bool {
        matches!(self, Self::InitialLoading | Self::IncrementalLoad)
    }

    /// Whether we're actively watching
    pub fn is_watching(&self) -> bool {
        matches!(self, Self::Watching | Self::IncrementalLoad)
    }
}

/// Internal state for tracking watched files
#[derive(Resource, Default)]
pub struct WatchState {
    /// Set of known frame paths (already loaded or queued)
    pub known_frames: HashSet<PathBuf>,
    /// Files pending stability check: path -> (size, first_seen_instant)
    pub pending_files: Vec<PendingFile>,
    /// Timer for polling
    pub poll_timer: f32,
    /// Sequence pattern used for discovery (cached from initial load)
    pub sequence_pattern: Option<String>,
    /// Sequence name (cached from initial load)
    pub sequence_name: Option<String>,
}

/// A file that's been discovered but not yet confirmed stable
#[derive(Clone, Debug)]
pub struct PendingFile {
    pub path: PathBuf,
    pub size: u64,
    pub first_seen: f32, // Time since startup when first seen
    pub frame_number: usize,
}

impl WatchState {
    /// Reset watch state for a new sequence
    pub fn reset(&mut self) {
        self.known_frames.clear();
        self.pending_files.clear();
        self.poll_timer = 0.0;
        self.sequence_pattern = None;
        self.sequence_name = None;
    }

    /// Initialize from an existing sequence
    pub fn init_from_sequence(&mut self, sequence: &Sequence) {
        self.known_frames.clear();
        for frame in &sequence.frames {
            self.known_frames.insert(frame.path.clone());
        }
        self.sequence_pattern = Some(sequence.pattern.clone());
        self.sequence_name = Some(sequence.name.clone());
    }

    /// Check if a path is already known
    pub fn is_known(&self, path: &PathBuf) -> bool {
        self.known_frames.contains(path)
    }

    /// Add a path to the known set
    pub fn mark_known(&mut self, path: PathBuf) {
        self.known_frames.insert(path);
    }
}

/// System that polls for new frames when watch mode is enabled
fn poll_for_new_frames(
    time: Res<Time>,
    watch_config: Res<WatchConfig>,
    mut watch_state: ResMut<WatchState>,
    mut loading_phase: ResMut<LoadingPhase>,
    _sequence_assets: Res<SequenceAssets>,
) {
    // Only poll when watching
    if !watch_config.enabled || loading_phase.state != LoadingPhaseState::Watching {
        return;
    }

    // Update poll timer
    watch_state.poll_timer += time.delta_secs();
    if watch_state.poll_timer < watch_config.poll_interval.as_secs_f32() {
        return;
    }
    watch_state.poll_timer = 0.0;

    // Get directory to scan
    let Some(directory) = &watch_config.directory else {
        return;
    };

    // Scan for new files
    let new_frames = scan_for_new_frames(
        directory,
        &watch_state,
        &watch_config,
        time.elapsed_secs(),
    );

    if !new_frames.is_empty() {
        debug!("Found {} new stable frames to load", new_frames.len());
        // TODO: Trigger incremental load
        // For now, just mark them as known
        for frame in new_frames {
            watch_state.mark_known(frame.path);
        }
        loading_phase.state = LoadingPhaseState::IncrementalLoad;
    }
}

/// Scan directory for new frames that aren't in our known set
fn scan_for_new_frames(
    directory: &PathBuf,
    watch_state: &WatchState,
    watch_config: &WatchConfig,
    current_time: f32,
) -> Vec<PendingFile> {
    let mut stable_frames = Vec::new();

    // Get the pattern regex for this sequence
    let patterns = super::discovery::SequencePatterns::default();

    if let Ok(entries) = std::fs::read_dir(directory) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            // Skip if already known
            if watch_state.is_known(&path) {
                continue;
            }

            // Try to match against sequence patterns
            if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
                if let Some((frame_number, _pattern_name)) = try_match_patterns(filename, &patterns)
                {
                    // Check file stability
                    if let Ok(metadata) = std::fs::metadata(&path) {
                        let size = metadata.len();

                        // Check if file is old enough
                        if let Ok(modified) = metadata.modified() {
                            if let Ok(age) = modified.elapsed() {
                                if age >= watch_config.min_file_age {
                                    stable_frames.push(PendingFile {
                                        path,
                                        size,
                                        first_seen: current_time,
                                        frame_number,
                                    });
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Sort by frame number
    stable_frames.sort_by_key(|f| f.frame_number);
    stable_frames
}

/// Try to match a filename against known sequence patterns
fn try_match_patterns(
    filename: &str,
    patterns: &super::discovery::SequencePatterns,
) -> Option<(usize, String)> {
    for pattern in patterns.iter() {
        if let Some(frame_num) = pattern.try_match(filename) {
            return Some((frame_num, pattern.name().to_string()));
        }
    }
    None
}

/// System that handles Ctrl+W to toggle watch mode
fn handle_keyboard_watch_toggle(
    input: Res<ButtonInput<KeyCode>>,
    mut watch_config: ResMut<WatchConfig>,
    mut loading_phase: ResMut<LoadingPhase>,
) {
    let ctrl = input.pressed(KeyCode::ControlLeft) || input.pressed(KeyCode::ControlRight);

    if ctrl && input.just_pressed(KeyCode::KeyW) {
        watch_config.enabled = !watch_config.enabled;

        if watch_config.enabled {
            debug!("Watch mode enabled");
            // Transition to watching if we're idle or done loading
            if loading_phase.state == LoadingPhaseState::Idle {
                loading_phase.state = LoadingPhaseState::Watching;
            }
        } else {
            debug!("Watch mode disabled");
            // Transition back to idle if we were watching
            if loading_phase.state == LoadingPhaseState::Watching {
                loading_phase.state = LoadingPhaseState::Idle;
            }
        }
    }
}

/// Extension trait for Sequence to support incremental frame addition
pub trait SequenceExt {
    /// Merge new frames into the sequence, maintaining sorted order by frame number
    fn merge_frames(&mut self, new_frames: Vec<FrameInfo>);
}

impl SequenceExt for Sequence {
    fn merge_frames(&mut self, new_frames: Vec<FrameInfo>) {
        for frame in new_frames {
            // Find insertion point to maintain sorted order
            let insert_pos = self
                .frames
                .binary_search_by_key(&frame.frame_number, |f| f.frame_number)
                .unwrap_or_else(|pos| pos);

            self.frames.insert(insert_pos, frame);
        }
    }
}

/// Extension trait for SequenceAssets to support incremental loading
pub trait SequenceAssetsExt {
    /// Extend the assets to accommodate new frames at specific positions
    fn extend_for_frames(&mut self, new_count: usize, insert_positions: &[usize]);
}

impl SequenceAssetsExt for SequenceAssets {
    fn extend_for_frames(&mut self, new_count: usize, _insert_positions: &[usize]) {
        // For now, just extend at the end
        // A more sophisticated implementation would insert at the correct positions
        self.total_frames += new_count;

        // Extend tracking vectors
        for _ in 0..new_count {
            self.loaded_frames.push(false);
            self.frame_entities.push(None);
        }
    }
}
