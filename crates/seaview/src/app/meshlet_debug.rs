//! Meshlet cluster debug visualization material.
//!
//! An empty [`Material`] — Bevy's meshlet renderer automatically uses a
//! built-in fragment shader that colors each cluster a distinct color,
//! making cluster boundaries and LOD structure visible.
//!
//! Only compiled with `--features meshlet`.

use bevy::prelude::*;
use bevy::render::render_resource::AsBindGroup;

#[derive(Asset, TypePath, AsBindGroup, Clone, Default)]
pub struct MeshletDebugMaterial {}

impl Material for MeshletDebugMaterial {}
