//! Custom Bevy asset loaders for mesh file formats.
//!
//! This module provides [`AssetLoader`](bevy::asset::AssetLoader) implementations
//! for file formats that Bevy does not support out of the box. Currently:
//!
//! - **STL** (binary and ASCII) via [`stl_loader::StlLoader`]
//!
//! glTF / GLB loading is handled by Bevy's built-in `bevy_gltf` crate and does
//! not need a custom loader here.
//!
//! # Usage
//!
//! Add [`AssetLoadersPlugin`] to your Bevy app. After that the asset server
//! will transparently load `.stl` files into [`Mesh`](bevy::mesh::Mesh) assets:
//!
//! ```ignore
//! app.add_plugins(AssetLoadersPlugin);
//!
//! // Later, in a system:
//! let handle: Handle<Mesh> = asset_server.load("my_model.stl");
//! ```

pub mod stl_loader;

use bevy::prelude::*;

pub use stl_loader::StlLoader;

/// Plugin that registers all custom asset loaders.
///
/// Currently registers:
/// - [`StlLoader`] for `.stl` files
pub struct AssetLoadersPlugin;

impl Plugin for AssetLoadersPlugin {
    fn build(&self, app: &mut App) {
        app.register_asset_loader(StlLoader);
        debug!("Registered custom asset loaders: STL");
    }
}
