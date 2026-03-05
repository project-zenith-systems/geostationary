use bevy::prelude::*;

use crate::lifecycle::{WorldLoading, WorldReady};
use crate::map_file::{CURRENT_MAP_VERSION, MapFile, MapLayerRegistry};

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
/// 3. Validates that `file.version <= CURRENT_MAP_VERSION`. If the file was
///    written by a newer build (unknown version), loading is aborted to avoid
///    silent misloads.
/// 4. Writes [`WorldLoading`] so that systems observing the start of loading
///    can prepare their resources.
/// 5. Calls [`MapLayerRegistry::load_all`] via [`World::resource_scope`] so
///    that [`crate::MapLayer::load`] implementations can borrow `&mut World`
///    without conflicting with the registry borrow.
/// 6. On success writes [`WorldReady`]; on failure logs the error.
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

    if file.version > CURRENT_MAP_VERSION {
        error!(
            "WorldPlugin: map file {:?} has version {} which is newer than the \
             supported version {}; refusing to load to avoid silent misloads",
            map_path, file.version, CURRENT_MAP_VERSION
        );
        return;
    }

    world.write_message(WorldLoading);

    let result = world.resource_scope(|world, registry: Mut<MapLayerRegistry>| {
        registry.load_all(&file, world)
    });

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

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};

    use bevy::prelude::*;
    use ron::value::RawValue;

    use super::*;
    use crate::map_file::{MapLayer, MapLayerRegistry, to_layer_value};

    // ---------------------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------------------

    /// Counter used to generate unique temp file names so parallel tests do not
    /// collide.
    static FILE_COUNTER: AtomicU64 = AtomicU64::new(0);

    /// RAII guard that deletes its temp file when dropped.
    struct TempFile(PathBuf);

    impl TempFile {
        /// Write `contents` to a unique file in the system temp directory and
        /// return a guard that deletes the file when dropped.
        fn new(contents: &str) -> Self {
            let n = FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir()
                .join(format!("loader_test_{}.station.ron", n));
            std::fs::write(&path, contents).expect("write temp map file");
            TempFile(path)
        }

        /// Return a path string suitable for inserting into [`MapPath`].
        fn path_str(&self) -> String {
            self.0.to_string_lossy().into_owned()
        }

        /// Return a path string that is guaranteed not to exist on disk by
        /// using the counter-based name but *not* creating the file.
        fn nonexistent_path() -> String {
            let n = FILE_COUNTER.fetch_add(1, Ordering::Relaxed);
            std::env::temp_dir()
                .join(format!("loader_test_{}_missing.station.ron", n))
                .to_string_lossy()
                .into_owned()
        }
    }

    impl Drop for TempFile {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    /// Build a minimal [`World`] with the resources that [`load_map`] requires:
    /// [`MapLayerRegistry`], [`Messages<WorldLoading>`], and
    /// [`Messages<WorldReady>`].
    fn make_world() -> World {
        let mut world = World::new();
        world.init_resource::<MapLayerRegistry>();
        world.init_resource::<Messages<WorldLoading>>();
        world.init_resource::<Messages<WorldReady>>();
        world
    }

    /// Return the number of [`WorldReady`] messages that have been written to
    /// `world` since it was created.
    fn world_ready_count(world: &World) -> usize {
        world.resource::<Messages<WorldReady>>().len()
    }

    // ---------------------------------------------------------------------------
    // Stub MapLayer used in success-path tests.
    // ---------------------------------------------------------------------------

    /// Marker resource inserted by [`StubLayer::load`].
    #[derive(Resource)]
    struct StubLoaded;

    struct StubLayer;

    impl MapLayer for StubLayer {
        fn key(&self) -> &'static str {
            "stub"
        }

        fn save(
            &self,
            _world: &World,
        ) -> Result<Box<RawValue>, Box<dyn std::error::Error + Send + Sync>> {
            Ok(to_layer_value(&())?)
        }

        fn load(
            &self,
            _data: &RawValue,
            world: &mut World,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            world.insert_resource(StubLoaded);
            Ok(())
        }
    }

    // ---------------------------------------------------------------------------
    // Tests
    // ---------------------------------------------------------------------------

    /// When no [`MapPath`] resource is present, [`load_map`] skips loading and
    /// does not emit [`WorldReady`].
    #[test]
    fn load_map_skips_without_map_path() {
        let mut world = make_world();
        load_map(&mut world);
        assert_eq!(world_ready_count(&world), 0);
    }

    /// When [`MapPath`] points at a non-existent file, [`load_map`] logs an
    /// error and does not emit [`WorldReady`].
    #[test]
    fn load_map_errors_on_missing_file() {
        let mut world = make_world();
        world.insert_resource(MapPath::new(TempFile::nonexistent_path()));
        load_map(&mut world);
        assert_eq!(world_ready_count(&world), 0);
    }

    /// When the map file contains invalid RON, [`load_map`] logs a parse error
    /// and does not emit [`WorldReady`].
    #[test]
    fn load_map_errors_on_bad_ron() {
        let file = TempFile::new("not valid ron {{{{");
        let mut world = make_world();
        world.insert_resource(MapPath::new(file.path_str()));
        load_map(&mut world);
        assert_eq!(world_ready_count(&world), 0);
    }

    /// When the map file declares a version newer than [`CURRENT_MAP_VERSION`],
    /// [`load_map`] refuses to load it and does not emit [`WorldReady`].
    #[test]
    fn load_map_errors_on_unsupported_version() {
        let file = TempFile::new("(version: 9999, layers: {})");
        let mut world = make_world();
        world.insert_resource(MapPath::new(file.path_str()));
        load_map(&mut world);
        assert_eq!(world_ready_count(&world), 0);
    }

    /// A valid map file with a registered layer causes the layer to be loaded
    /// and [`WorldReady`] to be emitted.
    #[test]
    fn load_map_dispatches_registered_layer_and_emits_world_ready() {
        let file = TempFile::new("(version: 1, layers: { \"stub\": () })");
        let mut world = make_world();
        world
            .get_resource_mut::<MapLayerRegistry>()
            .unwrap()
            .register(StubLayer);
        world.insert_resource(MapPath::new(file.path_str()));
        load_map(&mut world);
        assert_eq!(world_ready_count(&world), 1, "WorldReady must be emitted after successful load");
        assert!(
            world.contains_resource::<StubLoaded>(),
            "registered layer must have been called"
        );
    }

    /// A valid map file with no registered layers still emits [`WorldReady`].
    #[test]
    fn load_map_emits_world_ready_with_no_layers() {
        let file = TempFile::new("(version: 1, layers: {})");
        let mut world = make_world();
        world.insert_resource(MapPath::new(file.path_str()));
        load_map(&mut world);
        assert_eq!(world_ready_count(&world), 1, "WorldReady must be emitted even when no layers are registered");
    }
}
