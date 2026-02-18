# Seaview Research Documentation

Research and planning documents for the Seaview mesh sequence viewer.

## Active Research

### [Camera, Lighting & Animation](./camera_lighting_animation.md)
Camera rigs (orbit, turntable, dolly, FPS switching), camera animation with dual-spline paths, lighting animation with keyframed properties, 3D gizmos, Bezier curves, and a 5-phase implementation plan.

### [Architectural Review](./architectural_review.md)
Full codebase audit rating every file 🟢/🟡/🟠/🔴. Eight priority actions identified — top issues: settings.rs monolith, main.rs god function, system code in lib/, bin/ duplication.

### [Mesh Loading Ecosystem](./mesh_loading_ecosystem.md)
Bevy mesh loading ecosystem analysis, community loader landscape, loader-agnostic `MeshSource` trait design, and capability querying (`MeshCapabilities`). Informs loadfu architecture.

### [mmap Loader Architecture](./mmap_loader_architecture.md)
Full data-flow analysis from disk through parse crates to GPU. Custom `svmesh` format design (memory-mappable, pre-triangulated). Peak RSS analysis. Informs seaview-joh (zero-copy mmap format).

### [Mesh Loading Bypass](./mesh-loading-bypass.md)
Strategies for bypassing Bevy's asset system for direct mesh loading when needed.

### [Settings Save System](./settings_save_system.md)
Research on persisting application settings and UI state. Informs seaview-z6m (panel layout persistence) and seaview-nqv (persistent UI state).

## Reference

### [Bevy Asset Loading](./bevy_asset_loading.md)
Idiomatic mesh loading using Bevy's native asset system — async parallel loading, arbitrary directories, indexed mesh handling, sequence loading strategies. See also: [Quick Reference](./bevy_asset_loading_summary.md).

### [Lighting Units](./lighting_units.md)
Physical lighting unit reference for Bevy's PBR renderer — lumens, lux, candelas, and practical values for scene lighting.

### [Overlays](./overlays.md)
HUD overlay research — rendering stats, debug info, viewport annotations. Informs seaview-nvh.

### [Bobbing Figure](./bobbing_figure.md)
Animated test object for verifying playback, timing, and mesh swap correctness.

## External

### [loadfu Design Doc](../../vendor/loadfu/docs/loadfu.md)
Design doc for `loadfu`, the format-agnostic 3D asset loader crate in `vendor/loadfu/`. Orthogonal to svmesh (format vs loader).

## Retired (this session)

The following were removed as superseded (210+ days stale):

- `cameras.md` → superseded by [camera_lighting_animation.md](./camera_lighting_animation.md)
- `mesh-loading.md`, `mesh_formats_comparison.md` → superseded by [mesh_loading_ecosystem.md](./mesh_loading_ecosystem.md) and loadfu
- `async_implementation_summary.md`, `parallel_stl_loading.md` → superseded by loadfu
- `baby_shark_mesh_conversion.md` → completed, no longer relevant
- `../performance/` (profiling_guide, tracing_setup) → superseded by [../profiling-guide.md](../profiling-guide.md)
- `../BRP_EXTRAS.md`, `../brp_mcp_integration.md` → stale BRP integration docs
- `../async_loader_integration.md`, `../mesh_cache_migration_summary.md` → old loader pipeline
- `../mesh-conversion-tools.md`, `../mesh-format-summary.md` → superseded by loadfu
- `../REMOVAL_PLAN.md` → completed (old loader removal done)
- `../roadmaps/01_network_sequencing.md`, `04_config.md`, `05_crate-reorg.md`, `center-camera-feature.md` → completed or superseded