//! Diagnostics system for tracking rendering performance and mesh statistics

use bevy::diagnostic::{DiagnosticsStore, FrameTimeDiagnosticsPlugin};
use bevy::prelude::*;
use std::time::Duration;

/// Resource to track rendering statistics
#[derive(Resource, Debug)]
pub struct RenderingStats {
    /// Total number of vertices being rendered
    pub total_vertices: usize,
    /// Total number of triangles being rendered
    pub total_triangles: usize,
    /// Number of mesh entities
    pub mesh_count: usize,
    /// Number of visible mesh entities
    pub visible_mesh_count: usize,
    /// Largest single mesh (in vertices)
    pub largest_mesh_vertices: usize,
    /// Frame time history for analysis
    pub frame_time_history: Vec<f32>,
    /// Maximum history size
    pub max_history: usize,
    /// Last update time
    pub last_update: Duration,
}

impl Default for RenderingStats {
    fn default() -> Self {
        Self {
            total_vertices: 0,
            total_triangles: 0,
            mesh_count: 0,
            visible_mesh_count: 0,
            largest_mesh_vertices: 0,
            frame_time_history: Vec::new(),
            max_history: 300, // 5 seconds at 60fps
            last_update: Duration::ZERO,
        }
    }
}

impl RenderingStats {

    /// Add a frame time to history
    pub fn record_frame_time(&mut self, time_ms: f32) {
        self.frame_time_history.push(time_ms);
        if self.frame_time_history.len() > self.max_history {
            self.frame_time_history.remove(0);
        }
    }

    /// Get frame time statistics
    pub fn frame_stats(&self) -> FrameStats {
        if self.frame_time_history.is_empty() {
            return FrameStats::default();
        }

        let mut sorted = self.frame_time_history.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        let len = sorted.len();
        let sum: f32 = sorted.iter().sum();

        FrameStats {
            avg: sum / len as f32,
            min: sorted[0],
            max: sorted[len - 1],
            p50: sorted[len / 2],
            p95: sorted[(len as f32 * 0.95) as usize],
            p99: sorted[(len as f32 * 0.99) as usize],
            std_dev: {
                let avg = sum / len as f32;
                let variance = sorted.iter().map(|x| (x - avg).powi(2)).sum::<f32>() / len as f32;
                variance.sqrt()
            },
            spikes: sorted.iter().filter(|&&t| t > 33.33).count(), // > 30fps threshold
        }
    }
}

#[derive(Default, Debug)]
pub struct FrameStats {
    pub avg: f32,
    pub min: f32,
    pub max: f32,
    #[allow(dead_code)]
    pub p50: f32,
    pub p95: f32,
    pub p99: f32,
    pub std_dev: f32,
    pub spikes: usize,
}

/// Plugin for rendering diagnostics
pub struct RenderingDiagnosticsPlugin;

impl Plugin for RenderingDiagnosticsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<RenderingStats>().add_systems(
            Update,
            (
                update_rendering_stats,
                log_rendering_diagnostics,
                detect_performance_issues,
            ),
        );
    }
}

/// System to update rendering statistics
fn update_rendering_stats(
    mut stats: ResMut<RenderingStats>,
    time: Res<Time>,
    diagnostics: Res<DiagnosticsStore>,
    meshes: Res<Assets<Mesh>>,
    mesh_query: Query<(&Mesh3d, &Visibility), With<Mesh3d>>,
) {
    // Reset counters
    stats.total_vertices = 0;
    stats.total_triangles = 0;
    stats.mesh_count = 0;
    stats.visible_mesh_count = 0;
    stats.largest_mesh_vertices = 0;

    // Count mesh statistics
    for (mesh_handle, visibility) in mesh_query.iter() {
        stats.mesh_count += 1;

        if visibility == Visibility::Hidden {
            continue;
        }

        stats.visible_mesh_count += 1;

        if let Some(mesh) = meshes.get(&mesh_handle.0) {
            let vertex_count = mesh.count_vertices();
            stats.total_vertices += vertex_count;
            stats.total_triangles += vertex_count / 3; // Assuming triangle list
            stats.largest_mesh_vertices = stats.largest_mesh_vertices.max(vertex_count);
        }
    }

    // Record frame time
    if let Some(frame_time) = diagnostics.get(&FrameTimeDiagnosticsPlugin::FRAME_TIME) {
        if let Some(value) = frame_time.smoothed() {
            stats.record_frame_time(value as f32);
        }
    }

    stats.last_update = time.elapsed();
}

/// System to log rendering diagnostics periodically
fn log_rendering_diagnostics(
    stats: Res<RenderingStats>,
    time: Res<Time>,
    mut last_log: Local<f32>,
) {
    let current_time = time.elapsed_secs();

    // Log every 5 seconds
    if current_time - *last_log > 5.0 {
        *last_log = current_time;

        let frame_stats = stats.frame_stats();

        debug!(
            "Rendering Stats - Meshes: {} (visible: {}), Vertices: {}M, Triangles: {}M, Largest: {}k verts",
            stats.mesh_count,
            stats.visible_mesh_count,
            stats.total_vertices as f32 / 1_000_000.0,
            stats.total_triangles as f32 / 1_000_000.0,
            stats.largest_mesh_vertices as f32 / 1000.0,
        );

        debug!(
            "Frame Times - Avg: {:.1}ms, Min: {:.1}ms, Max: {:.1}ms, P95: {:.1}ms, P99: {:.1}ms, Spikes: {}",
            frame_stats.avg,
            frame_stats.min,
            frame_stats.max,
            frame_stats.p95,
            frame_stats.p99,
            frame_stats.spikes,
        );

        // Calculate GPU memory estimate (rough)
        let bytes_per_vertex = 12 + 12 + 8; // position + normal + uv
        let gpu_memory_mb = (stats.total_vertices * bytes_per_vertex) as f32 / 1_048_576.0;
        debug!("Estimated GPU memory usage: {:.1} MB", gpu_memory_mb);
    }
}

