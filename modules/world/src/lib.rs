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
/// registry, lifecycle events, and state-driven map loading.
///
/// Add this plugin before any module that registers a `MapLayer`.
///
/// Map loading runs on `OnEnter(loading)` via [`loader::load_map`].  If a
/// [`MapPath`] resource is present at that point, the `.station.ron` file is
/// read and each layer dispatched to its registered [`MapLayer`]
/// implementation.  If absent, loading is skipped (e.g. pure-client joins).
///
/// [`MapPath`] is cleaned up on `OnExit(in_game)` so a subsequent host
/// session picks up fresh config.
pub struct WorldPlugin<S: States + Copy> {
    pub loading: S,
    pub in_game: S,
}

impl<S: States + Copy> Plugin for WorldPlugin<S> {
    fn build(&self, app: &mut App) {
        app.init_resource::<MapLayerRegistry>();
        app.add_message::<WorldLoading>();
        app.add_message::<WorldReady>();
        app.add_message::<WorldTeardown>();
        app.add_systems(OnEnter(self.loading), loader::load_map);
        app.add_systems(OnExit(self.in_game), cleanup_map_path);
    }
}

fn cleanup_map_path(mut commands: Commands) {
    commands.remove_resource::<MapPath>();
}
