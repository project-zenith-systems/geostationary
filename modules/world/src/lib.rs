pub mod lifecycle;
pub mod loader;
pub mod map_file;

pub use lifecycle::{WorldLoading, WorldReady, WorldTeardown};
pub use loader::MapPath;
pub use map_file::{
    CURRENT_MAP_VERSION, MapFile, MapLayer, MapLayerRegistry, MapLayerRegistryExt,
    from_layer_value, to_layer_value,
};

use bevy::prelude::*;

/// L0 plugin that owns the map-file container, the `MapLayer` dispatch
/// registry, and lifecycle events.
///
/// Add this plugin before any module that registers a `MapLayer`.
///
/// If a [`MapPath`] resource is present when the app starts, the plugin's
/// [`loader::load_map`] startup system will read the `.station.ron` file and
/// dispatch each layer to its registered [`MapLayer`] implementation.
pub struct WorldPlugin;

impl Plugin for WorldPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MapLayerRegistry>();
        app.add_message::<WorldLoading>();
        app.add_message::<WorldReady>();
        app.add_message::<WorldTeardown>();
        // Map loading is driven by the application's Loading state
        // (see WorldInitPlugin), not by Startup.
    }
}
