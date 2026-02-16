# Water Material Reference (Seaview)

## Material Settings

| Property    | Value       | Notes                                    |
|-------------|-------------|------------------------------------------|
| Base Color  | `#1A6E8E`   | Deep blue-teal; sky reflections add rest |
| Metallic    | 0.0–0.05    | Water is dielectric, NOT metallic        |
| Roughness   | 0.02–0.05   | Very smooth; sharp reflections           |
| Reflectance | 0.5–0.7     | Strong Fresnel at glancing angles        |
| Opacity     | 0.90–0.95   | Slightly translucent                     |

## Lighting

| Light         | Color / Temp   | RGB Approx            | Notes                        |
|---------------|----------------|-----------------------|------------------------------|
| Direct sun    | 5500–6500K     | `(1.0, 0.95, 0.85)`  | Warm white, strong specular  |
| Sky ambient   | Cool blue      | `(0.53, 0.81, 0.92)` | Fill light from sky dome     |
| Skybox / HDRI | —              | —                     | Drives environment reflections; this is where water gets most of its color |

## Key Principles

- Water looks "plasticy" when **metallic is too high** — keep it near zero
- Water color comes from **environment reflections**, not base color
- **Low roughness** = mirror-like highlights from the sun
- **Fresnel effect** is critical — reflective at glancing angles, transparent when looking straight down