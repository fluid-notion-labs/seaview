# Bobbing Figure on Ocean Surface

Depends on: `seaview-n3o.3` (SpatialQuery trait)

## Concept

Static GLB figure (scarecrow body + oversized head) positioned on the wave surface each frame via `project_point(x, z)` → wave height + surface normal. Orient figure to normal, done.

## Face-Mapped Head

Generate with **Meshy** (meshy.ai):

| Approach | How | Effort | Comedy |
|----------|-----|--------|--------|
| Image to 3D | Upload face photo → Meshy generates 3D head with likeness baked in | Low | High (uncanny valley IS the joke) |
| Flat cutout | Textured quad with face photo, parented to neck like a cardboard mask | Minimal | Maximum |
| UV swap | Generate head in Meshy, replace face texture in Blender with photo | Medium | Medium |

Recommendation: **Image to 3D** or **flat cutout**. Both export GLB, loader already handles it.

## Runtime Stack

1. Load scarecrow GLB as a static entity (separate from sequence meshes)
2. Each frame: `spatial_query.project_point(figure_pos.xz)` → surface point + normal
3. Set figure transform: translation = surface point, rotation = align Y to normal
4. Brute-force spatial query is fine — single query/frame, even 760K tris is ~5-15ms