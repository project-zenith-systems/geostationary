use std::collections::BTreeMap;

use bevy::prelude::*;
use ron::value::RawValue;
use serde::{Deserialize, Serialize};

/// On-disk representation of a station map.
///
/// `MapFile` is a pure data container: it holds a version number and a map of
/// raw RON blobs keyed by layer name. Deserializing a `MapFile` and then
/// re-serializing it preserves **all** layer entries verbatim, including
/// layers whose keys are not registered with any [`MapLayer`] implementation.
/// This means an older build that does not know about a `"structures"` layer
/// can still load the file, leave that blob untouched, and write it back out
/// without data loss.
///
/// Higher-level helpers like [`MapLayerRegistry::save_all`] construct a
/// **new** `MapFile` that only contains registered layers. Callers that need
/// to preserve unknown layers must merge an existing `MapFile`'s
/// `layers` map with the output of `save_all` before writing to disk.
///
/// # Why `Box<RawValue>` rather than `ron::Value`?
///
/// Spike 1 (Q1) found that `ron::Value` is a lossy representation: unit enum
/// variants like `floor` or `Wall` parse to `Value::Unit` and lose their
/// name. This makes `into_rust::<TileDef>()` fail because the enum variant
/// can no longer be identified. `RawValue` stores the raw RON bytes verbatim
/// and is therefore exact for all types, including those with unit enum
/// variants. See [`from_layer_value`] and the spike tests for details.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MapFile {
    pub version: u32,
    pub layers: BTreeMap<String, Box<RawValue>>,
}

impl MapFile {
    pub fn new(version: u32) -> Self {
        Self {
            version,
            layers: BTreeMap::new(),
        }
    }
}

/// The current on-disk format version written by [`MapLayerRegistry::save_all`].
/// Bump this constant when making breaking changes to the map file format.
pub const CURRENT_MAP_VERSION: u32 = 1;

/// A named section of a map file.
///
/// Each module that contributes map data implements this trait and registers
/// it during plugin `build()` via [`MapLayerRegistryExt::register_map_layer`].
///
/// ## Spike findings (Spike 1)
///
/// **Q3 – context for `load()`:** `&mut World` is sufficient and flexible
/// enough for all current layers. It allows direct access to resources such
/// as `ThingRegistry` (`world.resource::<ThingRegistry>()`) and lets the
/// implementor insert, remove, or modify any resource or component without
/// an intermediate `Commands` buffer. The tradeoff is that observers and
/// hooks do not fire during a direct world mutation (they would via
/// `Commands`), but map loading happens before simulation starts so this is
/// acceptable.
///
/// **Q4 – shared save() path:** A single `save(&self, world: &World)`
/// signature works for both the running server and the editor because both
/// run inside a Bevy `App` and the layers' state lives in the same `World`.
/// The server queries its live entities; the editor queries its edit-time
/// entities. No separate serialization path is needed.
pub trait MapLayer: Send + Sync + 'static {
    /// Unique key for this layer in the map file (e.g., `"tiles"`, `"spawns"`).
    fn key(&self) -> &'static str;

    /// Serialize this module's world state into a raw RON value.
    ///
    /// The returned value is stored verbatim under [`key()`](MapLayer::key)
    /// in [`MapFile::layers`]. Use [`to_layer_value`] as a helper.
    fn save(
        &self,
        world: &World,
    ) -> Result<Box<RawValue>, Box<dyn std::error::Error + Send + Sync>>;

    /// Deserialize `data` and apply it to `world`.
    ///
    /// `world` provides direct access to all resources and entities, which
    /// is sufficient for the `tiles` layer (insert `TileGrid` resource) and
    /// the `things` layer (read `ThingRegistry`, spawn entities).
    /// Use [`from_layer_value`] as a helper.
    fn load(
        &self,
        data: &RawValue,
        world: &mut World,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;

    /// Remove all resources and entities that [`load`](MapLayer::load) inserted.
    ///
    /// Called in two situations:
    /// 1. **Rollback** — when a later layer fails during [`MapLayerRegistry::load_all`],
    ///    previously loaded layers are unloaded in reverse order so the world
    ///    does not contain partial state.
    /// 2. **Teardown** — when the world is being torn down (e.g. leaving InGame).
    fn unload(&self, world: &mut World);
}

