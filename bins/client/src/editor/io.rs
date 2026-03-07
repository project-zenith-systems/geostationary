//! Save and load `.station.ron` map files in the editor.
//!
//! ## Save
//!
//! Queries the live editor world via [`MapLayerRegistry::save_all`], merges
//! with any unknown layers from the previously loaded file, and writes the
//! result to disk.
//!
//! ## Load
//!
//! Reads a `.station.ron` file, clears the editor world (tiles + spawn
//! markers), and rebuilds from the file data.  The `"tiles"` layer is loaded
//! via its [`MapLayer`] implementation (which inserts a [`TileGrid`] resource).
//! The `"spawns"` layer is parsed manually to create lightweight editor
//! markers instead of full physics entities.

use bevy::prelude::*;
use shared::config::AppConfig;
use things::{SpawnMarker, SpawnPoint, Thing, ThingRegistry};
use tiles::{AtmoSeed, GridSize, Tile, TileGrid, TileKind};
use world::{CURRENT_MAP_VERSION, MapFile, MapLayerRegistry, from_layer_value};

use super::spawns::{EditorSpawnMarker, SpawnMarkerAssets};

/// Resource that stores the last loaded [`MapFile`] so unknown layers are
/// preserved across save/load round-trips.
#[derive(Resource, Default)]
pub struct EditorMapFile(pub Option<MapFile>);

/// Event message requesting an editor save.
#[derive(Message, Clone, Debug)]
pub struct EditorSaveEvent;

/// Event message requesting an editor load.
#[derive(Message, Clone, Debug)]
pub struct EditorLoadEvent;

/// Exclusive system: handle save requests.
///
/// Uses [`MapLayerRegistry::save_all`] to serialize all registered layers,
/// merges unknown layers from the previously loaded file, and writes to disk.
pub fn handle_save(world: &mut World) {
    // Check if there's a save event pending.
    let has_event = world
        .resource_mut::<Messages<EditorSaveEvent>>()
        .drain()
        .next()
        .is_some();
    if !has_event {
        return;
    }

    let path = world.resource::<AppConfig>().world.map_path.clone();
    info!("Editor: saving map to {path}");

    let result =
        world.resource_scope(|world, registry: Mut<MapLayerRegistry>| registry.save_all(world));

    match result {
        Ok(mut file) => {
            // Merge unknown layers from the previously loaded file.
            if let Some(ref old_file) = world.resource::<EditorMapFile>().0 {
                for (key, value) in &old_file.layers {
                    if !file.layers.contains_key(key) {
                        file.layers.insert(key.clone(), value.clone());
                    }
                }
            }

            let pretty = ron::ser::PrettyConfig::default();
            match ron::ser::to_string_pretty(&file, pretty) {
                Ok(ron_str) => {
                    // Ensure parent directory exists.
                    if let Some(parent) = std::path::Path::new(&path).parent()
                        && let Err(e) = std::fs::create_dir_all(parent)
                    {
                        error!("Editor: failed to create directory {parent:?}: {e}");
                        return;
                    }
                    match std::fs::write(&path, &ron_str) {
                        Ok(()) => info!("Editor: map saved to {path}"),
                        Err(e) => error!("Editor: failed to write {path}: {e}"),
                    }
                }
                Err(e) => error!("Editor: failed to serialize map: {e}"),
            }
        }
        Err(e) => error!("Editor: save_all failed: {e}"),
    }
}

/// Exclusive system: handle load requests.
///
/// Reads the map file, clears the editor world, loads the tiles layer via
/// [`MapLayer::load`], and creates editor spawn markers for the spawns layer.
pub fn handle_load(world: &mut World) {
    // Check if there's a load event pending.
    let has_event = world
        .resource_mut::<Messages<EditorLoadEvent>>()
        .drain()
        .next()
        .is_some();
    if !has_event {
        return;
    }

    let path = world.resource::<AppConfig>().world.map_path.clone();
    info!("Editor: loading map from {path}");

    let contents = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => {
            error!("Editor: failed to read {path}: {e}");
            return;
        }
    };

    let file: MapFile = match ron::from_str(&contents) {
        Ok(f) => f,
        Err(e) => {
            error!("Editor: failed to parse {path}: {e}");
            return;
        }
    };

    if file.version > CURRENT_MAP_VERSION {
        error!(
            "Editor: map file version {} is newer than supported version {}",
            file.version, CURRENT_MAP_VERSION
        );
        return;
    }

    // Clear existing editor world: remove tile grid, despawn tiles and markers.
    world.remove_resource::<TileGrid<TileKind>>();
    world.remove_resource::<GridSize>();
    world.remove_resource::<AtmoSeed>();

    let tile_entities: Vec<Entity> = world
        .query_filtered::<Entity, With<Tile>>()
        .iter(world)
        .collect();
    for entity in tile_entities {
        world.despawn(entity);
    }

    let marker_entities: Vec<Entity> = world
        .query_filtered::<Entity, With<EditorSpawnMarker>>()
        .iter(world)
        .collect();
    for entity in marker_entities {
        world.despawn(entity);
    }

    // Load the tiles layer via the registered MapLayer implementation.
    // This inserts the TileGrid resource which spawn_tile_meshes then picks up.
    if let Some(tiles_data) = file.layers.get("tiles") {
        let load_result = world.resource_scope(|world, registry: Mut<MapLayerRegistry>| {
            registry.load_layer("tiles", tiles_data, world)
        });

        match load_result {
            Ok(true) => info!("Editor: tiles layer loaded"),
            Ok(false) => {
                warn!("Editor: no tiles MapLayer registered, inserting default tilemap");
                world.insert_resource(super::default_editor_grid());
            }
            Err(e) => {
                error!("Editor: failed to load tiles layer: {e}");
                world.insert_resource(super::default_editor_grid());
            }
        }
    } else {
        world.insert_resource(super::default_editor_grid());
    }

    // Load the spawns layer manually to create lightweight editor markers
    // (no physics, no SpawnThing triggers).
    if let Some(spawns_data) = file.layers.get("spawns") {
        match from_layer_value::<Vec<SpawnPoint>>(spawns_data) {
            Ok(spawn_points) => {
                // Read assets and registry before spawning.
                let mesh = world
                    .get_resource::<SpawnMarkerAssets>()
                    .map(|a| a.mesh.clone());
                let material = world
                    .get_resource::<SpawnMarkerAssets>()
                    .map(|a| a.material.clone());

                for sp in &spawn_points {
                    let kind = {
                        let registry = world.resource::<ThingRegistry>();
                        match registry.kind_by_name(&sp.template) {
                            Some(k) => k,
                            None => {
                                warn!("Editor: unknown spawn template '{}', skipping", sp.template);
                                continue;
                            }
                        }
                    };

                    let pos = Vec3::from_array(sp.position);
                    let mut entity_commands = world.spawn((
                        Transform::from_translation(pos),
                        SpawnMarker,
                        Thing { kind },
                        EditorSpawnMarker,
                    ));
                    if let (Some(m), Some(mat)) = (&mesh, &material) {
                        entity_commands.insert((Mesh3d(m.clone()), MeshMaterial3d(mat.clone())));
                    }
                }
                info!(
                    "Editor: spawns layer loaded ({} points)",
                    spawn_points.len()
                );
            }
            Err(e) => {
                error!("Editor: failed to parse spawns layer: {e}");
            }
        }
    }

    // Store the loaded file for round-trip preservation of unknown layers.
    world.insert_resource(EditorMapFile(Some(file)));
}
