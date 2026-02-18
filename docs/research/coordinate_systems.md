# Coordinate Systems: terrain-forge → FluidX3D → Seaview

## Pipeline Overview

```
terrain-forge (Z-up)  →  FluidX3D (Z-up internal)  →  surface export (Y-up)  →  Seaview/Bevy (Y-up)
       ↓                        ↓                            ↓
  terrain.stl              voxelize_stl              surface_NNNNNN.stl
  (raw coords)         (scale + center + rotate)     (lattice or SI coords)
```

## Coordinate Conventions

### terrain-forge (Z-up)

Generates heightmap meshes with vertices `[x, y, z]`:

- **X** = grid column × `xy_scale` (horizontal)
- **Y** = grid row × `xy_scale` (horizontal, depth)
- **Z** = heightmap value × `z_scale` (vertical / up)

Origin at corner `(0, 0, 0)`. A 512×512 heightmap with `xy_scale=1.0`, `z_scale=200.0` spans:

```
x ∈ [0, 511]
y ∈ [0, 511]
z ∈ [0, ~200]  (depends on noise)
```

### FluidX3D (Z-up internal)

LBM domain created with `resolution(float3(width, depth, height), memory_mb)`:

- **X** = width axis (`Nx` cells)
- **Y** = depth axis (`Ny` cells)
- **Z** = height axis (`Nz` cells)
- Gravity: `(0, 0, -g)` — Z is up

The terrain STL is voxelized into this domain with scale + centering:

```cpp
const float cx = center_x * (float)nx;
const float cy = center_y * (float)ny;
const float cz = center_z * (float)nz;
lbm.voxelize_stl(stl_path, float3(cx, cy, cz), rotation, scale, TYPE_S);
```

When `scale = 0.0`, FluidX3D auto-fits the mesh to the domain.

### Bevy / Seaview (Y-up, right-handed)

- **+X** = right
- **+Y** = up
- **+Z** = toward viewer (out of screen)

## The Two Problems

### 1. Flipped: X-mirror in `convert_to_yup`

**Current** (`surface_export.hpp`):

```cpp
inline float3 convert_to_yup(const float3& v) {
    return float3(-v.x, v.z, v.y);   // ← mirrors X
}
```

This maps:
| FluidX3D (Z-up) | → Bevy (Y-up)     | Effect           |
|------------------|--------------------|------------------|
| +X (right)       | −X (left)          | **mirrored** ✗   |
| +Y (depth)       | +Z (toward viewer) | reversed depth   |
| +Z (up)          | +Y (up)            | correct          |

Transform matrix determinant = +1 (handedness preserved), but the scene is **left-right mirrored** compared to the source.

**Fix** — use `(x, z, -y)`:

```cpp
inline float3 convert_to_yup(const float3& v) {
    return float3(v.x, v.z, -v.y);   // no mirror
}
```

| FluidX3D (Z-up) | → Bevy (Y-up)        | Effect    |
|------------------|----------------------|-----------|
| +X (right)       | +X (right)           | correct ✓ |
| +Y (depth)       | −Z (into screen)     | correct ✓ |
| +Z (up)          | +Y (up)              | correct ✓ |

Determinant = +1 (handedness preserved). No mirroring.

**Winding order**: Both transforms have det = +1, so triangle winding (CCW) is preserved. No vertex swap needed.

### 2. Moved: Coordinate Space Mismatch

The terrain STL loaded directly by Seaview is in **terrain-forge coordinates**:

```
origin = (0, 0, 0), extent = (511, 511, ~200)
```

But the fluid surface meshes are in **FluidX3D lattice coordinates** (or SI if `convert_to_si = true`):

```
origin = (0, 0, 0), extent = (Nx, Ny, Nz)     # lattice
origin = (0, 0, 0), extent = (Wx, Wy, Wz) m    # SI
```

The terrain was scaled and centered within the FluidX3D domain during voxelization:

```
terrain_forge_coords  →  scale(auto_fit)  →  translate(center)  →  rotate  →  lattice_coords
```

So the raw terrain STL and the fluid frames are at **different scales and positions**.

## Fix Options

### Option A: Bridge Script (recommended)

A Python script that reads the landscape TOML config + terrain STL, replicates the FluidX3D transform, and outputs a Y-up terrain mesh aligned with the fluid frames.

**Steps:**
1. Parse landscape TOML → extract `scale`, `center_x/y/z`, `rotation_x/y/z`, domain dims, `memory_mb`
2. Compute grid resolution (same formula as FluidX3D's `resolution()`)
3. Read terrain STL vertices
4. If `scale == 0.0`, compute auto-fit scale from mesh AABB vs domain
5. Apply: scale → center → rotation (matching FluidX3D's `voxelize_stl`)
6. Apply Y-up conversion: `(x, z, -y)`
7. If `convert_to_si`, apply SI unit conversion
8. Write `terrain_yup.stl`

**Pros:** No changes to FluidX3D core or terrain-forge. Scriptable, part of the sim pipeline.

**Cons:** Must replicate FluidX3D's `resolution()` and auto-fit logic exactly.

### Option B: FluidX3D Exports Placed Terrain

After `voxelize_stl`, FluidX3D writes the transformed terrain mesh (smooth, pre-voxelize) as an STL in the export directory, with the same Y-up + SI conversion as the fluid frames.

**Pros:** Perfect alignment guaranteed — same code path.

**Cons:** Requires FluidX3D code changes. Need to keep the original mesh vertices in memory post-voxelize (currently discarded).

### Option C: Seaview Applies Transform at Load Time

Seaview reads transform metadata (from the landscape TOML or a sidecar file) and applies it when loading environment objects.

**Pros:** No extra files, flexible.

**Cons:** Couples Seaview to FluidX3D config format. More complex env object loading.

## Recommended Plan

1. **Fix `convert_to_yup`** in `FluidX3D-fork/cases/include/surface_export.hpp`:
   `(-v.x, v.z, v.y)` → `(v.x, v.z, -v.y)`

2. **Implement Option A** (bridge script) as `FluidX3D-fork/cases/scripts/terrain_to_yup.py`
   or `seaview/scripts/terrain_to_yup.py` — reads TOML + STL, outputs aligned Y-up mesh.

3. **Re-export existing fluid frames** with the fixed `convert_to_yup` (or re-run sim).

4. **terrain-forge stays Z-up** — it's the natural convention for heightmap generation and FluidX3D consumption. No changes needed there.

## Reference: FluidX3D `resolution()` Formula

The grid resolution is computed from domain aspect ratio and VRAM budget. To replicate auto-fit in the bridge script, we need:

- Domain dims: `(width, depth, height)` from TOML `[domain]`
- `memory_mb` from TOML `[simulation]`
- `resolution()` returns `uint3` grid dimensions

The auto-fit scale in `voxelize_stl` scales the mesh AABB to fit within the grid dimensions, preserving aspect ratio.

## Files Involved

| File | Change |
|------|--------|
| `FluidX3D-fork/cases/include/surface_export.hpp` | Fix `convert_to_yup`: `(-x,z,y)` → `(x,z,-y)` |
| `FluidX3D-fork/cases/include/surface_export.hpp` | Update default normal for degenerate tris |
| New: bridge script | Transform terrain STL to match fluid coordinate space |
| `FluidX3D-fork/cases/assets/configs/*.toml` | Already has `convert_to_yup = true` ✓ |
| terrain-forge | No changes needed |