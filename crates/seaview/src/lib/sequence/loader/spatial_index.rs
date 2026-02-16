//! Spatial index for loaded mesh frames.
//!
//! This module maintains a [`SpatialIndex`] resource that holds a
//! [`SpatialMesh`](crate::lib::spatial::SpatialMesh) for each loaded frame,
//! enabling spatial queries (ray casting, point projection, AABB) against the
//! mesh geometry.
//!
//! The [`build_spatial_meshes`] system watches for [`FrameLoadedEvent`]s and
//! builds the spatial acceleration structures from the now-ready Bevy `Mesh`
//! assets.
//!
//! ## Time-budgeted building
//!
//! BVH construction for large meshes (>300K triangles) can take 250–450 ms,
//! which would freeze the UI if done eagerly for every loaded frame.  Instead
//! the system maintains a **deferred queue** and spends at most
//! [`BUILD_BUDGET_MS`] per frame on construction.  The current playhead frame
//! is always prioritised so that spatial queries on the visible mesh are
//! available as soon as possible.

use bevy::mesh::Mesh;
use bevy::prelude::*;
use std::collections::VecDeque;
use std::time::Instant;

use super::assets::FrameStore;
use super::FrameLoadedEvent;
use crate::lib::sequence::Timeline;
use crate::lib::spatial::{SpatialMesh, SpatialMeshError};

// ---------------------------------------------------------------------------
// Tuning
// ---------------------------------------------------------------------------

/// Maximum milliseconds to spend on BVH construction per frame.
///
/// One large mesh takes ~250–450 ms, so a budget of 4 ms means we'll skip
/// building entirely most frames and only build when the queue has been
/// waiting long enough (or when the playhead frame needs it).  Set to 0 to
/// disable eager background building entirely (lazy-only mode).
#[allow(dead_code)]
const BUILD_BUDGET_MS: f64 = 2.0;

/// When the deferred queue exceeds this length, log a warning.
#[allow(dead_code)]
const QUEUE_WARN_THRESHOLD: usize = 200;

// ---------------------------------------------------------------------------
// Resource
// ---------------------------------------------------------------------------

/// Per-frame spatial acceleration structures, indexed in parallel with
/// [`FrameStore::frame_handles`].
///
/// Each frame may contain multiple meshes (e.g. multi-object GLB files), so
/// the inner type is `Vec<SpatialMesh>`.
#[derive(Resource, Default)]
pub struct SpatialIndex {
    /// Spatial meshes per frame.  `None` means the spatial mesh has not been
    /// built yet (either the frame isn't loaded, or build failed).
    meshes: Vec<Option<Vec<SpatialMesh>>>,

    /// Number of frames that have a spatial mesh built.
    built_count: usize,

    /// Cumulative build time across all frames (for diagnostics).
    total_build_time_ms: f64,

    /// Deferred build queue — frame indices whose FrameLoadedEvent arrived
    /// but we couldn't build within the time budget.  The current playhead
    /// frame is promoted to the front whenever it appears.
    deferred_queue: VecDeque<usize>,

    // -- Diagnostics --
    /// Number of frames deferred (not built) because budget was exhausted.
    pub(crate) deferred_count: u64,

    /// Number of frames built via lazy (on-demand) path.
    pub(crate) lazy_build_count: u64,

    /// Number of frames built via eager (budgeted) path.
    pub(crate) eager_build_count: u64,

    /// Peak deferred queue length observed.
    pub(crate) peak_queue_len: usize,

    /// Total wall-clock time spent in build_spatial_meshes system (ms).
    pub(crate) total_system_time_ms: f64,

    /// Number of ticks the build system has run.
    pub(crate) tick_count: u64,
}

