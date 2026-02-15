//! Memory budget detection and management
//!
//! This module provides trait-based memory detection with multiple backend implementations:
//! - `SysInfoBackend`: Uses sysinfo crate for RAM detection
//! - `WgpuBackend`: Uses wgpu limits for VRAM estimation
//! - `NvmlBackend`: (Future) Uses NVIDIA Management Library for accurate VRAM on NVIDIA GPUs
//!
//! The `MemoryBudget` resource combines these to compute frame loading limits.
//!
//! ## Mesh Memory Estimation
//!
//! Use [`estimate_mesh_gpu_bytes`] to estimate GPU memory for a mesh based on
//! vertex and index counts. This accounts for:
//! - Position (3 x f32 = 12 bytes/vertex)
//! - Normal (3 x f32 = 12 bytes/vertex)
//! - UV (2 x f32 = 8 bytes/vertex)
//! - Indices (4 bytes/index for u32)
//! - Buffer alignment overhead (~10%)

use bevy::prelude::*;
use bevy::render::renderer::RenderDevice;
use sysinfo::System;

use crate::lib::global_settings::MemorySettings;

/// Trait for detecting system RAM
pub trait RamDetector: Send + Sync {
    /// Returns total system RAM in bytes
    fn total_ram_bytes(&self) -> u64;

    /// Returns available (free) RAM in bytes
    fn available_ram_bytes(&self) -> u64;
}

/// Trait for detecting GPU VRAM
pub trait VramDetector: Send + Sync {
    /// Returns total VRAM in bytes, if detectable
    fn total_vram_bytes(&self) -> Option<u64>;

    /// Returns available VRAM in bytes, if detectable
    fn available_vram_bytes(&self) -> Option<u64>;

    /// Returns the GPU device name
    fn device_name(&self) -> String;
}

// ============================================================================
// SysInfo Backend (RAM)
// ============================================================================

/// RAM detection using the sysinfo crate
pub struct SysInfoBackend {
    system: System,
}

impl SysInfoBackend {
    pub fn new() -> Self {
        let mut system = System::new();
        system.refresh_memory();
        Self { system }
    }

    /// Refresh memory information
    pub fn refresh(&mut self) {
        self.system.refresh_memory();
    }
}

impl Default for SysInfoBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl RamDetector for SysInfoBackend {
    fn total_ram_bytes(&self) -> u64 {
        self.system.total_memory()
    }

    fn available_ram_bytes(&self) -> u64 {
        self.system.available_memory()
    }
}

// ============================================================================
// Wgpu Backend (VRAM estimation)
// ============================================================================

/// VRAM estimation using wgpu device limits
///
/// Note: wgpu doesn't expose actual VRAM amounts directly. We use `max_buffer_size`
/// as a heuristic - it's typically a fraction of total VRAM that the driver allows
/// for a single buffer allocation.
pub struct WgpuBackend {
    device_name: String,
    max_buffer_size: u64,
    /// Estimated total VRAM based on heuristics
    estimated_vram: u64,
}

impl WgpuBackend {
    /// Create from wgpu limits and adapter info
    ///
    /// # Arguments
    /// * `max_buffer_size` - From `RenderDevice::limits().max_buffer_size`
    /// * `device_name` - From `RenderAdapterInfo.name`
    pub fn new(max_buffer_size: u64, device_name: String) -> Self {
        // Heuristic: max_buffer_size is often ~25-50% of total VRAM
        // We estimate conservatively at 4x max_buffer_size, capped at reasonable values
        let estimated_vram = estimate_vram_from_buffer_limit(max_buffer_size);

        Self {
            device_name,
            max_buffer_size,
            estimated_vram,
        }
    }

    /// Get the raw max_buffer_size limit
    pub fn max_buffer_size(&self) -> u64 {
        self.max_buffer_size
    }
}

impl VramDetector for WgpuBackend {
    fn total_vram_bytes(&self) -> Option<u64> {
        Some(self.estimated_vram)
    }

    fn available_vram_bytes(&self) -> Option<u64> {
        // wgpu doesn't provide this - return None to indicate unknown
        None
    }

    fn device_name(&self) -> String {
        self.device_name.clone()
    }
}

/// Estimate total VRAM from max_buffer_size limit
///
/// This is a heuristic based on typical driver behavior:
/// - Most drivers set max_buffer_size to 25-50% of VRAM
/// - We use 4x as a conservative multiplier
/// - Clamp to reasonable bounds (512MB - 48GB)
fn estimate_vram_from_buffer_limit(max_buffer_size: u64) -> u64 {
    const MIN_VRAM: u64 = 512 * 1024 * 1024; // 512 MB
    const MAX_VRAM: u64 = 48 * 1024 * 1024 * 1024; // 48 GB
    const MULTIPLIER: u64 = 4;

    let estimated = max_buffer_size.saturating_mul(MULTIPLIER);
    estimated.clamp(MIN_VRAM, MAX_VRAM)
}

