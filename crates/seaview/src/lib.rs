//! Seaview mesh viewer library
//!
//! This library provides functionality for viewing and processing 3D mesh data,
//! including network communication for real-time mesh streaming.

pub mod lib {
    pub mod asset_loaders;
    pub mod coordinates;
    pub mod global_settings;
    pub mod lighting;
    pub mod memory_budget;
    pub mod mesh_info;
    // pub mod network;
    pub mod sequence;
    pub mod session;
    pub mod settings;
    pub mod toast;
}

pub mod app {
    pub mod cli;
    pub mod systems;
    pub mod ui;
}

// Re-export commonly used types from lib modules
pub use app::ui::SeaviewUiPlugin;
pub use lib::coordinates;
pub use lib::lighting::{self, NightLightingConfig, NightLightingPlugin};
pub use lib::mesh_info::{MeshDimensions, MeshInfoPlugin, RecomputeMeshBounds};
// pub use lib::network::{self, MeshReceiver, ReceivedMesh};
pub use lib::asset_loaders::{self, AssetLoadersPlugin};
pub use lib::memory_budget::{
    estimate_mesh_gpu_bytes, estimate_mesh_gpu_bytes_from_vertices, MemoryBudget,
    MemoryBudgetPlugin, NvmlBackend, RamDetector, SysInfoBackend, VramDetector,
};
pub use lib::sequence::{
    self, compute_spawn_window,
    discovery::{discover_sequences, SequencePatterns},
    distribute_sections, FrameInfo, LoadSection, MeshFileFormat, Sequence, Timeline,
};
pub use lib::session::{self, Session, SessionManager, SessionPlugin};
pub use lib::toast::{Toast, ToastLevel, ToastQueue};
