use bevy::prelude::*;
use network::Server;
use tiles::{AtmoSeed, GridSize, TileFlags, TileGrid, TileKind};
use world::MapPath;

use crate::app_state::AppState;
use crate::config::AppConfig;

/// Marker resource inserted after [`load_map_on_host`] has attempted to load
/// the map.  Prevents the system from retrying every frame when the load fails
/// (missing file, parse error, unsupported version, etc.).
#[derive(Resource)]
struct MapLoadAttempted;

/// Exclusive system that loads the map on a listen-server when the [`Server`]
/// resource appears but no [`TileGrid<TileKind>`] has been inserted yet.
///
/// Gated by `run_if(resource_exists::<Server>
///     .and(not(resource_exists::<TileGrid<TileKind>>))
///     .and(not(resource_exists::<MapLoadAttempted>)))`.
///
/// On a dedicated server `load_map` already ran at `Startup` (because
/// [`MapPath`] was present), so the grid guard prevents re-loading.
/// On a pure client (no [`Server`] resource) this system never fires.
///
/// The [`MapLoadAttempted`] guard ensures that if the load fails the system
/// does not retry every frame.  The guard is removed by [`cleanup_world`] on
/// `OnExit(AppState::InGame)` so a subsequent host session gets a fresh
/// attempt.
fn load_map_on_host(world: &mut World) {
    // Insert the one-shot guard before attempting to load so that failures
    // do not cause a tight retry loop.
    world.insert_resource(MapLoadAttempted);

    // Insert MapPath from config if it wasn't set already.
    if !world.contains_resource::<MapPath>() {
        let config = world.resource::<AppConfig>();
        let map_path = config.world.map_path.clone();
        world.insert_resource(MapPath::new(map_path));
    }

    world::loader::load_map(world);
}

/// Initializes the [`atmospherics::GasGrid`] and [`atmospherics::PressureForceScale`]
/// resources from the loaded [`TileGrid<TileKind>`] once the server is running.
///
/// Per-tile atmosphere is read from the [`AtmoSeed`] overrides: `Pressurised`
/// tiles get standard pressure, `Vacuum` tiles start at 0.0 moles.
fn init_atmosphere(
    mut commands: Commands,
    grid: Res<TileGrid<TileKind>>,
    atmo_seed: Option<Res<AtmoSeed>>,
    config: Res<AppConfig>,
) {
    let gas_grid = atmospherics::initialize_gas_grid(
        &grid,
        atmo_seed.as_deref(),
        config.atmospherics.standard_pressure,
        config.atmospherics.diffusion_rate,
    );
    commands.insert_resource(gas_grid);
    commands.remove_resource::<AtmoSeed>();
    commands.insert_resource(atmospherics::PressureForceScale(
        config.atmospherics.pressure_force_scale,
    ));
    info!("WorldInitPlugin: atmosphere initialized");
}

/// Cleans up world resources when exiting `InGame`.
fn cleanup_world(mut commands: Commands) {
    commands.remove_resource::<TileGrid<TileKind>>();
    commands.remove_resource::<GridSize>();
    commands.remove_resource::<TileFlags>();
    commands.remove_resource::<AtmoSeed>();
    commands.remove_resource::<atmospherics::GasGrid>();
    commands.remove_resource::<atmospherics::PressureForceScale>();
    commands.remove_resource::<MapLoadAttempted>();
}

/// Post-load world initialization plugin.
///
/// Provides the glue logic that the old `world_setup.rs` contained:
///
/// * **Listen-server map loading** — triggers [`world::loader::load_map`]
///   when the [`Server`] resource appears but no [`TileGrid<TileKind>`] exists
///   yet (the dedicated server already loads at `Startup` via
///   [`world::WorldPlugin`]).
/// * **Atmosphere initialization** — creates the [`atmospherics::GasGrid`] and
///   [`atmospherics::PressureForceScale`] resources from the loaded tile grid
///   and config values.
/// * **World cleanup** — removes tile grid, gas grid, and related resources on
///   `OnExit(AppState::InGame)`.
pub struct WorldInitPlugin;

impl Plugin for WorldInitPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            load_map_on_host.run_if(
                resource_exists::<Server>
                    .and(not(resource_exists::<TileGrid<TileKind>>))
                    .and(not(resource_exists::<MapLoadAttempted>)),
            ),
        );
        app.add_systems(
            Update,
            init_atmosphere.run_if(
                resource_exists::<Server>
                    .and(resource_exists::<TileGrid<TileKind>>)
                    .and(not(resource_exists::<atmospherics::GasGrid>)),
            ),
        );
        app.add_systems(OnExit(AppState::InGame), cleanup_world);
    }
}
