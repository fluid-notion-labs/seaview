# Lighting Units Reference

Quick reference for photometric units used in Seaview's lighting system.

## Core Units

### Lumens (lm) — Total Light Output
**What it measures:** Total luminous flux emitted in all directions.

- **Analogy:** The total amount of light "sprayed" by a source
- **Example:** A 60W incandescent bulb emits ~800 lumens
- **Usage:** Describes the total light power of a bulb or fixture
- **Bevy PBR:** Native unit for `SpotLight` and `PointLight` intensity

### Candela (cd) — Directional Intensity
**What it measures:** Luminous intensity in a specific direction.

- **Analogy:** How "bright" the source appears when looking at it from one direction
- **Example:** A standard candle emits ~1 candela
- **Usage:** Describes focused/directional lights (spotlights, laser beams)
- **Relationship to lumens:** Depends on beam spread

### Lux (lx) — Illuminance
**What it measures:** Luminous flux per unit area (lumens/m²).

- **Analogy:** How "lit up" a surface is
- **Example:** Full daylight ≈ 10,000 lux; office lighting ≈ 500 lux
- **Usage:** Measures light falling on a surface
- **Bevy PBR:** Used for `DirectionalLight` illuminance

## Relationship Between Lumens and Candela

For a **spotlight with a cone**:

```
Lumens = Candela × Solid Angle (steradians)
```

**Solid angle (Ω)** for a cone with full angle θ:
```
Ω = 2π(1 - cos(θ/2))
```

### Examples

| Cone Angle | Solid Angle (Ω) | Candela → Lumens |
|------------|-----------------|------------------|
| 30°        | 0.214 sr        | cd × 0.214       |
| 60°        | 0.842 sr        | cd × 0.842       |
| 90°        | 1.840 sr        | cd × 1.840       |
| 180°       | 6.283 sr (2π)   | cd × 6.283       |

**Key insight:** A narrow beam (small cone) has fewer lumens per candela than a wide beam.

## Practical Context

### Theater/Stage Lighting
- **ETC Source Four 750W spotlight:** 18,000 lumens, ~45,000 cd (with 19° lens)
- **Moving head spotlight:** 10,000-100,000 lumens depending on fixture

### Architectural/Outdoor
- **Stadium flood light:** 40,000-100,000 lumens
- **Street light (LED):** 5,000-15,000 lumens

### Seaview Defaults
- **Default intensity:** 800,000 lumens (800 klm) — very bright, suitable for high-altitude flood lighting
- **Equivalent candela (at 60° cone):** ~950,000 cd
- **Use case:** Simulating aerial/nautical search lights or stadium-scale flood lighting over fluid simulation domains

## Why Bevy Uses Lumens

Modern physically-based rendering (PBR) works with **radiometric units** (energy) converted to **photometric units** (perceived brightness). Lumens represent the total light energy weighted by human eye sensitivity, making them the natural choice for:

1. **Energy conservation** in path tracing
2. **Physical accuracy** in light transport
3. **Hardware compatibility** with real-world light meters

For spotlights, Bevy multiplies lumens by the cone's solid angle internally to compute the directional intensity used in shading calculations.

## Light Range (Attenuation Distance)

The **range** parameter controls how far light travels before it attenuates (fades) to near-zero.

### What it does:
- **NOT a hard cutoff** — light doesn't stop abruptly at the range value
- Controls the **falloff curve** — physically-based inverse-square law with a smooth fade-out
- Affects **performance** — larger range = more pixels/objects affected = slower rendering

### Physical behavior:
In Bevy's PBR renderer, light intensity follows the inverse-square law:

```
I(d) = I₀ / (d² + 1)
```

The `range` parameter modifies this with a windowing function that smoothly fades the light to zero as distance approaches `range`, preventing the light from affecting geometry infinitely far away (which would be expensive).

### Practical values:
- **Small interior lights:** 10-50m
- **Street lights:** 50-100m
- **Stadium/flood lights:** 200-500m
- **Searchlights/aircraft landing lights:** 1000-5000m

### Seaview default:
- **500m range** — suitable for high-altitude flood lighting over large fluid simulation domains
- Balance between physical realism and performance

### Rule of thumb:
Set `range` to approximately **2-3× the distance** from the light to the furthest geometry you want illuminated. Beyond that distance, the light will smoothly fade to negligible contribution.

## Further Reading

- [Photometry (Wikipedia)](https://en.wikipedia.org/wiki/Photometry_(optics))
- [Luminous intensity vs luminous flux](https://en.wikipedia.org/wiki/Luminous_intensity)
- [Solid angle calculator](https://en.wikipedia.org/wiki/Solid_angle#Cone,_spherical_cap,_hemisphere)