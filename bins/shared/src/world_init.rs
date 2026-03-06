use bevy::prelude::*;
use network::Server;
use tiles::Tilemap;
use world::MapPath;

use crate::app_state::AppState;
use crate::config::AppConfig;

/// Marker resource inserted after [`load_map_on_host`] has attempted to load
/// the map.  Prevents the system from retrying every frame when the load fails
/// (missing file, parse error, unsupported version, etc.).
#[derive(Resource)]
struct MapLoadAttempted;

/// Exclusive system that loads the map on a listen-server when the [`Server`]
/// resource appears but no [`Tilemap`] has been inserted yet.
///
/// Gated by `run_if(resource_exists::<Server>
///     .and(not(resource_exists::<Tilemap>))
///     .and(not(resource_exists::<MapLoadAttempted>)))`.
///
/// On a dedicated server `load_map` already ran at `Startup` (because
/// [`MapPath`] was present), so the [`Tilemap`] guard prevents re-loading.
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
/// resources from the loaded [`Tilemap`] once the server is running.
///
/// All walkable cells are filled with `standard_pressure` from config (no
/// vacuum region — vacuum regions were an artifact of the old hardcoded test
/// room and will be expressed as a map layer in the future).
fn init_atmosphere(
    mut commands: Commands,
    tilemap: Res<Tilemap>,
    config: Res<AppConfig>,
) {
    let gas_grid = atmospherics::initialize_gas_grid(
        &tilemap,
        config.atmospherics.standard_pressure,
        None, // No vacuum region — uniform standard pressure for now.
              // Vacuum regions will be expressed via an "atmospherics" map layer.
        config.atmospherics.diffusion_rate,
    );
    commands.insert_resource(gas_grid);
    commands.insert_resource(atmospherics::PressureForceScale(
        config.atmospherics.pressure_force_scale,
    ));
    info!("WorldInitPlugin: atmosphere initialized");
}

/// Cleans up world resources when exiting `InGame`.
fn cleanup_world(mut commands: Commands) {
    commands.remove_resource::<Tilemap>();
    commands.remove_resource::<atmospherics::GasGrid>();
    commands.remove_resource::<atmospherics::PressureForceScale>();
    commands.remove_resource::<MapLoadAttempted>();
}

/// Post-load world initialization plugin.
///
/// Provides the glue logic that the old `world_setup.rs` contained:
///
/// * **Listen-server map loading** — triggers [`world::loader::load_map`]
///   when the [`Server`] resource appears but no [`Tilemap`] exists yet (the
///   dedicated server already loads at `Startup` via [`world::WorldPlugin`]).
/// * **Atmosphere initialization** — creates the [`atmospherics::GasGrid`] and
///   [`atmospherics::PressureForceScale`] resources from the loaded tilemap and
///   config values.
/// * **World cleanup** — removes `Tilemap`, `GasGrid`, and
///   `PressureForceScale` on `OnExit(AppState::InGame)`.
pub struct WorldInitPlugin;

impl Plugin for WorldInitPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            load_map_on_host.run_if(
                resource_exists::<Server>
                    .and(not(resource_exists::<Tilemap>))
                    .and(not(resource_exists::<MapLoadAttempted>)),
            ),
        );
        app.add_systems(
            Update,
            init_atmosphere.run_if(
                resource_exists::<Server>
                    .and(resource_exists::<Tilemap>)
                    .and(not(resource_exists::<atmospherics::GasGrid>)),
            ),
        );
        app.add_systems(OnExit(AppState::InGame), cleanup_world);
    }
}