/// System to detect and warn about performance issues
fn detect_performance_issues(
    stats: Res<RenderingStats>,
    time: Res<Time>,
    mut last_check: Local<f32>,
) {
    let current_time = time.elapsed_secs();

    // Check every 2 seconds
    if current_time - *last_check > 2.0 {
        *last_check = current_time;

        let frame_stats = stats.frame_stats();

        // Warn if average frame time is bad
        if frame_stats.avg > 20.0 {
            warn!(
                "Poor average frame time: {:.1}ms ({:.0} FPS)",
                frame_stats.avg,
                1000.0 / frame_stats.avg
            );
        }

        // Warn if too many vertices
        if stats.total_vertices > 10_000_000 {
            warn!(
                "Very high vertex count: {}M vertices. Consider LOD or culling.",
                stats.total_vertices as f32 / 1_000_000.0
            );
        }

        // Warn if single mesh is too large
        if stats.largest_mesh_vertices > 5_000_000 {
            warn!(
                "Single mesh with {}M vertices detected. Consider splitting or decimating.",
                stats.largest_mesh_vertices as f32 / 1_000_000.0
            );
        }

        // Warn about frame time variance
        if frame_stats.std_dev > 10.0 {
            warn!(
                "High frame time variance: std dev {:.1}ms. FPS is unstable.",
                frame_stats.std_dev
            );
        }

        // Suggest optimizations based on stats
        if stats.total_vertices > 50_000_000 && stats.visible_mesh_count > 1 {
            debug!("Consider implementing:");
            debug!("  - Frustum culling (only render visible meshes)");
            debug!("  - Level of Detail (LOD) system");
            debug!("  - Mesh instancing for repeated geometry");
            debug!("  - Occlusion culling");
        }
    }
}

/// Component to track mesh statistics
#[derive(Component, Debug)]
pub struct MeshStats {
    #[allow(dead_code)]
    pub vertex_count: usize,
    #[allow(dead_code)]
    pub triangle_count: usize,
    #[allow(dead_code)]
    pub file_path: String,
}

/// System to add mesh stats when meshes are created
#[allow(dead_code)]
pub fn track_mesh_stats(
    mut commands: Commands,
    meshes: Res<Assets<Mesh>>,
    new_meshes: Query<(Entity, &Mesh3d), Added<Mesh3d>>,
) {
    for (entity, mesh_handle) in new_meshes.iter() {
        if let Some(mesh) = meshes.get(&mesh_handle.0) {
            let vertex_count = mesh.count_vertices();
            let triangle_count = vertex_count / 3;

            commands.entity(entity).insert(MeshStats {
                vertex_count,
                triangle_count,
                file_path: String::new(), // Would need to be passed through
            });

            if vertex_count > 1_000_000 {
                debug!(
                    "Large mesh spawned: {} vertices ({:.1}M triangles)",
                    vertex_count,
                    triangle_count as f32 / 1_000_000.0
                );
            }
        }
    }
}

/// Helper to analyze why performance might be bad
#[allow(dead_code)]
pub fn analyze_performance_issues(stats: &RenderingStats) -> Vec<String> {
    let mut issues = Vec::new();
    let frame_stats = stats.frame_stats();

    // Check total geometry
    if stats.total_vertices > 100_000_000 {
        issues.push(format!(
            "Extreme vertex count: {}M (recommended: <50M for smooth playback)",
            stats.total_vertices / 1_000_000
        ));
    } else if stats.total_vertices > 50_000_000 {
        issues.push(format!(
            "High vertex count: {}M (may cause performance issues)",
            stats.total_vertices / 1_000_000
        ));
    }

    // Check individual mesh size
    if stats.largest_mesh_vertices > 10_000_000 {
        issues.push(format!(
            "Oversized mesh detected: {}M vertices (should be <5M per mesh)",
            stats.largest_mesh_vertices / 1_000_000
        ));
    }

    // Check frame consistency
    if frame_stats.std_dev > 15.0 {
        issues.push(format!(
            "Unstable frame times: {:.1}ms std deviation",
            frame_stats.std_dev
        ));
    }

    // Check for GPU bottleneck indicators
    if frame_stats.p99 > frame_stats.avg * 3.0 {
        issues.push("Severe frame time spikes detected - likely GPU memory pressure".to_string());
    }

    // Memory bandwidth estimate
    let bandwidth_gb_s = (stats.total_vertices * 32 * 60) as f32 / 1_000_000_000.0; // rough estimate
    if bandwidth_gb_s > 50.0 {
        issues.push(format!(
            "High memory bandwidth requirement: ~{bandwidth_gb_s:.1} GB/s"
        ));
    }

    issues
}
