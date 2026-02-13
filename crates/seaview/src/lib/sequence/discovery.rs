//! Sequence discovery module for finding mesh file sequences

use super::{loader::LoadSequenceRequest, FrameInfo, Sequence, SequenceEvent, SequenceManager};
use bevy::prelude::*;
use regex::Regex;
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Plugin for sequence discovery functionality
pub struct SequenceDiscoveryPlugin;

impl Plugin for SequenceDiscoveryPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, handle_discovery_requests);
    }
}

/// Component that requests sequence discovery in a directory
#[derive(Component)]
pub struct DiscoverSequenceRequest {
    pub directory: PathBuf,
    pub recursive: bool,
    pub source_orientation: crate::lib::coordinates::SourceOrientation,
}

/// Resource for managing sequence discovery patterns
#[derive(Resource)]
pub struct SequencePatterns {
    /// List of regex patterns to match sequence files
    patterns: Vec<SequencePattern>,
}

impl Default for SequencePatterns {
    fn default() -> Self {
        Self {
            patterns: vec![
                // Common simulation output patterns - STL
                SequencePattern::new("frame_number", r"^(.+?)[\._-]?(\d{3,6})\.stl$", 2),
                SequencePattern::new("timestep", r"^(.+?)[\._-]?[Tt](\d+)\.stl$", 2),
                SequencePattern::new("step", r"^(.+?)[\._-]?[Ss]tep[\._-]?(\d+)\.stl$", 2),
                SequencePattern::new("time", r"^(.+?)[\._-]?[Tt]ime[\._-]?(\d+)\.stl$", 2),
                // Generic numbered pattern - STL
                SequencePattern::new("numbered", r"^(.+?)(\d{3,})\.stl$", 2),
                // Common simulation output patterns - glTF
                SequencePattern::new("frame_number_gltf", r"^(.+?)[\._-]?(\d{3,6})\.gltf$", 2),
                SequencePattern::new("timestep_gltf", r"^(.+?)[\._-]?[Tt](\d+)\.gltf$", 2),
                SequencePattern::new("step_gltf", r"^(.+?)[\._-]?[Ss]tep[\._-]?(\d+)\.gltf$", 2),
                SequencePattern::new("time_gltf", r"^(.+?)[\._-]?[Tt]ime[\._-]?(\d+)\.gltf$", 2),
                // Generic numbered pattern - glTF
                SequencePattern::new("numbered_gltf", r"^(.+?)(\d{3,})\.gltf$", 2),
                // Common simulation output patterns - GLB
                SequencePattern::new("frame_number_glb", r"^(.+?)[\._-]?(\d{3,6})\.glb$", 2),
                SequencePattern::new("timestep_glb", r"^(.+?)[\._-]?[Tt](\d+)\.glb$", 2),
                SequencePattern::new("step_glb", r"^(.+?)[\._-]?[Ss]tep[\._-]?(\d+)\.glb$", 2),
                SequencePattern::new("time_glb", r"^(.+?)[\._-]?[Tt]ime[\._-]?(\d+)\.glb$", 2),
                // Generic numbered pattern - GLB
                SequencePattern::new("numbered_glb", r"^(.+?)(\d{3,})\.glb$", 2),
            ],
        }
    }
}

/// A pattern for matching sequence files
struct SequencePattern {
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
}

/// System that handles sequence discovery requests
fn handle_discovery_requests(
    mut commands: Commands,
    query: Query<(Entity, &DiscoverSequenceRequest)>,
    mut sequence_manager: ResMut<SequenceManager>,
    mut events: MessageWriter<SequenceEvent>,
    mut load_events: MessageWriter<LoadSequenceRequest>,
    patterns: Option<Res<SequencePatterns>>,
) {
    // Use default patterns if none provided
    let default_patterns = SequencePatterns::default();
    let patterns = patterns.as_deref().unwrap_or(&default_patterns);

    for (entity, request) in query.iter() {
        debug!("Discovering sequences in: {:?}", request.directory);
        events.write(SequenceEvent::DiscoveryStarted(request.directory.clone()));

        match discover_sequences(
            &request.directory,
            request.recursive,
            patterns,
            request.source_orientation,
        ) {
            Ok(sequences) => {
                debug!("Found {} sequences", sequences.len());

                // For now, load the first sequence if any found
                if let Some(sequence) = sequences.into_iter().next() {
                    let frame_count = sequence.frame_count();
                    let sequence_name = sequence.name.clone();

                    // Trigger mesh loading via the loader system
                    let frame_paths: Vec<_> =
                        sequence.frames.iter().map(|f| f.path.clone()).collect();
                    if !frame_paths.is_empty() {
                        debug!(
                            "Triggering load of {} mesh frames for sequence '{}'",
                            frame_paths.len(),
                            sequence_name
                        );
                        load_events.write(LoadSequenceRequest { frame_paths });
                    }

                    sequence_manager.load_sequence(sequence);
                    events.write(SequenceEvent::SequenceLoaded(sequence_name));
                    events.write(SequenceEvent::DiscoveryCompleted(frame_count));
                } else {
                    events.write(SequenceEvent::Error("No sequences found".to_string()));
                }
            }
            Err(e) => {
                error!("Failed to discover sequences: {}", e);
                events.write(SequenceEvent::Error(format!("Discovery failed: {e}")));
            }
        }

        // Remove the request component
        commands.entity(entity).remove::<DiscoverSequenceRequest>();
    }
}

/// Discover sequences in a directory
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
                // Try each pattern
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
}