// ============================================================================
// Mesh Memory Estimation
// ============================================================================

/// Estimate GPU memory usage for a mesh based on vertex and index counts.
///
/// This accounts for typical mesh vertex attributes:
/// - Position: 3 x f32 = 12 bytes per vertex
/// - Normal: 3 x f32 = 12 bytes per vertex
/// - UV: 2 x f32 = 8 bytes per vertex
/// - Indices: 4 bytes per index (u32)
///
/// Adds ~10% overhead for GPU buffer alignment and metadata.
///
/// # Example
/// ```ignore
/// let mesh_bytes = estimate_mesh_gpu_bytes(100_000, 300_000);
/// // ~100k vertices * 32 bytes + 300k indices * 4 bytes + 10% overhead
/// // ≈ 4.8 MB
/// ```
pub fn estimate_mesh_gpu_bytes(vertex_count: usize, index_count: usize) -> u64 {
    const BYTES_PER_VERTEX: usize = 12 + 12 + 8; // position + normal + uv = 32 bytes
    const BYTES_PER_INDEX: usize = 4; // u32
    const OVERHEAD_FACTOR: f64 = 1.1; // 10% for alignment/metadata

    let vertex_bytes = vertex_count * BYTES_PER_VERTEX;
    let index_bytes = index_count * BYTES_PER_INDEX;
    let total = (vertex_bytes + index_bytes) as f64 * OVERHEAD_FACTOR;

    total as u64
}

/// Estimate GPU memory for a mesh using only vertex count (assumes indexed triangles).
///
/// For triangle meshes without explicit index count, assumes indices ≈ vertices
/// (typical for indexed triangle lists).
pub fn estimate_mesh_gpu_bytes_from_vertices(vertex_count: usize) -> u64 {
    // For typical indexed triangle meshes, index count is roughly equal to vertex count
    // (each triangle has 3 indices, and vertices are shared)
    estimate_mesh_gpu_bytes(vertex_count, vertex_count)
}

// ============================================================================
// NVML Backend (Future - accurate NVIDIA VRAM)
// ============================================================================

/// Placeholder for future NVML-based VRAM detection
///
/// NVML (NVIDIA Management Library) provides accurate VRAM information
/// for NVIDIA GPUs. This would require the `nvml-wrapper` crate and
/// linking against the NVML library.
#[cfg(feature = "nvml")]
pub struct NvmlBackend {
    // TODO: Add nvml-wrapper Device handle
    device_name: String,
    total_vram: u64,
    // nvml: nvml_wrapper::Nvml,
}

#[cfg(feature = "nvml")]
impl NvmlBackend {
    pub fn new() -> Option<Self> {
        // TODO: Initialize NVML and query GPU memory
        // let nvml = nvml_wrapper::Nvml::init().ok()?;
        // let device = nvml.device_by_index(0).ok()?;
        // let memory_info = device.memory_info().ok()?;
        None
    }
}

#[cfg(feature = "nvml")]
impl VramDetector for NvmlBackend {
    fn total_vram_bytes(&self) -> Option<u64> {
        Some(self.total_vram)
    }

    fn available_vram_bytes(&self) -> Option<u64> {
        // TODO: Query nvml for free memory
        None
    }

    fn device_name(&self) -> String {
        self.device_name.clone()
    }
}

// ============================================================================
// Memory Budget Resource
// ============================================================================

/// Computed memory budget for frame loading
///
/// This resource is inserted after render initialization and provides
/// the calculated memory limits based on detected hardware and user settings.
#[derive(Resource, Debug, Clone)]
pub struct MemoryBudget {
    /// Total detected system RAM in bytes
    pub total_ram: u64,
    /// Available system RAM in bytes (at detection time)
    pub available_ram: u64,
    /// Estimated/detected VRAM in bytes
    pub total_vram: Option<u64>,
    /// GPU device name
    pub device_name: String,

    /// Computed RAM budget in bytes (after applying fraction)
    pub ram_budget: u64,
    /// Computed VRAM budget in bytes (after applying fraction)
    pub vram_budget: u64,

    /// Estimated size per frame in bytes (updated after first frame loads)
    pub frame_size_estimate: u64,
    /// Maximum frames that fit in budget
    pub max_frames: usize,

    /// Current memory used by loaded frames
    pub used_bytes: u64,
}

