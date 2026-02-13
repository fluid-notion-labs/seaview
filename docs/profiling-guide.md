# Profiling Guide for Seaview

## Quick Start: Profile the Current Implementation

### 1. Build with Chrome Tracing Enabled

```bash
# Build with profiling features (includes trace_chrome)
cargo build --release --features perf

# Or just Chrome tracing alone
cargo build --release --features trace_chrome
```

### 2. Run the Application

**Option A: Traces in /tmp (recommended)**
```bash
# Use the wrapper script that runs seaview from /tmp
# (traces will be written to /tmp instead of project root)
/tmp/seaview-trace.sh <path-to-sequence>
```

**Option B: Traces in project root**
```bash
# The trace file is automatically generated when you run
./target/release/seaview

# Trace files are saved in the project root as:
# trace-<timestamp>.json
```

**With logging to file:**
```bash
# Capture logs for analysis
RUST_LOG=info ./target/release/seaview | tee /tmp/seaview.log

# After loading completes, view frame-change logs:
grep -A 50 "LOADING COMPLETE" /tmp/seaview.log
```

### 3. Analyze the Trace

1. **Open Chrome/Chromium browser**
2. **Navigate to:** `chrome://tracing`
3. **Click "Load"** and select your `trace-*.json` file
4. **Use keyboard shortcuts:**
   - `W` / `S` - Zoom in / out
   - `A` / `D` - Pan left / right
   - `1` / `2` / `3` / `4` - Select tool modes
   - Click and drag to select time ranges

### 4. What to Look For

**Frame Change Performance:**
- Look for spikes in the timeline during frame transitions
- Search for `allocate_and_free_meshes` in the trace
- Check `update_mesh_display` and `handle_frame_changes` timing
- Expected: <0.1ms with visibility-swap approach
- Problem: 50ms-2600ms indicates GPU alloc/free is still happening

**Loading Performance:**
- `track_asset_loading` system timing
- Asset load events
- Entity spawn operations

**Rendering Pipeline:**
- `extract_meshes` - should only process visible entities
- `prepare_meshes` - GPU buffer uploads
- `queue_material_meshes` - material binding

## Alternative: Tracy Profiler (Advanced)

Tracy provides more detailed GPU profiling but requires setup:

```bash
# Build with Tracy support
cargo build --release --features trace_tracy

# Install Tracy server (on Ubuntu/Debian)
sudo apt install tracy

# Or download from: https://github.com/wolfpld/tracy/releases

# Run Tracy server first, then run seaview
./target/release/seaview
```

## Performance Testing Workflow

### Test 1: Verify Visibility Swap is Working

1. Load a sequence
2. Step through frames with arrow keys (or play)
3. In trace, look for:
   - **GOOD:** Only `Visibility::Hidden` / `Visibility::Visible` changes
   - **BAD:** `allocate_and_free_meshes` or mesh handle changes

### Test 2: Identify Frame Time Spikes

```bash
# Run with sequence loaded and playing, capture logs
RUST_LOG=info /tmp/seaview-trace.sh <path-to-sequence> | tee /tmp/seaview.log

# Steps to reproduce:
# 1. Wait for "✓ SEQUENCE LOADING COMPLETE" message
# 2. Start playback (spacebar)
# 3. Let it run for ~30 seconds
# 4. Exit and check trace + logs

# View frame switching logs:
grep -A 100 "LOADING COMPLETE" /tmp/seaview.log | grep "Switched visibility"
```

In the trace:
- **Filter by:** `update_mesh_display` or `handle_frame_changes`
- **Measure:** Min / avg / max / p99 frame times
- **Expected:** Consistent <1ms
- **Problem:** Spikes >10ms indicate GPU operations

### Test 3: Memory Pressure Detection

Look for these patterns in the trace:
- Increasing time in `allocate_and_free_meshes` over time (fragmentation)
- GPU stalls (gaps in render timeline)
- CPU waiting for GPU (long `finish` calls)

