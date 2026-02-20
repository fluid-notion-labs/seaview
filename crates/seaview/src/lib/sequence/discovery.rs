//! Sequence discovery module for finding mesh file sequences
//!
//! Directory scanning runs on Bevy's [`IoTaskPool`] so the main thread never
//! blocks on filesystem I/O.  The two-system split works as follows:
//!
//! 1. **`start_async_discovery`** — picks up [`DiscoverSequenceRequest`]
//!    components, spawns an I/O task, and replaces the component with a
//!    [`PendingDiscovery`] that holds the in-flight [`Task`].
//! 2. **`poll_async_discovery`** — each frame, checks whether any pending
//!    tasks have completed.  When one finishes it fires the appropriate
//!    events and kicks off asset loading.

use super::{loader::LoadSequenceRequest, FrameInfo, Sequence, SequenceEvent, SequenceManager};
use crate::lib::memory_budget::MemoryBudget;
use super::SeriesManagerRes;
use bevy::prelude::*;
use bevy::tasks::{block_on, IoTaskPool, Task};
use regex::Regex;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Plugin
// ---------------------------------------------------------------------------

/// Plugin for sequence discovery functionality
pub struct SequenceDiscoveryPlugin;

impl Plugin for SequenceDiscoveryPlugin {
    fn build(&self, app: &mut App) {
        app.add_message::<AllSequencesDiscovered>()
            .add_systems(
                Update,
                (
                    start_async_discovery,
                    poll_async_discovery,
                    register_discovered_series,
                )
                    .chain(),
            );
    }
}

/// Event fired when async discovery completes, carrying **all** discovered
/// sequences (not just the first).
///
/// Downstream systems can read this to populate [`SeriesManager`] or present
/// a selection UI.
#[derive(Message, Debug, Clone)]
pub struct AllSequencesDiscovered {
    /// All sequences found in the scanned directory.
    pub sequences: Vec<Sequence>,
    /// Source orientation requested for these sequences.
    pub source_orientation: crate::lib::coordinates::SourceOrientation,
}

// ---------------------------------------------------------------------------
// Components / resources
// ---------------------------------------------------------------------------

/// Component that requests sequence discovery in a directory.
///
/// Attach this to an entity and the discovery systems will pick it up,
/// start an async scan, and eventually fire [`SequenceEvent`]s and a
/// [`LoadSequenceRequest`].
#[derive(Component)]
pub struct DiscoverSequenceRequest {
    pub directory: PathBuf,
    pub recursive: bool,
    pub source_orientation: crate::lib::coordinates::SourceOrientation,
}

/// Component attached while an async discovery task is in flight.
///
/// Replaces [`DiscoverSequenceRequest`] for the duration of the scan so the
/// start system doesn't re-trigger on the same entity.
#[derive(Component)]
struct PendingDiscovery {
    task: Task<Result<Vec<Sequence>, String>>,
}

// ---------------------------------------------------------------------------
// Patterns
// ---------------------------------------------------------------------------

/// Supported mesh file extensions
const SUPPORTED_EXTENSIONS: &[&str] = &["stl", "obj", "glb", "gltf"];

/// Check if a filename ends with a supported mesh extension
fn has_supported_extension(filename: &str) -> bool {
    let lower = filename.to_lowercase();
    SUPPORTED_EXTENSIONS
        .iter()
        .any(|ext| lower.ends_with(&format!(".{}", ext)))
}

/// Resource for managing sequence discovery patterns
#[derive(Resource)]
pub struct SequencePatterns {
    /// List of regex patterns to match sequence files
    patterns: Vec<SequencePattern>,
}

impl SequencePatterns {
    /// Iterate over all patterns
    pub fn iter(&self) -> impl Iterator<Item = &SequencePattern> {
        self.patterns.iter()
    }
}

impl Default for SequencePatterns {
    fn default() -> Self {
        Self {
            patterns: vec![
                // Generic numbered patterns that work regardless of extension
                // Pattern 1: name-NNN (with optional separator)
                SequencePattern::new("numbered", r"^(.+?)[\._-](\d{3,})", 2),
                // Pattern 2: nameNNN (no separator)
                SequencePattern::new("numbered_nosep", r"^([a-zA-Z_]+)(\d{3,})", 2),
                // Pattern 3: named variants (timestep, step, time, frame)
                SequencePattern::new("timestep", r"^(.+?)[\._-]?[Tt](\d+)", 2),
                SequencePattern::new("step", r"^(.+?)[\._-]?[Ss]tep[\._-]?(\d+)", 2),
                SequencePattern::new("time", r"^(.+?)[\._-]?[Tt]ime[\._-]?(\d+)", 2),
                SequencePattern::new("frame", r"^(.+?)[\._-]?[Ff]rame[\._-]?(\d+)", 2),
            ],
        }
    }
}