/// Holds all registered [`MapLayer`] implementations.
///
/// Populated during plugin `build()` via
/// [`MapLayerRegistryExt::register_map_layer`].
#[derive(Resource, Default)]
pub struct MapLayerRegistry {
    pub(crate) layers: Vec<Box<dyn MapLayer>>,
}

impl MapLayerRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register(&mut self, layer: impl MapLayer) {
        let key = layer.key();
        assert!(
            !self.layers.iter().any(|l| l.key() == key),
            "duplicate MapLayer key: \"{key}\" is already registered"
        );
        self.layers.push(Box::new(layer));
    }

    /// Save all registered layers from `world` into a new [`MapFile`].
    ///
    /// The returned file uses [`CURRENT_MAP_VERSION`] as its version number.
    /// Layers not registered with this registry are **not** included — callers
    /// that need round-trip fidelity for unknown layers should start from an
    /// existing [`MapFile`] and merge updated layers into its `layers` map
    /// rather than discarding and replacing it.
    pub fn save_all(
        &self,
        world: &World,
    ) -> Result<MapFile, Box<dyn std::error::Error + Send + Sync>> {
        let mut file = MapFile::new(CURRENT_MAP_VERSION);
        for layer in &self.layers {
            let value = layer.save(world)?;
            file.layers.insert(layer.key().to_owned(), value);
        }
        Ok(file)
    }

    /// Load each registered layer that is present in `file`, in registration
    /// order.
    ///
    /// Iterates registered layers (not file keys) and looks up each layer's
    /// [`key()`](MapLayer::key) in the file. Layers present in the file but
    /// not registered are silently skipped and remain in `file.layers`
    /// untouched. Registration order is therefore the effective load order,
    /// which matters when one layer depends on resources inserted by an
    /// earlier layer.
    pub fn load_all(
        &self,
        file: &MapFile,
        world: &mut World,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let mut loaded: Vec<usize> = Vec::new();
        for (i, layer) in self.layers.iter().enumerate() {
            if let Some(data) = file.layers.get(layer.key()) {
                if let Err(e) = layer.load(data, world) {
                    // Rollback previously loaded layers in reverse order.
                    for &j in loaded.iter().rev() {
                        self.layers[j].unload(world);
                    }
                    return Err(e);
                }
                loaded.push(i);
            }
        }
        Ok(())
    }

    /// Unload all registered layers in reverse registration order.
    ///
    /// Used during world teardown to cleanly remove all layer state.
    pub fn unload_all(&self, world: &mut World) {
        for layer in self.layers.iter().rev() {
            layer.unload(world);
        }
    }

    /// Load a single registered layer by its key.
    ///
    /// Returns `Ok(true)` if the layer was found and loaded successfully,
    /// `Ok(false)` if no layer with that key is registered, or `Err` if
    /// loading failed.
    pub fn load_layer(
        &self,
        key: &str,
        data: &RawValue,
        world: &mut World,
    ) -> Result<bool, Box<dyn std::error::Error + Send + Sync>> {
        for layer in &self.layers {
            if layer.key() == key {
                layer.load(data, world)?;
                return Ok(true);
            }
        }
        Ok(false)
    }
}

/// Extension trait for [`App`] that provides `register_map_layer`.
pub trait MapLayerRegistryExt {
    fn register_map_layer(&mut self, layer: impl MapLayer) -> &mut Self;
}

