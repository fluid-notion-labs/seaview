//! Watch mode for auto-detecting and loading new frames
//!
//! This module provides functionality for monitoring a directory for new mesh
//! files and incrementally loading them into the current sequence.

use bevy::prelude::*;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{FrameInfo, Sequence, SequenceAssets, SequenceManager};

/// Plugin for watch mode functionality
pub struct WatchPlugin;

impl Plugin for WatchPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<WatchConfig>()
            .init_resource::<LoadingPhase>()
            .init_resource::<WatchState>()
            .add_message::<IncrementalLoadRequest>()
            .add_systems(
                Update,
                (
                    init_watch_state_from_sequence,
                    poll_for_new_frames,
                    handle_incremental_load,
                    update_loading_phase_on_load_complete,
                    handle_keyboard_watch_toggle,
                )
                    .chain(),
            );
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
    /// Number of frames being loaded in current incremental batch
    pub pending_frame_count: usize,
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
    /// Set of known frame numbers (already loaded or queued)
    pub known_frames: HashSet<usize>,
    /// Timer for polling
    pub poll_timer: f32,
    /// Sequence pattern used for discovery (cached from initial load)
    pub sequence_pattern: Option<String>,
    /// Sequence name (cached from initial load)
    pub sequence_name: Option<String>,
    /// Whether we've initialized from the current sequence
    pub initialized: bool,
    /// Previous loaded count to detect when loading completes
    pub prev_loaded_count: usize,
}

impl WatchState {
    /// Reset watch state for a new sequence
    pub fn reset(&mut self) {
        self.known_frames.clear();
        self.poll_timer = 0.0;
        self.sequence_pattern = None;
        self.sequence_name = None;
        self.initialized = false;
        self.prev_loaded_count = 0;
    }

    /// Initialize from an existing sequence
    pub fn init_from_sequence(&mut self, sequence: &Sequence) {
        self.known_frames.clear();
        for frame in &sequence.frames {
            self.known_frames.insert(frame.frame_number);
        }
        self.sequence_pattern = Some(sequence.pattern.clone());
        self.sequence_name = Some(sequence.name.clone());
        self.initialized = true;
    }

    /// Check if a frame number is already known
    pub fn is_known(&self, frame_number: usize) -> bool {
        self.known_frames.contains(&frame_number)
    }

    /// Add a frame number to the known set
    pub fn mark_known(&mut self, frame_number: usize) {
        self.known_frames.insert(frame_number);
    }
}

/// Event to request incremental loading of new frames
#[derive(Message)]
pub struct IncrementalLoadRequest {
    /// Paths to new frame files (in order by frame number)
    pub frame_paths: Vec<PathBuf>,
    /// Frame numbers corresponding to each path
    pub frame_numbers: Vec<usize>,
}

/// System that initializes watch state when a sequence is first loaded
fn init_watch_state_from_sequence(
    sequence_manager: Res<SequenceManager>,
    mut watch_state: ResMut<WatchState>,
    mut loading_phase: ResMut<LoadingPhase>,
    watch_config: Res<WatchConfig>,
    sequence_assets: Res<SequenceAssets>,
) {
    // Wait for initial load to complete before initializing watch state
    if !watch_state.initialized {
        if let Some(sequence) = sequence_manager.current_sequence() {
            // Only initialize once we have frames loaded
            if sequence_assets.loaded_count > 0 && !sequence_assets.loading {
                watch_state.init_from_sequence(sequence);
                debug!(
                    "Watch state initialized with {} known frames",
                    watch_state.known_frames.len()
                );

                // Transition to watching if enabled
                if watch_config.enabled {
                    loading_phase.state = LoadingPhaseState::Watching;
                    debug!("Transitioned to Watching state");
                }
            }
        }
    }
}