impl SpatialIndex {
    /// Reset for a new sequence with `n` frames.
    pub fn reset(&mut self, num_frames: usize) {
        self.meshes.clear();
        self.meshes.resize_with(num_frames, || None);
        self.built_count = 0;
        self.total_build_time_ms = 0.0;
        self.deferred_queue.clear();
        self.deferred_count = 0;
        self.lazy_build_count = 0;
        self.eager_build_count = 0;
        self.peak_queue_len = 0;
        self.total_system_time_ms = 0.0;
        self.tick_count = 0;
    }

    /// Get the spatial meshes for a frame, if built.
    pub fn get(&self, frame_index: usize) -> Option<&[SpatialMesh]> {
        self.meshes
            .get(frame_index)
            .and_then(|opt| opt.as_ref())
            .map(|v| v.as_slice())
    }

    /// Get the first spatial mesh for a frame (convenience for single-mesh frames).
    pub fn get_first(&self, frame_index: usize) -> Option<&SpatialMesh> {
        self.get(frame_index).and_then(|meshes| meshes.first())
    }

    /// Check whether a spatial mesh has been built for this frame.
    pub fn is_built(&self, frame_index: usize) -> bool {
        self.meshes
            .get(frame_index)
            .is_some_and(|opt| opt.is_some())
    }

    /// Number of frames with spatial meshes built.
    pub fn built_count(&self) -> usize {
        self.built_count
    }

    /// Total number of frame slots.
    pub fn total_slots(&self) -> usize {
        self.meshes.len()
    }

    /// Cumulative BVH build time in milliseconds (for diagnostics).
    pub fn total_build_time_ms(&self) -> f64 {
        self.total_build_time_ms
    }

    /// Current deferred queue length.
    pub fn deferred_queue_len(&self) -> usize {
        self.deferred_queue.len()
    }

    /// Store spatial meshes for a frame.  Returns `true` if this was a new
    /// entry (frame didn't already have spatial data).
    #[allow(dead_code)]
    fn insert(&mut self, frame_index: usize, spatial_meshes: Vec<SpatialMesh>) -> bool {
        if frame_index >= self.meshes.len() {
            return false;
        }
        let is_new = self.meshes[frame_index].is_none();
        self.meshes[frame_index] = Some(spatial_meshes);
        if is_new {
            self.built_count += 1;
        }
        is_new
    }

    /// Record build time for diagnostics.
    #[allow(dead_code)]
    fn record_build_time(&mut self, ms: f64) {
        self.total_build_time_ms += ms;
    }

    /// Push a frame index onto the deferred queue (if not already built).
    #[allow(dead_code)]
    fn defer(&mut self, frame_index: usize) {
        if !self.is_built(frame_index) {
            self.deferred_queue.push_back(frame_index);
            self.deferred_count += 1;
            if self.deferred_queue.len() > self.peak_queue_len {
                self.peak_queue_len = self.deferred_queue.len();
            }
        }
    }

    /// Promote a frame index to the front of the deferred queue (for playhead
    /// priority).  If the frame is already in the queue, moves it to front;
    /// otherwise pushes it to front.
    #[allow(dead_code)]
    fn prioritise(&mut self, frame_index: usize) {
        if self.is_built(frame_index) {
            return;
        }
        // Remove existing entry if present.
        if let Some(pos) = self
            .deferred_queue
            .iter()
            .position(|&idx| idx == frame_index)
        {
            self.deferred_queue.remove(pos);
        }
        self.deferred_queue.push_front(frame_index);
    }
}

// ---------------------------------------------------------------------------
// Pure function
// ---------------------------------------------------------------------------

