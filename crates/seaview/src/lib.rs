//! Seaview mesh viewer library
//!
//! This library provides functionality for viewing and processing 3D mesh data,
//! including network communication for real-time mesh streaming.

pub mod lib {
    pub mod actions;
    pub mod coordinates;
    pub mod environment_objects;
    pub mod fade;
    pub mod global_settings;
    pub mod lighting;
    pub mod memory_budget;
    pub mod mesh_info;
    // pub mod network;
    pub mod sequence;
    pub mod series;
    pub mod session;
    pub mod settings;
    pub mod spatial;
    pub mod toast;
}

pub mod app {
    pub mod cli;
    pub mod environment_objects;
    pub mod input;
    #[cfg(feature = "spatial")]
    pub mod spatial_accelerator;
    pub mod systems;
    pub mod ui;
}

// Re-export commonly used types from lib modules
pub use app::ui::SeaviewUiPlugin;
pub use lib::coordinates;
pub use lib::lighting::{self, EnvironmentLightConfig, LightingRig, NightLightingPlugin};
pub use lib::mesh_info::{MeshDimensions, MeshInfoPlugin, RecomputeMeshBounds};
// pub use lib::network::{self, MeshReceiver, ReceivedMesh};
pub use lib::global_settings::{GlobalSettings, MemorySettings};
pub use lib::memory_budget::{
    estimate_mesh_gpu_bytes, estimate_mesh_gpu_bytes_from_vertices, human_bytes, MemoryBudget,
    MemoryBudgetPlugin, NoVramDetector, NvmlBackend, RamDetector, SysInfoBackend, VramDetector,
};
pub use lib::sequence::{
    self, compute_spawn_window,
    discovery::{discover_sequences, SequencePatterns},
    distribute_sections, FrameInfo, LoadSection, Sequence, SpatialIndex, Timeline,
};
// Re-export loadfu's MeshFormat so bins and downstream crates can use it
pub use app::input::{ActionStateRes, InputActionPlugin, InputMap};
pub use lib::actions::{Action, ActionState};
pub use lib::environment_objects::{
    EnvironmentObject, EnvironmentObjectId, EnvironmentObjectStore, MeshEntry, Transform3D,
};
pub use lib::fade::{format_relative_time, FadeEffect};
pub use lib::session::{self, Session, SessionManager, SessionPlugin};
pub use lib::spatial::{BvhNodeAabb, PointProjection, RayHit, SpatialMesh, SpatialMeshError};
pub use lib::series::{SeriesId, SeriesManager, SeriesState, SeriesSummary};
pub use lib::sequence::SeriesManagerRes;
pub use lib::sequence::discovery::AllSequencesDiscovered;
pub use lib::toast::{Toast, ToastLevel, ToastQueue};
pub use loadfu::MeshFormat;