/// A pattern for matching sequence files
pub struct SequencePattern {
    name: String,
    regex: Regex,
    frame_group: usize,
}

impl SequencePattern {
    fn new(name: &str, pattern: &str, frame_group: usize) -> Self {
        Self {
            name: name.to_string(),
            regex: Regex::new(pattern).expect("Invalid regex pattern"),
            frame_group,
        }
    }

    /// Get the pattern name
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Try to match a filename and extract the frame number
    pub fn try_match(&self, filename: &str) -> Option<usize> {
        self.regex.captures(filename).and_then(|captures| {
            captures
                .get(self.frame_group)
                .and_then(|m| m.as_str().parse::<usize>().ok())
        })
    }
}

// ---------------------------------------------------------------------------
// Systems
// ---------------------------------------------------------------------------

/// System that picks up [`DiscoverSequenceRequest`] components and kicks off
/// an async filesystem scan on the I/O task pool.
///
/// The request component is removed and replaced with [`PendingDiscovery`]
/// which holds the in-flight task handle.
fn start_async_discovery(
    mut commands: Commands,
    query: Query<(Entity, &DiscoverSequenceRequest)>,
    mut events: MessageWriter<SequenceEvent>,
) {
    let pool = IoTaskPool::get();

    for (entity, request) in query.iter() {
        let directory = request.directory.clone();
        let recursive = request.recursive;
        let source_orientation = request.source_orientation;

        debug!("Starting async discovery in: {:?}", directory);
        events.write(SequenceEvent::DiscoveryStarted(directory.clone()));

        // Capture values for the async closure.
        let dir = directory.clone();
        let task = pool.spawn(async move {
            // Everything inside here runs OFF the main thread.
            discover_sequences_blocking(&dir, recursive, source_orientation)
                .map_err(|e| format!("{e}"))
        });

        // Swap the request component for a pending handle.
        commands
            .entity(entity)
            .remove::<DiscoverSequenceRequest>()
            .insert(PendingDiscovery { task });
    }
}

/// System that polls in-flight discovery tasks each frame and, when one
/// completes, fires events and triggers asset loading.
/// System that polls in-flight discovery tasks each frame and, when one
/// completes, fires events and kicks off asset loading.
///
/// **Multi-series change:** emits [`AllSequencesDiscovered`] with the full
/// list of found sequences.  For backward compatibility the *first*
/// sequence is still loaded immediately into `SequenceManager` +
/// `LoadSequenceRequest`.  The [`register_discovered_series`] system
/// picks up `AllSequencesDiscovered` to populate [`SeriesManager`].
fn poll_async_discovery(
    mut commands: Commands,
    mut pending_query: Query<(Entity, &mut PendingDiscovery)>,
    mut sequence_manager: ResMut<SequenceManager>,
    mut events: MessageWriter<SequenceEvent>,
    mut load_events: MessageWriter<LoadSequenceRequest>,
    mut all_discovered: MessageWriter<AllSequencesDiscovered>,
) {
    for (entity, mut pending) in pending_query.iter_mut() {
        if !pending.task.is_finished() {
            continue;
        }

        // Task is done — retrieve the result.  Because `is_finished()` was
        // true this `block_on` returns immediately without actually blocking.
        let result = block_on(&mut pending.task);

        // Clean up the component regardless of outcome.
        commands.entity(entity).remove::<PendingDiscovery>();

        match result {
            Ok(sequences) => {
                let count = sequences.len();
                info!(
                    "Async discovery found {} sequence(s): [{}]",
                    count,
                    sequences
                        .iter()
                        .map(|s| format!("'{}' ({} frames)", s.name, s.frame_count()))
                        .collect::<Vec<_>>()
                        .join(", "),
                );

                if sequences.is_empty() {
                    events.write(SequenceEvent::Error("No sequences found".to_string()));
                    continue;
                }

                // Emit the full list for SeriesManager / multi-series UI.
                // We need to infer source_orientation from the first seq —
                // it was set during construction.
                let source_orientation = sequences[0].source_orientation;
                all_discovered.write(AllSequencesDiscovered {
                    sequences: sequences.clone(),
                    source_orientation,
                });

                // Backward compat: load the first sequence immediately
                // into the legacy single-sequence path.
                let first = sequences.into_iter().next().unwrap();
                let frame_count = first.frame_count();
                let sequence_name = first.name.clone();

                let frame_paths: Vec<_> =
                    first.frames.iter().map(|f| f.path.clone()).collect();
                if !frame_paths.is_empty() {
                    debug!(
                        "Triggering load of {} mesh frames for sequence '{}'",
                        frame_paths.len(),
                        sequence_name,
                    );
                    load_events.write(LoadSequenceRequest { frame_paths });
                }

                sequence_manager.load_sequence(first);
                events.write(SequenceEvent::SequenceLoaded(sequence_name));
                events.write(SequenceEvent::DiscoveryCompleted(frame_count));
            }
            Err(e) => {
                error!("Async discovery failed: {}", e);
                events.write(SequenceEvent::Error(format!("Discovery failed: {e}")));
            }
        }
    }
}