/// Build `SpatialMesh` instances from all Bevy `Mesh` handles for a single
/// frame.
///
/// Returns `Ok(meshes)` with one `SpatialMesh` per handle, or `Err` with the
/// list of per-mesh errors (partial successes are included in the `Ok` path
/// only if *all* meshes succeed).
#[allow(dead_code)]
pub fn build_spatial_meshes_for_frame(
    mesh_handles: &[Handle<Mesh>],
    mesh_assets: &Assets<Mesh>,
) -> Result<Vec<SpatialMesh>, Vec<(usize, SpatialMeshError)>> {
    let mut spatial_meshes = Vec::with_capacity(mesh_handles.len());
    let mut errors = Vec::new();

    for (i, handle) in mesh_handles.iter().enumerate() {
        let Some(mesh) = mesh_assets.get(handle) else {
            errors.push((
                i,
                SpatialMeshError::MissingPositions, // mesh asset not found
            ));
            continue;
        };

        match SpatialMesh::from_bevy_mesh(mesh) {
            Ok(sm) => spatial_meshes.push(sm),
            Err(e) => errors.push((i, e)),
        }
    }

    if errors.is_empty() {
        Ok(spatial_meshes)
    } else {
        Err(errors)
    }
}

// ---------------------------------------------------------------------------
// Lazy builder (on-demand, for the current frame)
// ---------------------------------------------------------------------------

/// Build the spatial mesh for `frame_index` **right now** if it hasn't been
/// built yet.  Returns whether a build was performed.
///
/// This is the "lazy" path — called when external code (e.g. ray-cast
/// queries) needs the spatial mesh for a specific frame and can't wait for
/// the budgeted builder.
#[allow(dead_code)]
pub fn ensure_built(
    frame_index: usize,
    frame_store: &FrameStore,
    mesh_assets: &Assets<Mesh>,
    spatial_index: &mut SpatialIndex,
) -> bool {
    if spatial_index.is_built(frame_index) {
        return false;
    }
    let Some(mesh_handles) = frame_store.get_frame_meshes(frame_index) else {
        return false;
    };

    let start = Instant::now();
    match build_spatial_meshes_for_frame(mesh_handles, mesh_assets) {
        Ok(spatial_meshes) => {
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            spatial_index.record_build_time(elapsed_ms);
            spatial_index.insert(frame_index, spatial_meshes);
            spatial_index.lazy_build_count += 1;
            debug!(
                "SpatialIndex: lazy-built frame {} in {:.1} ms [{}/{}]",
                frame_index,
                elapsed_ms,
                spatial_index.built_count(),
                spatial_index.total_slots(),
            );
            true
        }
        Err(errors) => {
            let elapsed_ms = start.elapsed().as_secs_f64() * 1000.0;
            spatial_index.record_build_time(elapsed_ms);
            for (mesh_i, err) in &errors {
                warn!(
                    "SpatialIndex: lazy-build frame {} mesh {} failed: {}",
                    frame_index, mesh_i, err,
                );
            }
            false
        }
    }
}

// ---------------------------------------------------------------------------
// System
// ---------------------------------------------------------------------------

