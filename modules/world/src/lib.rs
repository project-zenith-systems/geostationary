pub mod lifecycle;
pub mod map_file;

pub use lifecycle::{WorldLoading, WorldReady, WorldTeardown};
pub use map_file::{MapFile, MapLayer, MapLayerRegistry, MapLayerRegistryExt, from_layer_value, to_layer_value};

use bevy::prelude::*;

/// L0 plugin that owns the map-file container, the `MapLayer` dispatch
/// registry, and lifecycle events.
///
/// Add this plugin before any module that registers a `MapLayer`.
pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MapLayerRegistry>();
        app.add_message::<WorldLoading>();
        app.add_message::<WorldReady>();
        app.add_message::<WorldTeardown>();
    }
}