/// System that picks up [`AllSequencesDiscovered`] events and registers
/// every discovered sequence into [`SeriesManager`].
///
/// The first sequence in each event becomes the active series (matching
/// the legacy behavior of loading the first discovered sequence).
fn register_discovered_series(
    mut discovered: MessageReader<AllSequencesDiscovered>,
    mut series_manager: ResMut<SeriesManagerRes>,
    memory_budget: Res<MemoryBudget>,
) {
    let mgr = &mut *series_manager;

    for event in discovered.read() {
        let max_frames = if memory_budget.max_frames < usize::MAX {
            memory_budget.max_frames
        } else {
            // No budget known yet — use total frames as ceiling.
            event
                .sequences
                .first()
                .map(|s| s.frame_count())
                .unwrap_or(usize::MAX)
        };

        for sequence in &event.sequences {
            let id = mgr.add(sequence.clone(), max_frames);
            info!(
                "Registered series '{}' as {} ({} frames, max_budget={})",
                sequence.name,
                id,
                sequence.frame_count(),
                max_frames,
            );
        }

        info!(
            "SeriesManager now has {} series, active: {:?}",
            mgr.len(),
            mgr.active_id(),
        );
    }
}

// ---------------------------------------------------------------------------
// Pure discovery logic (runs off-thread)
// ---------------------------------------------------------------------------

/// Blocking discovery entry-point.  Called from an I/O task — never from a
/// Bevy system directly.
fn discover_sequences_blocking(
    directory: &Path,
    recursive: bool,
    source_orientation: crate::lib::coordinates::SourceOrientation,
) -> Result<Vec<Sequence>, std::io::Error> {
    let patterns = SequencePatterns::default();
    discover_sequences(directory, recursive, &patterns, source_orientation)
}

/// Discover sequences in a directory.
///
/// This is a pure function with no Bevy dependencies — it only does filesystem
/// I/O and regex matching.  Safe to call from any thread.
pub fn discover_sequences(
    directory: &Path,
    recursive: bool,
    patterns: &SequencePatterns,
    source_orientation: crate::lib::coordinates::SourceOrientation,
) -> Result<Vec<Sequence>, std::io::Error> {
    let mut sequences = Vec::new();
    let mut file_groups: HashMap<(String, String), Vec<(PathBuf, usize)>> = HashMap::new();

    // Scan directory for matching files
    scan_directory(directory, recursive, patterns, &mut file_groups)?;

    // Convert file groups into sequences
    for ((pattern_name, base_name), mut files) in file_groups {
        if files.len() < 2 {
            // Skip single files - not a sequence
            continue;
        }

        // Sort files by frame number
        files.sort_by_key(|(_, frame_num)| *frame_num);

        let mut sequence = Sequence::new(
            base_name.clone(),
            directory.to_path_buf(),
            pattern_name.clone(),
            source_orientation,
        );

        for (path, frame_number) in files {
            sequence.add_frame(FrameInfo::new(path, frame_number));
        }

        sequences.push(sequence);
    }

    // Sort sequences by name
    sequences.sort_by(|a, b| a.name.cmp(&b.name));

    Ok(sequences)
}