/// System that builds [`SpatialMesh`] acceleration structures when frames
/// finish loading.
///
/// Listens for [`FrameLoadedEvent`]s emitted by the loading tracker and GLB
/// extractor.  New events are added to a deferred queue rather than built
/// immediately.  The system then drains the queue within a per-frame time
/// budget ([`BUILD_BUDGET_MS`]), prioritising the current playhead frame.
///
/// Frames that don't get built within the budget remain in the queue and
/// will be processed in subsequent frames.  External code that needs a
/// spatial mesh immediately can use [`ensure_built`].
#[allow(dead_code)]
pub(crate) fn build_spatial_meshes(
    mut events: MessageReader<FrameLoadedEvent>,
    frame_store: Res<FrameStore>,
    mesh_assets: Res<Assets<Mesh>>,
    mut spatial_index: ResMut<SpatialIndex>,
    timeline: Res<Timeline>,
) {
    let system_start = Instant::now();

    // 1. Collect new events into the deferred queue.
    let mut new_events = 0u32;
    for event in events.read() {
        if !event.success {
            continue;
        }
        let idx = event.frame_index;
        if !spatial_index.is_built(idx) {
            spatial_index.defer(idx);
            new_events += 1;
        }
    }

    // 2. Prioritise the current playhead frame so it's built first.
    let playhead = timeline.playhead;
    if playhead < spatial_index.total_slots() {
        spatial_index.prioritise(playhead);
    }

    // 3. Drain the deferred queue within the time budget.
    let mut builds_this_tick = 0u32;
    let mut build_ms_this_tick = 0.0f64;
    let drain_start = Instant::now();

    while let Some(idx) = spatial_index.deferred_queue.pop_front() {
        // Skip if already built (could have been built lazily or by a
        // previous iteration in this tick).
        if spatial_index.is_built(idx) {
            continue;
        }

        // Check if mesh data is available yet.
        let Some(mesh_handles) = frame_store.get_frame_meshes(idx) else {
            // Mesh not ready — re-queue at the back.
            spatial_index.deferred_queue.push_back(idx);
            break; // Don't spin; try again next tick.
        };

        let build_start = Instant::now();
        match build_spatial_meshes_for_frame(mesh_handles, &mesh_assets) {
            Ok(spatial_meshes) => {
                let elapsed_ms = build_start.elapsed().as_secs_f64() * 1000.0;
                let total_tris: usize = spatial_meshes.iter().map(|sm| sm.triangle_count()).sum();
                let total_verts: usize = spatial_meshes.iter().map(|sm| sm.vertex_count()).sum();
                let mesh_count = spatial_meshes.len();

                spatial_index.record_build_time(elapsed_ms);
                spatial_index.insert(idx, spatial_meshes);
                spatial_index.eager_build_count += 1;
                builds_this_tick += 1;
                build_ms_this_tick += elapsed_ms;

                if elapsed_ms > 5.0
                    || spatial_index.built_count() <= 3
                    || spatial_index.built_count() % 100 == 0
                {
                    info!(
                        "SpatialIndex: frame {} built ({} meshes, {} verts, {} tris) in {:.1} ms \
                         [{}/{}] queue: {}",
                        idx,
                        mesh_count,
                        total_verts,
                        total_tris,
                        elapsed_ms,
                        spatial_index.built_count(),
                        spatial_index.total_slots(),
                        spatial_index.deferred_queue.len(),
                    );
                }
            }
            Err(errors) => {
                let elapsed_ms = build_start.elapsed().as_secs_f64() * 1000.0;
                spatial_index.record_build_time(elapsed_ms);
                build_ms_this_tick += elapsed_ms;
                builds_this_tick += 1;

                for (mesh_i, err) in &errors {
                    warn!(
                        "SpatialIndex: frame {} mesh {} failed: {}",
                        idx, mesh_i, err,
                    );
                }
            }
        }

        // Check time budget — always allow at least one build per tick so
        // the queue makes progress, then enforce the budget.
        let elapsed_since_drain = drain_start.elapsed().as_secs_f64() * 1000.0;
        if builds_this_tick >= 1 && elapsed_since_drain >= BUILD_BUDGET_MS {
            break;
        }
    }

    // 4. Track diagnostics.
    let system_elapsed_ms = system_start.elapsed().as_secs_f64() * 1000.0;
    spatial_index.total_system_time_ms += system_elapsed_ms;
    spatial_index.tick_count += 1;

    // Update peak queue length.
    let qlen = spatial_index.deferred_queue.len();
    if qlen > spatial_index.peak_queue_len {
        spatial_index.peak_queue_len = qlen;
    }

    // Log summary periodically.
    if new_events > 0 || builds_this_tick > 0 {
        let avg_ms = if spatial_index.tick_count > 0 {
            spatial_index.total_system_time_ms / spatial_index.tick_count as f64
        } else {
            0.0
        };
        debug!(
            "SpatialIndex tick: +{} events, {} built ({:.1} ms), queue: {}, avg system: {:.2} ms/tick",
            new_events,
            builds_this_tick,
            build_ms_this_tick,
            qlen,
            avg_ms,
        );
    }

    // Warn if queue is getting very large.
    if qlen > QUEUE_WARN_THRESHOLD && spatial_index.tick_count % 60 == 0 {
        warn!(
            "SpatialIndex: deferred queue is large ({} frames). \
             Built {}/{} total, avg {:.1} ms/build. \
             Consider reducing loaded frame count or increasing BUILD_BUDGET_MS.",
            qlen,
            spatial_index.built_count(),
            spatial_index.total_slots(),
            if spatial_index.built_count() > 0 {
                spatial_index.total_build_time_ms / spatial_index.built_count() as f64
            } else {
                0.0
            },
        );
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lib::spatial::SpatialMesh;

    // ── SpatialIndex resource ─────────────────────────────────────────

    #[test]
    fn test_reset() {
        let mut index = SpatialIndex::default();
        index.reset(10);
        assert_eq!(index.total_slots(), 10);
        assert_eq!(index.built_count(), 0);
        assert!(!index.is_built(0));
        assert_eq!(index.deferred_queue_len(), 0);
    }

    #[test]
    fn test_insert_and_get() {
        let mut index = SpatialIndex::default();
        index.reset(5);

        let sm = SpatialMesh::from_raw(
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            vec![[0, 1, 2]],
        )
        .unwrap();

        assert!(index.insert(2, vec![sm]));
        assert!(index.is_built(2));
        assert!(!index.is_built(0));
        assert_eq!(index.built_count(), 1);

        let meshes = index.get(2).unwrap();
        assert_eq!(meshes.len(), 1);
        assert_eq!(meshes[0].triangle_count(), 1);
    }

    #[test]
    fn test_get_first() {
        let mut index = SpatialIndex::default();
        index.reset(3);

        let sm = SpatialMesh::from_raw(
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            vec![[0, 1, 2]],
        )
        .unwrap();

        index.insert(1, vec![sm]);

        assert!(index.get_first(1).is_some());
        assert!(index.get_first(0).is_none());
        assert!(index.get_first(99).is_none()); // out of bounds
    }

    #[test]
    fn test_insert_out_of_bounds() {
        let mut index = SpatialIndex::default();
        index.reset(3);

        let sm = SpatialMesh::from_raw(
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            vec![[0, 1, 2]],
        )
        .unwrap();

        assert!(!index.insert(99, vec![sm])); // out of bounds
        assert_eq!(index.built_count(), 0);
    }

    #[test]
    fn test_duplicate_insert_does_not_double_count() {
        let mut index = SpatialIndex::default();
        index.reset(3);

        let make_sm = || {
            SpatialMesh::from_raw(
                vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
                vec![[0, 1, 2]],
            )
            .unwrap()
        };

        assert!(index.insert(0, vec![make_sm()])); // first insert → true
        assert!(!index.insert(0, vec![make_sm()])); // duplicate → false
        assert_eq!(index.built_count(), 1);
    }

    #[test]
    fn test_record_build_time() {
        let mut index = SpatialIndex::default();
        index.record_build_time(1.5);
        index.record_build_time(2.5);
        assert!((index.total_build_time_ms() - 4.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_multi_mesh_frame() {
        let mut index = SpatialIndex::default();
        index.reset(2);

        let sm1 = SpatialMesh::from_raw(
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            vec![[0, 1, 2]],
        )
        .unwrap();
        let sm2 = SpatialMesh::from_raw(
            vec![[2.0, 0.0, 0.0], [3.0, 0.0, 0.0], [2.0, 1.0, 0.0]],
            vec![[0, 1, 2]],
        )
        .unwrap();

        index.insert(0, vec![sm1, sm2]);
        let meshes = index.get(0).unwrap();
        assert_eq!(meshes.len(), 2);
        assert_eq!(index.get_first(0).unwrap().triangle_count(), 1);
    }

    // ── Deferred queue ────────────────────────────────────────────────

    #[test]
    fn test_defer_adds_to_queue() {
        let mut index = SpatialIndex::default();
        index.reset(5);

        index.defer(2);
        index.defer(3);
        assert_eq!(index.deferred_queue_len(), 2);
        assert_eq!(index.deferred_count, 2);
    }

    #[test]
    fn test_defer_skips_already_built() {
        let mut index = SpatialIndex::default();
        index.reset(5);

        let sm = SpatialMesh::from_raw(
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            vec![[0, 1, 2]],
        )
        .unwrap();
        index.insert(2, vec![sm]);

        index.defer(2); // already built — should not queue
        assert_eq!(index.deferred_queue_len(), 0);
    }

    #[test]
    fn test_prioritise_moves_to_front() {
        let mut index = SpatialIndex::default();
        index.reset(10);

        index.defer(1);
        index.defer(2);
        index.defer(3);
        assert_eq!(index.deferred_queue.len(), 3);

        // Prioritise frame 3 — should move to front.
        index.prioritise(3);
        assert_eq!(index.deferred_queue[0], 3);
        assert_eq!(index.deferred_queue.len(), 3); // no duplicates
    }

    #[test]
    fn test_prioritise_new_frame_to_front() {
        let mut index = SpatialIndex::default();
        index.reset(10);

        index.defer(1);
        index.defer(2);

        // Prioritise frame 5 that isn't in the queue yet.
        index.prioritise(5);
        assert_eq!(index.deferred_queue[0], 5);
        assert_eq!(index.deferred_queue.len(), 3);
    }

    #[test]
    fn test_prioritise_skips_already_built() {
        let mut index = SpatialIndex::default();
        index.reset(5);

        let sm = SpatialMesh::from_raw(
            vec![[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
            vec![[0, 1, 2]],
        )
        .unwrap();
        index.insert(2, vec![sm]);

        index.defer(0);
        index.prioritise(2); // already built — no-op
        assert_eq!(index.deferred_queue.len(), 1);
        assert_eq!(index.deferred_queue[0], 0);
    }

    #[test]
    fn test_peak_queue_len_tracked() {
        let mut index = SpatialIndex::default();
        index.reset(10);

        index.defer(0);
        index.defer(1);
        index.defer(2);
        assert_eq!(index.peak_queue_len, 3);

        // Pop one — peak should stay at 3.
        index.deferred_queue.pop_front();
        assert_eq!(index.peak_queue_len, 3);
    }

    // ── build_spatial_meshes_for_frame (pure function) ────────────────

    #[test]
    fn test_build_from_bevy_mesh_assets() {
        use bevy::mesh::{Indices, Mesh, PrimitiveTopology};

        let mut mesh_assets = Assets::<Mesh>::default();

        let mut mesh = Mesh::new(PrimitiveTopology::TriangleList, Default::default());
        mesh.insert_attribute(
            Mesh::ATTRIBUTE_POSITION,
            vec![[0.0f32, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]],
        );
        mesh.insert_indices(Indices::U32(vec![0, 1, 2]));

        let handle = mesh_assets.add(mesh);

        let result = build_spatial_meshes_for_frame(&[handle], &mesh_assets);
        assert!(result.is_ok());
        let spatial = result.unwrap();
        assert_eq!(spatial.len(), 1);
        assert_eq!(spatial[0].triangle_count(), 1);
        assert_eq!(spatial[0].vertex_count(), 3);
    }

    #[test]
    fn test_build_missing_mesh_asset() {
        let mesh_assets = Assets::<Mesh>::default();
        let handle = Handle::<Mesh>::default();

        let result = build_spatial_meshes_for_frame(&[handle], &mesh_assets);
        assert!(result.is_err());
    }

    #[test]
    fn test_build_empty_handles() {
        let mesh_assets = Assets::<Mesh>::default();

        let result = build_spatial_meshes_for_frame(&[], &mesh_assets);
        assert!(result.is_ok());
        assert!(result.unwrap().is_empty());
    }
}
