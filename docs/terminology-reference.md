# Terminology Reference: CFD/Science vs Animation/Cinema

This document surveys naming conventions from computational fluid dynamics (CFD) tools, scientific visualization software, and animation/cinema pipelines. The goal is to inform naming decisions for seaview's multi-series architecture (see seaview-6pd).

## The Core Concepts We Need Names For

| Concept | Description |
|---------|-------------|
| **A simulation execution** | One run of a solver producing a time-varying mesh dataset |
| **A sequence of mesh files** | The ordered frames on disk from that run |
| **A resolution/fidelity variant** | Same sim run, different mesh density (decimated by seaview-pipeline) |
| **The combination of sim run + resolution** | The atomic unit the viewer loads and plays back |
| **A cache window into a sequence** | A contiguous range of frames loaded into memory |
| **The playback state for a sequence** | Playhead position, frame range, loaded sections |

---

## CFD / Scientific Visualization

### OpenFOAM

| Term | Meaning |
|------|---------|
| **Case** | A complete simulation setup: mesh, boundary conditions, solver config, and results. A directory (e.g. `cavity/`) containing `0/`, `constant/`, `system/`. |
| **Time directory** | A single timestep's output (e.g. `0.5/`). Contains field files (U, p, etc.). Analogous to a "frame". |
| **Run** | A single solver execution. One case can have multiple runs (restart, parameter sweep). |
| **Region** | A sub-domain within a case (e.g. fluid region, solid region in conjugate heat transfer). |
| **Mesh** | The spatial discretization. Can change between runs or be static. |
| **Field** | A named quantity defined over the mesh at a given time (velocity, pressure, etc.). |
| **Decomposition** | Splitting a case across MPI ranks. Each rank gets a `processor0/`, `processor1/`, etc. |
| **Reconstruction** | Reassembling decomposed results into a single dataset. |

**Naming style:** Filesystem-driven. The directory _is_ the data model. No concept of "series" — you point at a case directory.

### ParaView / VTK

| Term | Meaning |
|------|---------|
| **Source** | Anything that produces data in the pipeline (reader, generator, filter). |
| **Reader** | A source that loads data from files. |
| **Timestep** | A single temporal snapshot. ParaView indexes these as integers or float time values. |
| **Multiblock dataset** | A tree of datasets grouped together. Used for assemblies, coupled simulations, or multi-region meshes. |
| **Multipiece dataset** | Datasets that are parts of a whole mesh (e.g. from parallel decomposition). Same type, same attributes. |
| **Pipeline** | The chain of source → filter → filter → render. |
| **Data array / Field / Attribute** | Named quantities on points or cells. |
| **Temporal file series** | A collection of files representing timesteps of the same dataset (e.g. `result_0000.vtk` through `result_1000.vtk`). This is the closest analog to our "sequence". |
| **Animation** | ParaView's term for playback across timesteps. The "Animation View" controls time. |

**Naming style:** Data-pipeline oriented. Everything is a "source" producing "datasets" at "timesteps". No special term for "a sim run" — it's just whatever a reader loads.

### FluidX3D (LBM solver)

| Term | Meaning |
|------|---------|
| **Simulation** | The solver execution. |
| **Frame** | A single exported mesh/field snapshot. |
| **Export** | Writing mesh data to disk at a given interval. |
| **Resolution** | Grid resolution of the LBM lattice (e.g. 256³). Directly determines mesh density. |

**Naming style:** Minimal. "Simulation" and "frame" are the only real concepts.

### EnSight (CEI / Ansys)

| Term | Meaning |
|------|---------|
| **Case** | The top-level container. A `.case` file references geometry + variable files. |
| **Time set** | A sequence of timesteps. A case can have multiple time sets (e.g. geometry changing at different rate than variables). |
| **Part** | A named subset of the mesh (boundary, volume, surface group). |
| **Model** | The geometric representation at a given time. |

**Naming style:** "Case" is the universal container. "Time set" is notable — it separates the concept of temporal sequence from the data itself.

### VisIt (LLNL)

| Term | Meaning |
|------|---------|
| **Database** | Any data source (file, collection of files, remote server). A "database" can span many timesteps. |
| **Cycle** | Integer timestep index. |
| **Time** | Float simulation time. |
| **Mesh** | The spatial discretization. |
| **State** | A specific (cycle, time) pair. |

**Naming style:** "Database" as the top-level is unusual but makes sense for a tool that connects to live simulations.

---

## Animation / Cinema / VFX

### Houdini (SideFX)

| Term | Meaning |
|------|---------|
| **Take** | A hierarchically overlayed set of parameter changes. Lets you create alternative versions of a scene and switch between them. Children inherit from parents. |
| **Shot** | A segment of animation between two cuts. |
| **Sequence** | An ordered collection of shots (e.g. a scene in a film). |
| **Frame** | A single temporal snapshot. Integer-indexed. |
| **Frame range** | Start/end frame for playback or rendering. |
| **Geometry cache** | Baked mesh data on disk, frame-by-frame (`.bgeo`, `.abc`). |
| **File cache** | A SOP node that writes/reads geometry sequences to/from disk. The standard way to handle heavy sim data. |
| **Variant** | An alternative version of an asset (e.g. different LOD, damage state). USD-native concept. |
| **LOD (Level of Detail)** | Resolution variant of a mesh, typically for real-time rendering. |
| **Render pass** | One rendering of a scene with specific settings (beauty, shadow, ambient occlusion). Takes can drive these. |