impl Default for MemoryBudget {
    fn default() -> Self {
        Self {
            total_ram: 0,
            available_ram: 0,
            total_vram: None,
            device_name: "Unknown".to_string(),
            ram_budget: 0,
            vram_budget: 0,
            frame_size_estimate: 0,
            max_frames: usize::MAX,
            used_bytes: 0,
        }
    }
}

impl MemoryBudget {
    /// Create a new MemoryBudget by detecting hardware and applying settings
    pub fn detect(
        ram_detector: &impl RamDetector,
        vram_detector: &impl VramDetector,
        settings: &MemorySettings,
    ) -> Self {
        let total_ram = ram_detector.total_ram_bytes();
        let available_ram = ram_detector.available_ram_bytes();
        let total_vram = vram_detector.total_vram_bytes();
        let device_name = vram_detector.device_name();

        // Apply fractions to compute budgets
        let ram_budget = if settings.auto_detect {
            (total_ram as f64 * settings.ram_fraction as f64) as u64
        } else if let Some(manual_mb) = settings.budget_mb {
            manual_mb * 1024 * 1024
        } else {
            (total_ram as f64 * settings.ram_fraction as f64) as u64
        };

        let vram_budget = total_vram
            .map(|v| (v as f64 * settings.gpu_fraction as f64) as u64)
            .unwrap_or(ram_budget); // Fallback to RAM budget if VRAM unknown

        // Use the minimum of RAM and VRAM budgets as the effective limit
        // (meshes need to fit in both system RAM during loading and VRAM for rendering)
        let effective_budget = ram_budget.min(vram_budget);

        info!(
            "Memory budget: RAM {:.1} GB / {:.1} GB, VRAM ~{:.1} GB, effective budget: {:.1} GB",
            ram_budget as f64 / 1_073_741_824.0,
            total_ram as f64 / 1_073_741_824.0,
            vram_budget as f64 / 1_073_741_824.0,
            effective_budget as f64 / 1_073_741_824.0,
        );

        Self {
            total_ram,
            available_ram,
            total_vram,
            device_name,
            ram_budget,
            vram_budget,
            frame_size_estimate: 0,
            max_frames: usize::MAX,
            used_bytes: 0,
        }
    }

    /// Create with manual budget override (ignores detection)
    pub fn with_manual_budget(budget_mb: u64) -> Self {
        let budget_bytes = budget_mb * 1024 * 1024;
        Self {
            ram_budget: budget_bytes,
            vram_budget: budget_bytes,
            ..Default::default()
        }
    }

    /// Get the effective memory budget (minimum of RAM and VRAM)
    pub fn effective_budget(&self) -> u64 {
        self.ram_budget.min(self.vram_budget)
    }

    /// Update frame size estimate and recalculate max_frames
    ///
    /// Call this after the first frame is loaded to get accurate estimates.
    pub fn update_frame_estimate(&mut self, frame_bytes: u64) {
        self.frame_size_estimate = frame_bytes;
        if frame_bytes > 0 {
            let budget = self.effective_budget();
            self.max_frames = (budget / frame_bytes) as usize;
            info!(
                "Frame size estimate: {:.1} MB, max frames in budget: {}",
                frame_bytes as f64 / 1_048_576.0,
                self.max_frames
            );
        }
    }

    /// Check if we can load more frames
    pub fn can_load_frame(&self) -> bool {
        self.used_bytes + self.frame_size_estimate <= self.effective_budget()
    }

    /// Record that a frame was loaded
    pub fn record_frame_loaded(&mut self, bytes: u64) {
        self.used_bytes = self.used_bytes.saturating_add(bytes);
    }

    /// Record that a frame was unloaded
    pub fn record_frame_unloaded(&mut self, bytes: u64) {
        self.used_bytes = self.used_bytes.saturating_sub(bytes);
    }

    /// Get memory usage as a fraction (0.0 - 1.0)
    pub fn usage_fraction(&self) -> f32 {
        let budget = self.effective_budget();
        if budget == 0 {
            0.0
        } else {
            (self.used_bytes as f64 / budget as f64) as f32
        }
    }
}

// ============================================================================
// Bevy Plugin
// ============================================================================

/// Plugin that sets up memory budget detection
///
/// Detection happens in two phases:
/// 1. Startup: Detect RAM immediately using sysinfo
/// 2. First Update: Detect GPU once RenderDevice is available
pub struct MemoryBudgetPlugin;