/// Recursively scan a directory for sequence files
fn scan_directory(
    directory: &Path,
    recursive: bool,
    patterns: &SequencePatterns,
    file_groups: &mut HashMap<(String, String), Vec<(PathBuf, usize)>>,
) -> Result<(), std::io::Error> {
    let entries = fs::read_dir(directory)?;

    for entry in entries {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() && recursive {
            scan_directory(&path, recursive, patterns, file_groups)?;
        } else if path.is_file() {
            if let Some(filename) = path.file_name().and_then(|f| f.to_str()) {
                // First check if file has a supported extension
                if !has_supported_extension(filename) {
                    continue;
                }

                // Try each pattern to extract base name and frame number
                for pattern in &patterns.patterns {
                    if let Some(captures) = pattern.regex.captures(filename) {
                        // Extract base name and frame number
                        if let (Some(base_match), Some(frame_match)) =
                            (captures.get(1), captures.get(pattern.frame_group))
                        {
                            let base_name = base_match.as_str().to_string();
                            if let Ok(frame_number) = frame_match.as_str().parse::<usize>() {
                                let key = (pattern.name.clone(), base_name);
                                file_groups
                                    .entry(key)
                                    .or_default()
                                    .push((path.clone(), frame_number));
                                break; // Don't try other patterns for this file
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// UI helpers
// ---------------------------------------------------------------------------

/// Component for displaying discovered sequences in UI
#[derive(Component)]
pub struct SequenceList {
    #[allow(dead_code)]
    pub sequences: Vec<SequenceInfo>,
}

/// Basic info about a discovered sequence
#[derive(Debug, Clone)]
pub struct SequenceInfo {
    #[allow(dead_code)]
    pub name: String,
    #[allow(dead_code)]
    pub frame_count: usize,
    #[allow(dead_code)]
    pub pattern: String,
    #[allow(dead_code)]
    pub base_dir: PathBuf,
}

impl From<&Sequence> for SequenceInfo {
    fn from(sequence: &Sequence) -> Self {
        Self {
            name: sequence.name.clone(),
            frame_count: sequence.frame_count(),
            pattern: sequence.pattern.clone(),
            base_dir: sequence.base_dir.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::File;
    use tempfile::TempDir;

    #[test]
    fn test_sequence_discovery() {
        // Create a temporary directory with test files
        let temp_dir = TempDir::new().unwrap();
        let dir_path = temp_dir.path();

        // Create test sequence files
        let files = vec![
            "simulation_001.stl",
            "simulation_002.stl",
            "simulation_003.stl",
            "other_file.txt",
        ];

        for file in &files {
            File::create(dir_path.join(file)).unwrap();
        }

        // Discover sequences
        let patterns = SequencePatterns::default();
        let sequences = discover_sequences(
            dir_path,
            false,
            &patterns,
            crate::lib::coordinates::SourceOrientation::default(),
        )
        .unwrap();

        // Should find one sequence
        assert_eq!(sequences.len(), 1);
        let sequence = &sequences[0];
        assert_eq!(sequence.name, "simulation");
        assert_eq!(sequence.frame_count(), 3);
    }

    #[test]
    fn test_intermediate_extension_discovery() {
        // Test files with intermediate extensions like .ply.serde.obj
        let temp_dir = TempDir::new().unwrap();
        let dir_path = temp_dir.path();

        // Create test files with intermediate extensions
        let files = vec![
            "particles-1000.ply.serde.obj",
            "particles-1008.ply.serde.obj",
            "particles-1016.ply.serde.obj",
            "unrelated_file.txt",
        ];

        for file in &files {
            File::create(dir_path.join(file)).unwrap();
        }

        // Discover sequences
        let patterns = SequencePatterns::default();
        let sequences = discover_sequences(
            dir_path,
            false,
            &patterns,
            crate::lib::coordinates::SourceOrientation::default(),
        )
        .unwrap();

        // Should find one sequence
        assert_eq!(sequences.len(), 1);
        let sequence = &sequences[0];
        assert_eq!(sequence.name, "particles");
        assert_eq!(sequence.frame_count(), 3);

        // Verify frame numbers are extracted correctly
        assert_eq!(sequence.frames[0].frame_number, 1000);
        assert_eq!(sequence.frames[1].frame_number, 1008);
        assert_eq!(sequence.frames[2].frame_number, 1016);
    }

    #[test]
    fn test_extension_filter() {
        // Verify only supported extensions are matched
        assert!(has_supported_extension("test.stl"));
        assert!(has_supported_extension("test.STL"));
        assert!(has_supported_extension("test.obj"));
        assert!(has_supported_extension("test.glb"));
        assert!(has_supported_extension("test.gltf"));
        assert!(has_supported_extension("test.ply.serde.obj"));
        assert!(has_supported_extension("test.data.glb"));

        assert!(!has_supported_extension("test.txt"));
        assert!(!has_supported_extension("test.fbx"));
        assert!(!has_supported_extension("test.ply"));
        assert!(!has_supported_extension("test.dat"));
    }

    #[test]
    fn test_unsupported_extension_ignored() {
        // Files with unsupported extensions should be ignored
        let temp_dir = TempDir::new().unwrap();
        let dir_path = temp_dir.path();

        let files = vec![
            "frame_001.stl",
            "frame_002.stl",
            "frame_003.txt", // Unsupported
            "frame_004.ply", // Unsupported
            "frame_005.stl",
        ];

        for file in &files {
            File::create(dir_path.join(file)).unwrap();
        }

        let patterns = SequencePatterns::default();
        let sequences = discover_sequences(
            dir_path,
            false,
            &patterns,
            crate::lib::coordinates::SourceOrientation::default(),
        )
        .unwrap();

        // Should find one sequence with only 3 STL files
        assert_eq!(sequences.len(), 1);
        assert_eq!(sequences[0].frame_count(), 3);
    }
}