**Naming style:** "Take" for parameter variations. "Sequence" for ordered shots. "Cache" for heavy frame-by-frame data on disk. These are well-established terms in the VFX industry.

### Nuke (Foundry) / Compositing

| Term | Meaning |
|------|---------|
| **Clip** | A media file or sequence of frames. |
| **Read node** | Loads a clip into the node graph. |
| **Frame range** | The extent of a clip or project. |
| **Version** | An iteration of a shot (v001, v002, ...). Managed by pipeline tools. |
| **Plate** | Original camera footage (the "ground truth" for a shot). |

**Naming style:** "Clip" for a sequence of frames. "Version" for iterations (very pipeline-oriented).

### Non-Linear Editing (Premiere, Resolve, Avid)

| Term | Meaning |
|------|---------|
| **Project** | Top-level container for all media and edit decisions. |
| **Clip** | A single piece of media (video file, audio file, image sequence). |
| **Sequence** | An edited arrangement of clips on a timeline (Premiere/Avid). In Resolve, this is a "Timeline". |
| **Timeline** | The temporal arrangement where clips are placed and trimmed. |
| **Track** | A lane within a timeline (video track, audio track). |
| **In/Out point** | The start/end of the usable portion of a clip. |
| **Bin** | An organizational folder for clips. |
| **EDL (Edit Decision List)** | A list of edit points referencing source clips. The "recipe" for an edit. |
| **Proxy** | A low-resolution version of a clip used for faster editing. Swapped for full-res at render time. |
| **Reel** | Originally a physical film roll. Now used to mean a source tape or source identifier. |
| **Source monitor** | Preview of raw clip. **Program monitor** = preview of edited sequence. |

**Naming style:** "Clip" is the atomic media unit. "Sequence" or "Timeline" is the temporal arrangement. "Proxy" for low-res variants. Very clean separation between source material and edit decisions.

### USD (Universal Scene Description)

| Term | Meaning |
|------|---------|
| **Stage** | The composed scene (the "document"). |
| **Layer** | A file contributing opinions to a stage. Layers stack. |
| **Prim** | A scene element (mesh, xform, camera, etc.). |
| **Variant set** | Named alternatives for a prim (e.g. `modelComplexity` with values `low`, `medium`, `high`). |
| **Time sample** | A value at a specific time code. |
| **Clip** | USD Value Clips: a mechanism for loading per-frame data from separate files. Designed exactly for heavy sim caches. |
| **Asset** | A self-contained unit of scene description (a model, a material, etc.). |

**Naming style:** "Variant set" for alternatives (including resolution). "Clip" for temporal frame data from disk. USD's value clips are architecturally very close to what seaview does with mesh sequences.

---

## Summary Table

| Seaview concept | CFD/Science | Animation/Cinema | USD |
|----------------|-------------|-------------------|-----|
| A simulation execution | **Case** (OpenFOAM/EnSight), **Run**, **Database** (VisIt) | — | — |
| An ordered set of mesh frames on disk | **Temporal file series** (ParaView), **Time set** (EnSight) | **Clip** (NLE/Nuke), **Geometry cache** / **File cache** (Houdini) | **Value Clip** |
| A resolution/fidelity variant | — (rerun at different resolution) | **LOD**, **Variant** (Houdini/USD), **Proxy** (NLE) | **Variant set** |
| The loadable unit (sim + resolution) | **Case** (implicitly one resolution per case) | **Clip** | **Clip** + **Variant** |
| A cache window (loaded frame range) | — (ParaView loads on demand, no explicit concept) | **In/Out point**, **Frame range** | **Active time range** |
| Playback state | **Animation** (ParaView) | **Timeline** / **Sequence** (NLE) | **Time code** + **Stage** |

---

## Naming Recommendation for Seaview

Based on the survey above, here's what maps well:

| Seaview term | Rationale |
|-------------|-----------|
| **Series** | The atomic loadable unit: one sim run at one resolution. Avoids collision with ParaView's "source", Houdini's "take", or NLE's "clip" (which implies editing). "Series" naturally means "an ordered set of things" — a series of mesh frames. It's neutral across domains. |
| **Timeline** | Per-series playback state (playhead, frame range, load sections). Already used in seaview. Matches NLE convention. |
| **Load section** | A contiguous range of frames cached in memory. Already used in seaview. Closest NLE analog is "in/out point" but "section" is more descriptive for non-contiguous multi-window loading. |
| **Resolution** / **Variant** | A fidelity level within a sim run. "Variant" is USD/Houdini-native. "Resolution" is more intuitive for CFD users. Either works; "resolution" is more self-describing for our audience. |

### What we considered but rejected

| Term | Why not |
|------|---------|
| **Case** | Too OpenFOAM-specific. Also implies the full sim setup (boundary conditions, solver config), not just the output mesh frames. |
| **Clip** | Strong NLE connotation of "editable media". We're not editing — we're viewing sim data. Also USD uses "clip" for something more specific (value clips for per-frame file loading). |
| **Sequence** | Already used internally for `Sequence` struct (the raw frame list). Overloading it for the higher-level concept would be confusing. If we renamed the existing `Sequence` this could work, but it's a big refactor for marginal naming gain. |
| **Take** | Houdini-specific. Means "parameter variation" not "data source". |
| **Run** | Too generic. Collides with "run the application". |
| **Source** | ParaView uses this for pipeline nodes. Too abstract. |
| **Reel** | Archaic. Film-era term. |
| **Database** | VisIt-specific. Implies SQL-like storage, not mesh frames. |