impl Plugin for MemoryBudgetPlugin {
    fn build(&self, app: &mut App) {
        // Insert a default budget initially
        app.init_resource::<MemoryBudget>();
        app.init_resource::<GpuDetectionState>();

        // Phase 1: Detect RAM on startup
        app.add_systems(Startup, detect_ram_on_startup);

        // Phase 2: Detect GPU once render device is available (runs once)
        app.add_systems(
            Update,
            detect_gpu_on_first_frame.run_if(resource_exists::<RenderDevice>.and(run_once)),
        );
    }
}

/// Tracks whether GPU detection has completed
#[derive(Resource, Default)]
struct GpuDetectionState {
    completed: bool,
}

/// System to detect RAM on startup (Phase 1)
fn detect_ram_on_startup(
    mut budget: ResMut<MemoryBudget>,
    settings: Option<Res<crate::lib::global_settings::GlobalSettings>>,
) {
    let memory_settings = settings
        .as_ref()
        .and_then(|s| s.memory.clone())
        .unwrap_or_default();

    // Detect RAM using sysinfo
    let ram_backend = SysInfoBackend::new();

    let total_ram = ram_backend.total_ram_bytes();
    let available_ram = ram_backend.available_ram_bytes();

    // Calculate RAM budget
    let ram_budget = if let Some(manual_mb) = memory_settings.budget_mb {
        manual_mb * 1024 * 1024
    } else {
        (total_ram as f64 * memory_settings.ram_fraction as f64) as u64
    };

    budget.total_ram = total_ram;
    budget.available_ram = available_ram;
    budget.ram_budget = ram_budget;
    // Set VRAM budget to RAM budget initially (will be updated when GPU detected)
    budget.vram_budget = ram_budget;

    info!(
        "Detected RAM: {:.1} GB total, {:.1} GB available, budget: {:.1} GB",
        total_ram as f64 / 1_073_741_824.0,
        available_ram as f64 / 1_073_741_824.0,
        ram_budget as f64 / 1_073_741_824.0,
    );
}

