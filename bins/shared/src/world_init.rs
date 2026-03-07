use bevy::prelude::*;
use network::Server;
use world::{MapLoaded, MapPath};

use crate::app_state::AppState;
use crate::config::AppConfig;

/// Marker resource inserted after [`load_map_on_host`] has attempted to load
/// the map.  Prevents the system from retrying every frame when the load fails
/// (missing file, parse error, unsupported version, etc.).
#[derive(Resource)]
struct MapLoadAttempted;

/// Exclusive system that loads the map on a listen-server when the [`Server`]
/// resource appears but no map has been loaded yet.
///
/// Gated by `run_if(resource_exists::<Server>
///     .and(not(resource_exists::<MapLoaded>))
///     .and(not(resource_exists::<MapLoadAttempted>)))`.
///
/// On a dedicated server `load_map` already ran at `Startup` (because
/// [`MapPath`] was present), so the [`MapLoaded`] guard prevents re-loading.
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

/// Cleans up load-related guards when exiting `InGame` so a subsequent
/// host session gets a fresh attempt.
fn cleanup_world(mut commands: Commands) {
    commands.remove_resource::<MapLoadAttempted>();
    commands.remove_resource::<MapLoaded>();
    commands.remove_resource::<MapPath>();
}

/// Post-load world initialization plugin.
///
/// Provides glue logic for listen-server map loading and cleanup:
///
/// * **Listen-server map loading** — triggers [`world::loader::load_map`]
///   when the [`Server`] resource appears but no [`MapLoaded`] marker exists
///   yet (the dedicated server already loads at `Startup` via
///   [`world::WorldPlugin`]).
/// * **Load-attempt guard cleanup** — removes [`MapLoadAttempted`] on
///   `OnExit(AppState::InGame)` so a subsequent host session gets a fresh
///   attempt.
pub struct WorldInitPlugin;

impl Plugin for WorldInitPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            load_map_on_host.run_if(
                resource_exists::<Server>
                    .and(not(resource_exists::<MapLoaded>))
                    .and(not(resource_exists::<MapLoadAttempted>)),
            ),
        );
        app.add_systems(OnExit(AppState::InGame), cleanup_world);
    }
}