impl MapLayerRegistryExt for App {
    fn register_map_layer(&mut self, layer: impl MapLayer) -> &mut Self {
        let mut registry = self
            .world_mut()
            .get_resource_or_insert_with::<MapLayerRegistry>(MapLayerRegistry::default);
        registry.register(layer);
        self
    }
}

/// Serialize a concrete layer data type `T` into a [`Box<RawValue>`] for use
/// in [`MapLayer::save`].
pub fn to_layer_value<T: serde::Serialize>(value: &T) -> Result<Box<RawValue>, ron::Error> {
    let pretty = ron::ser::PrettyConfig::default().depth_limit(2);
    let ron_str = ron::ser::to_string_pretty(value, pretty)?;
    // ron_str was just produced by ron's own serializer, so it's valid RON.
    Ok(RawValue::from_boxed_ron(ron_str.into_boxed_str())
        .expect("ron serializer produced invalid RON"))
}

/// Deserialize a [`RawValue`] into a concrete layer data type `T` for use in
/// [`MapLayer::load`].
///
/// Unlike `ron::Value::into_rust`, this preserves unit enum variant names
/// (e.g., `floor`, `Wall`) because `RawValue` stores the raw RON bytes.
///
/// Spike finding Q1: `ron::Value` loses unit enum variant names — `Floor`
/// and `Wall` both become `Value::Unit`. Use this function (backed by
/// `RawValue`) instead to correctly reconstruct any deserializable type.
pub fn from_layer_value<T: serde::de::DeserializeOwned>(
    raw: &RawValue,
) -> Result<T, ron::error::SpannedError> {
    raw.into_rust::<T>()
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use bevy::prelude::*;
    use serde::{Deserialize, Serialize};

    use super::*;

    // ---------------------------------------------------------------------------
    // Minimal concrete layer types used across the spike tests.
    // These mirror the shapes defined in docs/map-format.md and will be moved
    // into modules/tiles and modules/things when those layers are implemented.
    // ---------------------------------------------------------------------------

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum TileKind {
        Floor,
        Wall,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    #[serde(rename_all = "snake_case")]
    enum Atmo {
        Pressurised,
        Vacuum,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct TileDef {
        pub kind: TileKind,
        #[serde(default)]
        pub atmosphere: Option<Atmo>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct TilesLayerData {
        pub chunk_size: u32,
        pub keys: BTreeMap<u16, TileDef>,
        pub chunks: BTreeMap<(i32, i32), String>,
    }

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct SpawnPoint {
        pub position: [f32; 3],
        pub template: String,
        #[serde(default)]
        pub contents: Vec<String>,
    }

    // ---------------------------------------------------------------------------
    // Spike Q1: Can ron::Value be deserialized into a concrete type in a
    //           second pass?
    //
    // Finding: NOT via ron::Value — it loses unit enum variant names (e.g.,
    // TileKind::Floor serializes as the identifier `floor`, which parses to
    // Value::Unit with the name discarded). Use ron::value::RawValue instead:
    // it stores the raw RON bytes verbatim, and RawValue::into_rust::<T>()
    // reconstructs any deserializable type including those with unit enum
    // variants. All layer storage uses Box<RawValue> for this reason.
    // ---------------------------------------------------------------------------

    /// A `MapFile` is first parsed from RON (all layer values land as
    /// `Box<RawValue>`).  The individual values are then deserialized into
    /// their concrete layer types via `from_layer_value`.  Both must succeed
    /// and the round-tripped data must equal the original.
    #[test]
    fn spike_q1_raw_value_to_concrete_type_second_pass() {
        let tiles_data = TilesLayerData {
            chunk_size: 32,
            keys: {
                let mut m = BTreeMap::new();
                m.insert(
                    0,
                    TileDef {
                        kind: TileKind::Wall,
                        atmosphere: None,
                    },
                );
                m.insert(
                    1,
                    TileDef {
                        kind: TileKind::Floor,
                        atmosphere: None,
                    },
                );
                m.insert(
                    2,
                    TileDef {
                        kind: TileKind::Floor,
                        atmosphere: Some(Atmo::Vacuum),
                    },
                );
                m
            },
            chunks: {
                let mut m = BTreeMap::new();
                m.insert((0, 0), "AAAA".to_owned());
                m
            },
        };

        let spawns: Vec<SpawnPoint> = vec![
            SpawnPoint {
                position: [5.0, 0.0, 3.0],
                template: "toolbox".to_owned(),
                contents: vec!["can".to_owned()],
            },
            SpawnPoint {
                position: [4.0, 0.0, 3.0],
                template: "can".to_owned(),
                contents: vec![],
            },
        ];

        // Build a MapFile with serialized layer values.
        let mut file = MapFile::new(1);
        file.layers
            .insert("tiles".to_owned(), to_layer_value(&tiles_data).unwrap());
        file.layers
            .insert("spawns".to_owned(), to_layer_value(&spawns).unwrap());

        // Serialize the whole file and re-parse it to simulate loading from disk.
        let file_ron = ron::to_string(&file).expect("MapFile serialize");
        let loaded: MapFile = ron::from_str(&file_ron).expect("MapFile deserialize");

        // Second-pass: deserialize each RawValue into its concrete type.
        let loaded_tiles: TilesLayerData =
            from_layer_value(loaded.layers.get("tiles").unwrap()).expect("tiles second-pass");
        let loaded_spawns: Vec<SpawnPoint> =
            from_layer_value(loaded.layers.get("spawns").unwrap()).expect("spawns second-pass");

        assert_eq!(
            loaded_tiles, tiles_data,
            "TilesLayerData must survive the round-trip"
        );
        assert_eq!(
            loaded_spawns, spawns,
            "Vec<SpawnPoint> must survive the round-trip"
        );
    }

    // ---------------------------------------------------------------------------
    // Spike Q1 (supplemental): Demonstrate WHY ron::Value cannot be used.
    //
    // ron::Value parses unit enum variant identifiers (e.g., `floor`) as
    // Value::Unit, discarding the variant name. Attempting to reconstruct
    // TileKind from Value::Unit fails with InvalidValueForType. This test
    // documents the limitation so future contributors understand why
    // Box<RawValue> is used in MapFile instead of ron::Value.
    // ---------------------------------------------------------------------------

    #[test]
    fn spike_q1_ron_value_loses_unit_enum_variant_name() {
        let tile = TileDef {
            kind: TileKind::Floor,
            atmosphere: None,
        };

        // Serialize to RON string: "(kind:floor)"
        let ron_str = ron::to_string(&tile).unwrap();

        // Parse to ron::Value — the identifier `floor` becomes Value::Unit.
        let value: ron::Value = ron::from_str(&ron_str).unwrap();
        let map = match &value {
            ron::Value::Map(m) => m,
            other => panic!("expected Map, got {other:?}"),
        };
        let kind_key = ron::Value::String("kind".to_owned());
        let kind_value = map.get(&kind_key).expect("kind key must exist");
        assert_eq!(
            *kind_value,
            ron::Value::Unit,
            "ron::Value stores the floor identifier as Value::Unit, losing its name"
        );

        // Attempting into_rust fails because Value::Unit cannot be matched
        // to any TileKind variant.
        let result = value.into_rust::<TileDef>();
        assert!(
            result.is_err(),
            "into_rust must fail because enum variant name is lost in ron::Value"
        );
    }

    // ---------------------------------------------------------------------------
    // Spike Q2: Does ron::Value round-trip with fidelity for unknown layers?
    //
    // Finding: YES for Box<RawValue>. Unknown layers are stored as raw RON
    // bytes, and Box<RawValue> round-trips with exact syntactic fidelity.
    // ---------------------------------------------------------------------------

    /// A map file that contains a layer the current build does not know about
    /// (`"structures"`) must re-save that layer's value unchanged so that a
    /// newer build can still load it.
    #[test]
    fn spike_q2_unknown_layer_round_trips() {
        // Original file written by a newer build with a "structures" layer.
        let original_ron = r#"(
            version: 1,
            layers: {
                "tiles": {},
                "structures": [
                    (id: "airlock", position: [3, 5]),
                    (id: "window", position: [7, 2]),
                ],
            },
        )"#;

        let file: MapFile = ron::from_str(original_ron).expect("parse original");

        // The current build only knows about "tiles"; "structures" is unknown.
        // Re-serialize — the unknown layer must survive verbatim.
        let reserialized = ron::to_string(&file).expect("serialize");
        let reloaded: MapFile = ron::from_str(&reserialized).expect("reload");

        assert!(
            reloaded.layers.contains_key("structures"),
            "unknown 'structures' layer must survive the round-trip"
        );
        assert_eq!(
            reloaded.layers.get("structures").map(|v| v.get_ron()),
            file.layers.get("structures").map(|v| v.get_ron()),
            "structures layer raw value must be identical after round-trip"
        );
        assert_eq!(
            reloaded.version, 1,
            "version field must survive the round-trip"
        );
    }

    // ---------------------------------------------------------------------------
    // Spike Q3: What context does MapLayer::load() need?
    //
    // Finding: &mut World is sufficient. It allows reading existing resources
    // (e.g. ThingRegistry) and inserting new ones (e.g. TileGrid). Commands
    // are not needed because simulation hasn't started — no observers need to
    // fire.  Deferred Commands would require an extra `world.flush()` call and
    // add complexity without benefit at load time.
    // ---------------------------------------------------------------------------

    /// A toy resource written by one layer and read by a second layer,
    /// proving that `&mut World` is rich enough for inter-layer dependencies.
    #[derive(Resource)]
    struct FooRegistry {
        items: Vec<String>,
    }

    struct FooLayer;
    impl MapLayer for FooLayer {
        fn key(&self) -> &'static str {
            "foo"
        }

        fn save(
            &self,
            world: &World,
        ) -> Result<Box<RawValue>, Box<dyn std::error::Error + Send + Sync>> {
            let names: Vec<String> = world
                .get_resource::<FooRegistry>()
                .map(|r| r.items.clone())
                .unwrap_or_default();
            Ok(to_layer_value(&names)?)
        }

        fn load(
            &self,
            data: &RawValue,
            world: &mut World,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            let names: Vec<String> = from_layer_value(data)?;
            world.insert_resource(FooRegistry { items: names });
            Ok(())
        }

        fn unload(&self, world: &mut World) {
            world.remove_resource::<FooRegistry>();
        }
    }

    struct BarLayer;
    impl MapLayer for BarLayer {
        fn key(&self) -> &'static str {
            "bar"
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
            // Read the FooRegistry resource that FooLayer inserted before us.
            let foo = world
                .get_resource::<FooRegistry>()
                .ok_or("FooRegistry not yet loaded")?;
            assert_eq!(foo.items, vec!["alpha", "beta"]);
            Ok(())
        }

        fn unload(&self, _world: &mut World) {}
    }

    #[test]
    fn spike_q3_load_receives_mut_world_with_resource_access() {
        let mut world = World::new();

        // Build a minimal file for the two layers.
        let mut file = MapFile::new(1);
        file.layers.insert(
            "foo".to_owned(),
            to_layer_value(&vec!["alpha", "beta"]).unwrap(),
        );
        file.layers
            .insert("bar".to_owned(), to_layer_value(&()).unwrap());

        // Register layers in load order.
        let mut registry = MapLayerRegistry::new();
        registry.register(FooLayer);
        registry.register(BarLayer);

        // load_all must not error — BarLayer can read FooRegistry set by FooLayer.
        registry
            .load_all(&file, &mut world)
            .expect("load_all must succeed");

        let foo = world.resource::<FooRegistry>();
        assert_eq!(foo.items, vec!["alpha", "beta"]);
    }

    // ---------------------------------------------------------------------------
    // Spike Q4: Can the same MapLayer::save() path work for both the running
    //           server and the editor?
    //
    // Finding: Yes. `save(&self, world: &World)` reads whatever live state
    // the world holds — tile entities in the editor, or tile entities in the
    // running server. The implementation is identical; only the entities
    // differ. No separate serialization path is needed.
    // ---------------------------------------------------------------------------

    #[derive(Resource)]
    struct TileCount(u32);

    struct CountLayer;
    impl MapLayer for CountLayer {
        fn key(&self) -> &'static str {
            "count"
        }

        fn save(
            &self,
            world: &World,
        ) -> Result<Box<RawValue>, Box<dyn std::error::Error + Send + Sync>> {
            let n = world.get_resource::<TileCount>().map(|r| r.0).unwrap_or(0);
            Ok(to_layer_value(&n)?)
        }

        fn load(
            &self,
            data: &RawValue,
            world: &mut World,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            let n: u32 = from_layer_value(data)?;
            world.insert_resource(TileCount(n));
            Ok(())
        }

        fn unload(&self, world: &mut World) {
            world.remove_resource::<TileCount>();
        }
    }

    /// Demonstrate that the same `save()` implementation works regardless of
    /// which "world context" (server vs editor) is calling it.
    #[test]
    fn spike_q4_same_save_path_for_server_and_editor() {
        let layer = CountLayer;
        let mut registry = MapLayerRegistry::new();
        registry.register(CountLayer);

        // Simulate a "server world" with one tile count.
        let mut server_world = World::new();
        server_world.insert_resource(TileCount(1024));
        let server_file = registry.save_all(&server_world).unwrap();
        let server_count: u32 = from_layer_value(server_file.layers.get("count").unwrap()).unwrap();
        assert_eq!(server_count, 1024);

        // Simulate an "editor world" with a different tile count.
        let mut editor_world = World::new();
        editor_world.insert_resource(TileCount(512));
        let editor_file = registry.save_all(&editor_world).unwrap();
        let editor_count: u32 = from_layer_value(editor_file.layers.get("count").unwrap()).unwrap();
        assert_eq!(editor_count, 512);

        // The same CountLayer.save() implementation produced both results —
        // no conditional logic or separate serialization paths.
        let server_ron = layer.save(&server_world).unwrap().get_ron().to_owned();
        let editor_ron = layer.save(&editor_world).unwrap().get_ron().to_owned();
        assert_ne!(
            server_ron, editor_ron,
            "different worlds produce different output"
        );
    }

    // ---------------------------------------------------------------------------
    // Spike Q5: Is atmosphere best kept in TileDef or as a separate overlay?
    //
    // Finding: Keep atmosphere in TileDef for this plan.
    //
    // Key-dictionary deduplication means a station with 32x32 tiles but only
    // three configurations (Wall, Pressurised floor, Vacuum floor) stores
    // exactly three dictionary entries — the chunk blob encodes only u16
    // indices. If atmosphere were a separate overlay grid, every floor tile
    // would need an entry regardless of uniqueness, growing the file and
    // complicating the loader.
    //
    // Atmosphere is also always co-read with tile kind at load time (the atmos
    // module reads TileDef to build GasGrid immediately after tiles load), so
    // there is no benefit to a separate layer at this scale.
    //
    // Future plans: if per-tile data grows (pipe overlays, connectables),
    // revisit the three options documented in docs/map-format.md:
    //   1. Flat extension with #[serde(default)] fields (current choice).
    //   2. Per-tile extras map (HashMap<String, Box<RawValue>>).
    //   3. Overlay grids (separate layers per module).
    // ---------------------------------------------------------------------------

    /// Verify that `TileDef` with and without `atmosphere` deserializes
    /// correctly when the field is optional (`#[serde(default)]`).
    #[test]
    fn spike_q5_atmosphere_in_tile_def_serde_default() {
        // Floor with no atmosphere field — defaults to None (Pressurised).
        let no_atmo: TileDef = ron::from_str("(kind: floor)").expect("no atmosphere field");
        assert_eq!(no_atmo.kind, TileKind::Floor);
        assert_eq!(no_atmo.atmosphere, None);

        // Floor explicitly marked Vacuum.
        let vacuum: TileDef =
            ron::from_str("(kind: floor, atmosphere: Some(vacuum))").expect("vacuum");
        assert_eq!(vacuum.atmosphere, Some(Atmo::Vacuum));

        // Wall with no atmosphere.
        let wall: TileDef = ron::from_str("(kind: wall)").expect("wall");
        assert_eq!(wall.kind, TileKind::Wall);
        assert_eq!(wall.atmosphere, None);

        // A full TilesLayerData with mixed keys round-trips through RawValue.
        let data = TilesLayerData {
            chunk_size: 32,
            keys: {
                let mut m = BTreeMap::new();
                m.insert(
                    0,
                    TileDef {
                        kind: TileKind::Wall,
                        atmosphere: None,
                    },
                );
                m.insert(
                    1,
                    TileDef {
                        kind: TileKind::Floor,
                        atmosphere: None,
                    },
                );
                m.insert(
                    2,
                    TileDef {
                        kind: TileKind::Floor,
                        atmosphere: Some(Atmo::Vacuum),
                    },
                );
                m
            },
            chunks: BTreeMap::new(),
        };
        let raw = to_layer_value(&data).unwrap();
        let loaded: TilesLayerData = from_layer_value(&raw).expect("deserialize");
        assert_eq!(data, loaded);
    }

    // ---------------------------------------------------------------------------
    // MapLayerRegistry::load_layer tests
    // ---------------------------------------------------------------------------

    struct DummyLayer {
        key: &'static str,
        load_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }

    impl MapLayer for DummyLayer {
        fn key(&self) -> &'static str {
            self.key
        }

        fn save(
            &self,
            _world: &World,
        ) -> Result<Box<RawValue>, Box<dyn std::error::Error + Send + Sync>> {
            to_layer_value(&()).map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>)
        }

        fn load(
            &self,
            _data: &RawValue,
            _world: &mut World,
        ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
            self.load_calls
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }

        fn unload(&self, _world: &mut World) {}
    }

    #[test]
    fn load_layer_returns_true_and_invokes_load_for_matching_key() {
        let load_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let layer = DummyLayer {
            key: "dummy",
            load_calls: load_calls.clone(),
        };

        let mut registry = MapLayerRegistry::new();
        registry.register(layer);

        let mut world = World::new();
        let data = to_layer_value(&()).expect("valid RON");

        let found = registry
            .load_layer("dummy", &data, &mut world)
            .expect("load_layer should not error");
        assert!(found, "expected true for matching key");
        assert_eq!(
            1,
            load_calls.load(std::sync::atomic::Ordering::SeqCst),
            "expected exactly one load() call"
        );
    }

    #[test]
    fn load_layer_returns_false_for_non_matching_key() {
        let load_calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let layer = DummyLayer {
            key: "dummy",
            load_calls: load_calls.clone(),
        };

        let mut registry = MapLayerRegistry::new();
        registry.register(layer);

        let mut world = World::new();
        let data = to_layer_value(&()).expect("valid RON");

        let not_found = registry
            .load_layer("other", &data, &mut world)
            .expect("load_layer should not error");
        assert!(!not_found, "expected false for non-matching key");
        assert_eq!(
            0,
            load_calls.load(std::sync::atomic::Ordering::SeqCst),
            "expected load() not to be called"
        );
    }
}