/// System to detect GPU capabilities once RenderDevice is available (Phase 2)
fn detect_gpu_on_first_frame(
    mut budget: ResMut<MemoryBudget>,
    mut state: ResMut<GpuDetectionState>,
    render_device: Res<RenderDevice>,
    settings: Option<Res<crate::lib::global_settings::GlobalSettings>>,
) {
    if state.completed {
        return;
    }
    state.completed = true;

    let memory_settings = settings
        .as_ref()
        .and_then(|s| s.memory.clone())
        .unwrap_or_default();

    let limits = render_device.limits();
    let raw_max_buffer_size = limits.max_buffer_size;

    // Sanity-cap: some drivers report absurd max_buffer_size (e.g. 2^62).
    // Cap at 32 GB — no single buffer should ever be larger than total VRAM.
    const MAX_SANE_BUFFER_SIZE: u64 = 32 * 1024 * 1024 * 1024; // 32 GB
    let max_buffer_size = raw_max_buffer_size.min(MAX_SANE_BUFFER_SIZE);

    if raw_max_buffer_size != max_buffer_size {
        warn!(
            "GPU reports max_buffer_size={} bytes ({:.1} TB) — capping to {:.1} GB",
            raw_max_buffer_size,
            raw_max_buffer_size as f64 / (1024.0 * 1024.0 * 1024.0 * 1024.0),
            max_buffer_size as f64 / 1_073_741_824.0,
        );
    }

    // Create wgpu backend for VRAM estimation
    let vram_backend = WgpuBackend::new(max_buffer_size, "GPU".to_string());

    if let Some(estimated_vram) = vram_backend.total_vram_bytes() {
        budget.total_vram = Some(estimated_vram);
        budget.vram_budget = (estimated_vram as f64 * memory_settings.gpu_fraction as f64) as u64;
        budget.device_name = vram_backend.device_name();

        info!(
            "Detected GPU: max_buffer_size={} bytes ({:.1} GB), estimated VRAM: {:.1} GB, budget: {:.1} GB",
            max_buffer_size,
            max_buffer_size as f64 / 1_073_741_824.0,
            estimated_vram as f64 / 1_073_741_824.0,
            budget.vram_budget as f64 / 1_073_741_824.0,
        );
        info!(
            "Effective memory budget: {:.1} GB (min of RAM and VRAM budgets)",
            budget.effective_budget() as f64 / 1_073_741_824.0,
        );
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    struct MockRamDetector {
        total: u64,
        available: u64,
    }

    impl RamDetector for MockRamDetector {
        fn total_ram_bytes(&self) -> u64 {
            self.total
        }
        fn available_ram_bytes(&self) -> u64 {
            self.available
        }
    }

    struct MockVramDetector {
        total: Option<u64>,
        name: String,
    }

    impl VramDetector for MockVramDetector {
        fn total_vram_bytes(&self) -> Option<u64> {
            self.total
        }
        fn available_vram_bytes(&self) -> Option<u64> {
            None
        }
        fn device_name(&self) -> String {
            self.name.clone()
        }
    }

    #[test]
    fn test_budget_calculation() {
        let ram = MockRamDetector {
            total: 16 * 1024 * 1024 * 1024, // 16 GB
            available: 8 * 1024 * 1024 * 1024,
        };
        let vram = MockVramDetector {
            total: Some(8 * 1024 * 1024 * 1024), // 8 GB
            name: "Test GPU".to_string(),
        };
        let settings = MemorySettings {
            budget_mb: None,
            auto_detect: true,
            ram_fraction: 0.5,
            gpu_fraction: 0.5,
        };

        let budget = MemoryBudget::detect(&ram, &vram, &settings);

        // RAM budget: 16GB * 0.5 = 8GB
        assert_eq!(budget.ram_budget, 8 * 1024 * 1024 * 1024);
        // VRAM budget: 8GB * 0.5 = 4GB
        assert_eq!(budget.vram_budget, 4 * 1024 * 1024 * 1024);
        // Effective is min(8GB, 4GB) = 4GB
        assert_eq!(budget.effective_budget(), 4 * 1024 * 1024 * 1024);
    }

    #[test]
    fn test_frame_estimate() {
        let mut budget = MemoryBudget {
            ram_budget: 4 * 1024 * 1024 * 1024, // 4 GB
            vram_budget: 4 * 1024 * 1024 * 1024,
            ..Default::default()
        };

        // 100 MB per frame
        budget.update_frame_estimate(100 * 1024 * 1024);

        // Should fit ~40 frames
        assert_eq!(budget.max_frames, 40);
    }

    #[test]
    fn test_vram_estimation_heuristic() {
        // 1 GB max buffer -> ~4 GB estimated VRAM
        let estimated = estimate_vram_from_buffer_limit(1024 * 1024 * 1024);
        assert_eq!(estimated, 4 * 1024 * 1024 * 1024);

        // Very small buffer -> clamps to minimum
        let estimated = estimate_vram_from_buffer_limit(64 * 1024 * 1024);
        assert_eq!(estimated, 512 * 1024 * 1024);

        // Huge buffer -> clamps to maximum
        let estimated = estimate_vram_from_buffer_limit(20 * 1024 * 1024 * 1024);
        assert_eq!(estimated, 48 * 1024 * 1024 * 1024);
    }

    #[test]
    fn test_manual_budget() {
        let budget = MemoryBudget::with_manual_budget(2048); // 2 GB

        assert_eq!(budget.ram_budget, 2048 * 1024 * 1024);
        assert_eq!(budget.vram_budget, 2048 * 1024 * 1024);
        assert_eq!(budget.effective_budget(), 2048 * 1024 * 1024);
    }

    #[test]
    fn test_mesh_memory_estimation() {
        // 100k vertices, 300k indices
        let bytes = estimate_mesh_gpu_bytes(100_000, 300_000);
        // 100k * 32 bytes + 300k * 4 bytes = 3.2M + 1.2M = 4.4M
        // With 10% overhead ≈ 4.84M
        assert!(bytes > 4_000_000 && bytes < 6_000_000);

        // Vertices-only estimate
        let bytes2 = estimate_mesh_gpu_bytes_from_vertices(100_000);
        // 100k * 32 + 100k * 4 = 3.6M * 1.1 ≈ 3.96M
        assert!(bytes2 > 3_500_000 && bytes2 < 5_000_000);
    }

    #[test]
    fn test_usage_tracking() {
        let mut budget = MemoryBudget {
            ram_budget: 1000 * 1024 * 1024, // 1000 MB (use round number for easy math)
            vram_budget: 1000 * 1024 * 1024,
            frame_size_estimate: 100 * 1024 * 1024, // 100 MB
            ..Default::default()
        };

        assert!(budget.can_load_frame());
        assert_eq!(budget.usage_fraction(), 0.0);

        // 100 MB / 1000 MB = 0.1
        budget.record_frame_loaded(100 * 1024 * 1024);
        assert!((budget.usage_fraction() - 0.1).abs() < 0.001);

        // (100 + 400) / 1000 = 0.5
        budget.record_frame_loaded(400 * 1024 * 1024);
        assert!((budget.usage_fraction() - 0.5).abs() < 0.001);

        // (500 - 200) / 1000 = 0.3
        budget.record_frame_unloaded(200 * 1024 * 1024);
        assert!((budget.usage_fraction() - 0.3).abs() < 0.001);
    }
}
