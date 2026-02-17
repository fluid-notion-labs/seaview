# Seaview Research Documentation

This directory contains research and planning documents for the Seaview mesh sequence viewer project.

## Documents

### [Reimplementation Plan](./reimplimentation.md)
A comprehensive phased approach to reimplementing mesh-ripper functionality using modern Rust and Bevy (0.14+). This is the main project roadmap outlining:

- 9 development phases from foundation to testing
- Technical architecture decisions
- Risk mitigation strategies
- Success metrics and timelines
- **Status**: Active development guide

### [Camera Plugins Research](./cameras.md)
Research and evaluation of available Bevy camera controller plugins for implementing FPS-style controls with recording/playback capabilities. Covers:

- Analysis of 6+ camera plugins
- Version compatibility and feature comparison
- Recommendations for Seaview implementation
- Custom implementation strategy
- **Status**: Research complete, implementation pending

### [Mesh File Loading Research](./mesh-loading.md)
Research on implementing multi-format mesh file loading in Bevy, with immediate focus on STL support. Covers:

- Native Bevy asset system architecture
- Third-party format support (STL, PLY, OBJ)
- Implementation strategies and code examples
- Performance considerations
- **Status**: Research complete, ready for implementation

### [Bevy Asset Loading Research](./bevy_asset_loading.md)
Comprehensive research on idiomatic mesh loading using Bevy's native asset system for well-behaved, indexed files. Covers:

- Bevy's async, parallel asset loading architecture
- Loading from arbitrary directories (not just `assets/`)
- Proper indexed mesh handling (fixes exploded mesh bug)
- Loading strategies for mesh sequences (preload, streaming, predictive)
- Reference counting and memory management
- Migration path from custom loaders to Bevy-native approach
- **Status**: Research complete, recommended as primary approach
- **Quick Reference**: See [summary version](./bevy_asset_loading_summary.md) for code snippets

### [Bevy Asset Loading - Quick Reference](./bevy_asset_loading_summary.md)
Quick reference guide with code patterns for loading glTF/GLB files from arbitrary directories. Includes:

- Working code for proper indexed mesh loading
- Async loading patterns with `AsyncComputeTaskPool`
- Sequence loading examples
- Migration checklist from current implementation
- **Status**: Ready to implement

### [Mesh Loading Ecosystem — Research & Strategy](./mesh_loading_ecosystem.md)
Analysis of the Bevy mesh loading ecosystem, asset system capabilities, and Seaview's loader-agnostic architecture strategy. Covers:

- Bevy's built-in asset system: what it solves, what it doesn't
- Community loader landscape (STL, OBJ, FBX, PLY, voxel, point cloud)
- Missing ecosystem pieces (PLY mesh, VTK, point clouds)
- Quads/n-gons and GPU reality (why TriangleList normalization is correct)
- Loader-agnostic `MeshSource` trait design
- Capability querying (`MeshCapabilities`) for format-aware UI/logic
- TriMesh normalization performance analysis
- **Status**: Research complete, informs seaview-bdz.4 (MeshSource trait)

### [mmap Loader Architecture — Research & Data Flow](./mmap_loader_architecture.md)
Analysis of the full data flow from disk through parse crates to GPU, with Mermaid graphs showing where mmap can intervene. Covers:

- Current pipeline copy audit (STL, OBJ, GLB — every byte gets copied 4–5 times)
- Full dependency matrix: format × layer (disk I/O, parse crate, triangulation, Mesh build)
- Four mmap intervention levels (read buffer → zero-copy parse → direct Mesh → GPU DMA)
- Custom `svmesh` format design (memory-mappable, pre-triangulated, pre-indexed) — note: svmesh is a *format*, orthogonal to loadfu (the *loader*)
- Integration points: which files change, which don't
- Peak RSS analysis: ~500 MB → ~140 MB per 5M-vertex frame
- Risks: page faults, Bevy Mesh ownership (owned Vec only), NFS
- **Status**: Research complete, informs seaview-joh (zero-copy mmap format)

### [loadfu — 3D Asset Loader for Bevy](../../vendor/loadfu/docs/loadfu.md)
Design doc for `loadfu`, a format-agnostic Bevy ecosystem crate for 3D asset loading (single assets and sequences). Covers:

- Crate architecture: single crate, Bevy behind feature flag, core parsers standalone
- Extraction plan from seaview's `lib/asset_loaders/` and `MeshFileFormat`
- Key types: `MeshData`, `MeshFormat`, `MeshCapabilities`
- Upstream path to Bevy proper
- Agentic refactoring skill with step-by-step extraction checklist
- **Status**: Design, pre-implementation
- **Relation to svmesh**: Orthogonal. svmesh is a format; loadfu is the loader.
- **Prior research**: [mesh_loading_ecosystem.md](./mesh_loading_ecosystem.md), [mmap_loader_architecture.md](./mmap_loader_architecture.md), [mesh-loading-bypass.md](./mesh-loading-bypass.md)
- **Location**: Lives in `vendor/loadfu/` — loadfu is its own crate

### [Baby Shark Mesh Conversion Research](./baby_shark_mesh_conversion.md)
Research on using the baby_shark library for converting polygon soup mesh data to indexed format. Covers:

- Analysis of baby_shark's mesh format types (PolygonSoup vs CornerTable)
- The merge_points algorithm for vertex deduplication
- Integration strategy for fixing mesh rendering issues
- Performance benefits and implementation considerations
- **Status**: Research complete, implementation in progress

## Project Status

Currently in **Phase 1: Foundation and Core Infrastructure**

### Completed:
- [x] Project workspace setup
- [x] Bevy 0.14 integration
- [x] Basic 3D scene rendering
- [x] Research documentation

### Next Steps:
- [ ] Simple mesh loading (Phase 1.2)
- [ ] Camera system implementation (Phase 4)
- [ ] File system utilities (Phase 1.3)

## Contributing

When adding new research documents:
1. Create the document in this directory
2. Add a link and description to this index
3. Follow the existing format for consistency
4. Include status information (research/planning/implementation/complete)

## Related Files

- [`../README.md`](../README.md) - Project overview and build instructions
- [`../Cargo.toml`](../Cargo.toml) - Workspace configuration
- [`../vendor/mesh-ripper/`](../vendor/mesh-ripper/) - Original implementation for reference
