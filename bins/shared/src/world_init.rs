use bevy::prelude::*;
use network::{Headless, Server};
use world::MapPath;

use crate::app_state::AppState;
use crate::config::AppConfig;

/// Exclusive system that runs on [`OnEnter(AppState::Loading)`].
///
/// * **Hosting (listen-server or dedicated):** loads the map from disk via
///   [`world::loader::load_map`].  Inserts [`MapPath`] from [`AppConfig`] if
///   it wasn't already present.
/// * **Headless (dedicated server):** skips the network sync barrier and
///   transitions straight to [`AppState::InGame`].
/// * **Pure client (no [`Server`] resource):** does nothing — the network
///   sync barrier in the orchestration layer handles the `Loading → InGame`
///   transition.
fn on_enter_loading(world: &mut World) {
    if world.contains_resource::<Server>() {
        if !world.contains_resource::<MapPath>() {
            let config = world.resource::<AppConfig>();
            let map_path = config.world.map_path.clone();
            world.insert_resource(MapPath::new(map_path));
        }
        world::loader::load_map(world);
    }

    // Dedicated server: no client-side sync barrier, go straight to InGame.
    if world.contains_resource::<Headless>() {
        world
            .resource_mut::<NextState<AppState>>()
            .set(AppState::InGame);
    }
}

/// Cleans up load-related resources when exiting `InGame` so a subsequent
/// host session gets a fresh attempt.
fn cleanup_world(mut commands: Commands) {
    commands.remove_resource::<MapPath>();
}

/// World-loading glue plugin.
///
/// * **`OnEnter(AppState::Loading)`** — runs [`on_enter_loading`] to load
///   the map (if hosting) and shortcut to `InGame` (if headless).
/// * **`OnExit(AppState::InGame)`** — removes [`MapPath`] so re-hosting
///   picks up fresh config.
pub struct WorldInitPlugin;

impl Plugin for WorldInitPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppState::Loading), on_enter_loading);
        app.add_systems(OnExit(AppState::InGame), cleanup_world);
    }
}
