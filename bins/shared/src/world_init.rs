use bevy::prelude::*;
use network::Server;
use tiles::Tilemap;
use world::MapPath;

use crate::app_state::AppState;
use crate::config::AppConfig;

/// Exclusive system that loads the map on a listen-server when the [`Server`]
/// resource appears but no [`Tilemap`] has been inserted yet.
///
/// On a dedicated server `load_map` already ran at `Startup` (because
/// [`MapPath`] was present), so the [`Tilemap`] guard causes an early return.
/// On a pure client (no [`Server`] resource) this system never fires.
fn load_map_on_host(world: &mut World) {
    if !world.contains_resource::<Server>() || world.contains_resource::<Tilemap>() {
        return;
    }

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
        None,
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
        app.add_systems(Update, load_map_on_host);
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