## Debugging Tools in the App

### FPS Counter
- Already available in diagnostics
- Check `RenderingStats` logs every 5 seconds

### Console Logging
- Info logs show loading complete: `"✓ SEQUENCE LOADING COMPLETE: N frames, N entities spawned"`
- Debug logs show frame switches: `"Switched visibility: frame X hidden, frame Y visible"`
- If you see these messages frequently, visibility swap is working

**Grep for important events:**
```bash
# Find when loading completed
grep "LOADING COMPLETE" /tmp/seaview.log

# View frame switches after loading
grep -A 200 "LOADING COMPLETE" /tmp/seaview.log | grep "Switched visibility"

# Count frame switches
grep "Switched visibility" /tmp/seaview.log | wc -l
```

### GPU Memory Monitoring (Linux)

```bash
# NVIDIA
watch -n 1 nvidia-smi

# AMD
watch -n 1 radeontop

# Intel
watch -n 1 intel_gpu_top
```

Expected: ~12-13GB stable usage for 388-frame sequence

## Known Issues to Check

### Issue 1: Visibility Not Actually Hiding Meshes
**Symptom:** GPU memory stays high, frame times still spike
**Check:** In trace, verify `extract_meshes` only processes 1 visible entity
**Fix:** Ensure Bevy's rendering respects `Visibility::Hidden`

### Issue 2: Commands Not Applied Same Frame
**Symptom:** Frame flicker or wrong mesh displayed
**Check:** System ordering - commands applied after render?
**Fix:** Use `apply_deferred` or check system schedule

### Issue 3: Material Changes Triggering GPU Upload
**Symptom:** Spikes when changing materials
**Check:** In trace, look for buffer uploads in `prepare_materials`
**Fix:** This is expected - material changes DO require GPU update

### Issue 4: All Entities Being Rendered Despite Visibility
**Symptom:** Vertex count in diagnostics = total all frames, not just visible
**Check:** `update_rendering_stats` counts
**Expected:** ~700K vertices (1 frame), not 270M (388 frames)
**Fix:** Bevy may be batching/culling incorrectly

## Build Profiles Comparison

| Profile | Command | Use Case | Symbols | LTO |
|---------|---------|----------|---------|-----|
| `dev` | `cargo run` | Quick iteration | Yes | No |
| `release` | `cargo build --release` | Production | No | Thin |
| `release-perf` | `cargo build --profile release-perf` | Max performance | No | Fat |
| `release --features perf` | For profiling | With traces | No | Thin |

## Next Steps After Profiling

1. **If visibility swap IS working (no alloc/free):**
   - Profile other bottlenecks (mesh extraction, culling, etc.)
   - Check GPU utilization vs CPU bottleneck
   - Consider LOD or frustum culling

2. **If visibility swap NOT working (still seeing alloc/free):**
   - Check Bevy version - visibility behavior changed
   - Verify entities are actually being hidden
   - Check if Bevy is aggressively culling hidden entities
   - May need to keep entities but remove from render world

3. **If slowness is during loading:**
   - Profile `track_asset_loading` and entity spawning
   - May need async spawning or frame spreading
   - Check asset loading parallelism

## Advanced: Perf Tools (Linux)

```bash
# System-wide profiling
perf record -F 999 -g ./target/release/seaview
perf report

# Flame graph generation
perf script | stackcollapse-perf.pl | flamegraph.pl > flamegraph.svg

# GPU profiling (NVIDIA)
nsys profile --stats=true ./target/release/seaview
nsys-ui report.nsys-rep
```

## Contact / Issues

If profiling shows unexpected behavior, capture:
1. Chrome trace file (`.json`)
2. Console logs with `RUST_LOG=debug`
3. GPU memory usage (`nvidia-smi` output)
4. System specs (GPU model, VRAM, driver version)

File issue with these artifacts for detailed analysis.