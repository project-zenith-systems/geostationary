use bevy::prelude::*;

use crate::lifecycle::{WorldLoading, WorldReady};
use crate::map_file::{MapFile, MapLayerRegistry};

/// Resource that specifies the path to the `.station.ron` map file to load on
/// startup.
///
/// Insert this resource before the app starts so [`load_map`] can locate the
/// map file. If the resource is absent when [`load_map`] runs, loading is
/// skipped and a warning is logged — this allows the editor to run without a
/// pre-existing map file.
#[derive(Resource, Debug, Clone)]
pub struct MapPath(pub String);

impl MapPath {
    /// Create a new `MapPath` from any string-like value.
    pub fn new(path: impl Into<String>) -> Self {
        Self(path.into())
    }
}

/// Exclusive startup system that reads the `.station.ron` map file and
/// dispatches its layers to all registered [`crate::MapLayer`] implementations.
///
/// # Lifecycle
///
/// 1. Reads [`MapPath`] to locate the map file on disk. If absent, logs a
///    warning and returns early — the app continues in an uninitialised state.
/// 2. Reads and parses the RON map file into a [`MapFile`].
/// 3. Writes [`WorldLoading`] so that systems observing the start of loading
///    can prepare their resources.
/// 4. Removes [`MapLayerRegistry`] from the world, calls
///    [`MapLayerRegistry::load_all`], and re-inserts the registry. Removing it
///    first allows individual [`crate::MapLayer::load`] implementations to also
///    borrow the world mutably without conflicting with the registry.
/// 5. On success writes [`WorldReady`]; on failure logs the error.
///
/// This system runs at [`Startup`] and is registered by [`crate::WorldPlugin`].
pub fn load_map(world: &mut World) {
    let Some(map_path) = world.get_resource::<MapPath>().map(|r| r.0.clone()) else {
        warn!("WorldPlugin: no MapPath resource found, skipping map load");
        return;
    };

    let contents = match std::fs::read_to_string(&map_path) {
        Ok(c) => c,
        Err(e) => {
            error!(
                "WorldPlugin: failed to read map file {:?}: {}",
                map_path, e
            );
            return;
        }
    };

    let file: MapFile = match ron::from_str(&contents) {
        Ok(f) => f,
        Err(e) => {
            error!(
                "WorldPlugin: failed to parse map file {:?}: {}",
                map_path, e
            );
            return;
        }
    };

    world.write_message(WorldLoading);

    // Remove the registry temporarily so that MapLayer::load() implementations
    // can receive &mut World without a conflicting borrow of MapLayerRegistry.
    let registry = world
        .remove_resource::<MapLayerRegistry>()
        .expect("MapLayerRegistry must be present; add WorldPlugin before plugins that register layers");

    let result = registry.load_all(&file, world);

    world.insert_resource(registry);

    match result {
        Ok(()) => {
            info!("WorldPlugin: map loaded from {:?}", map_path);
            world.write_message(WorldReady);
        }
        Err(e) => {
            error!(
                "WorldPlugin: layer loading failed for {:?}: {}",
                map_path, e
            );
        }
    }
}