/// System that polls for new frames when watch mode is enabled
fn poll_for_new_frames(
    time: Res<Time>,
    watch_config: Res<WatchConfig>,
    mut watch_state: ResMut<WatchState>,
    mut loading_phase: ResMut<LoadingPhase>,
    mut incremental_load_events: MessageWriter<IncrementalLoadRequest>,
) {
    // Only poll when watching and initialized
    if !watch_config.enabled
        || loading_phase.state != LoadingPhaseState::Watching
        || !watch_state.initialized
    {
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
    let new_frames = scan_for_new_frames(directory, &watch_state, &watch_config);

    if !new_frames.is_empty() {
        let count = new_frames.len();
        debug!("Found {} new stable frames to load", count);

        // Collect paths and frame numbers
        let frame_paths: Vec<PathBuf> = new_frames.iter().map(|f| f.path.clone()).collect();
        let frame_numbers: Vec<usize> = new_frames.iter().map(|f| f.frame_number).collect();

        // Mark frames as known immediately to avoid re-detecting them
        for frame in &new_frames {
            watch_state.mark_known(frame.frame_number);
        }

        // Transition to incremental load state
        loading_phase.state = LoadingPhaseState::IncrementalLoad;
        loading_phase.pending_frame_count = count;

        // Emit incremental load request
        incremental_load_events.write(IncrementalLoadRequest {
            frame_paths,
            frame_numbers,
        });
    }
}

/// System that handles incremental load requests
fn handle_incremental_load(
    mut incremental_requests: MessageReader<IncrementalLoadRequest>,
    mut sequence_manager: ResMut<SequenceManager>,
    mut sequence_assets: ResMut<SequenceAssets>,
) {
    for request in incremental_requests.read() {
        debug!(
            "Processing incremental load of {} frames",
            request.frame_paths.len()
        );

        // Update sequence manager with new frames
        if let Some(sequence) = sequence_manager.current_sequence.as_mut() {
            for (path, frame_number) in request.frame_paths.iter().zip(request.frame_numbers.iter())
            {
                let frame_info = FrameInfo::new(path.clone(), *frame_number);

                // Insert in sorted order by frame number
                let insert_pos = sequence
                    .frames
                    .binary_search_by_key(frame_number, |f| f.frame_number)
                    .unwrap_or_else(|pos| pos);

                sequence.frames.insert(insert_pos, frame_info);
            }

            debug!(
                "Sequence now has {} frames after incremental update",
                sequence.frame_count()
            );
        }

        // Register new frames — store paths and pre-allocate slots.
        // Actual asset loading is handled by `sync_section_loads` which
        // only loads frames within the timeline's budget sections.
        for path in &request.frame_paths {
            sequence_assets.frame_paths.push(path.clone());
            sequence_assets.frame_handles.push(None);
            sequence_assets.loaded_frames.push(false);
            sequence_assets.frame_entities.push(None);
            sequence_assets.gltf_handles.push(None);
        }

        // Update total frames and mark as loading so sync_section_loads
        // picks up the new frames on the next tick.
        sequence_assets.total_frames += request.frame_paths.len();
        sequence_assets.loading = true;

        debug!(
            "Started incremental load of {} frames, total now {}",
            request.frame_paths.len(),
            sequence_assets.total_frames
        );
    }
}

/// System that transitions back to Watching state when incremental load completes
fn update_loading_phase_on_load_complete(
    sequence_assets: Res<SequenceAssets>,
    mut loading_phase: ResMut<LoadingPhase>,
    mut watch_state: ResMut<WatchState>,
    watch_config: Res<WatchConfig>,
) {
    // Check if we just finished an incremental load
    if loading_phase.state == LoadingPhaseState::IncrementalLoad {
        // Detect load completion by checking if loaded count increased and loading stopped
        if !sequence_assets.loading && sequence_assets.loaded_count > watch_state.prev_loaded_count
        {
            debug!(
                "Incremental load complete: {} frames now loaded",
                sequence_assets.loaded_count
            );

            // Transition back to watching if still enabled
            if watch_config.enabled {
                loading_phase.state = LoadingPhaseState::Watching;
            } else {
                loading_phase.state = LoadingPhaseState::Idle;
            }
            loading_phase.pending_frame_count = 0;
        }
    }

    // Also handle transition from InitialLoading to Watching
    if loading_phase.state == LoadingPhaseState::InitialLoading {
        if !sequence_assets.loading && sequence_assets.is_fully_loaded() {
            if watch_config.enabled {
                loading_phase.state = LoadingPhaseState::Watching;
                debug!("Initial load complete, transitioning to Watching");
            } else {
                loading_phase.state = LoadingPhaseState::Idle;
            }
        }
    }

    // Track loaded count for next frame
    watch_state.prev_loaded_count = sequence_assets.loaded_count;
}

/// A file that's been discovered and confirmed stable
#[derive(Clone, Debug)]
struct NewFrame {
    path: PathBuf,
    frame_number: usize,
}

/// Scan directory for new frames that aren't in our known set
fn scan_for_new_frames(
    directory: &Path,
    watch_state: &WatchState,
    watch_config: &WatchConfig,
) -> Vec<NewFrame> {
    let mut stable_frames = Vec::new();

    // Get the pattern regex for this sequence
    let patterns = super::discovery::SequencePatterns::default();

    if let Ok(entries) = std::fs::read_dir(directory) {
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }

            // Try to match against sequence patterns
            if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
                if let Some((frame_number, _pattern_name)) = try_match_patterns(filename, &patterns)
                {
                    // Skip if frame number already known
                    if watch_state.is_known(frame_number) {
                        continue;
                    }

                    // Check file stability (file age)
                    if let Ok(metadata) = std::fs::metadata(&path) {
                        if let Ok(modified) = metadata.modified() {
                            if let Ok(age) = modified.elapsed() {
                                if age >= watch_config.min_file_age {
                                    stable_frames.push(NewFrame { path, frame_number });
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
    watch_state: Res<WatchState>,
) {
    let ctrl = input.pressed(KeyCode::ControlLeft) || input.pressed(KeyCode::ControlRight);

    if ctrl && input.just_pressed(KeyCode::KeyW) {
        watch_config.enabled = !watch_config.enabled;

        if watch_config.enabled {
            debug!("Watch mode enabled via Ctrl+W");
            // Transition to watching if we're idle and initialized
            if loading_phase.state == LoadingPhaseState::Idle && watch_state.initialized {
                loading_phase.state = LoadingPhaseState::Watching;
            }
        } else {
            debug!("Watch mode disabled via Ctrl+W");
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
