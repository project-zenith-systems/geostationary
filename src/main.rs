use bevy::prelude::*;
use main_menu::{MainMenuPlugin, MenuEvent};
use network::NetworkPlugin;
use physics::{PhysicsDebugPlugin, PhysicsPlugin};
use things::ThingsPlugin;
use tiles::TilesPlugin;
use ui::UiPlugin;
use atmospherics::AtmosphericsPlugin;

mod app_state;
mod client;
mod config;
mod main_menu;
mod server;
mod world_setup;

fn main() {
    let app_config = config::load_config();

    let mut app = App::new();
    app.insert_resource(app_config.clone())
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: app_config.window.title.clone(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(UiPlugin::new().with_event::<MenuEvent>())
        .add_plugins(MainMenuPlugin)
        .add_plugins(NetworkPlugin)
        .add_plugins(PhysicsPlugin);

    if app_config.debug.physics_debug {
        app.add_plugins(PhysicsDebugPlugin);
    }

    app.add_plugins(TilesPlugin)
        .add_plugins(ThingsPlugin)
        .add_plugins(creatures::CreaturesPlugin)
        .add_plugins(player::PlayerPlugin)
        .add_plugins(camera::CameraPlugin::<app_state::AppState>::in_state(
            app_state::AppState::InGame,
        ))
        .add_plugins(AtmosphericsPlugin)
        .add_plugins(world_setup::WorldSetupPlugin)
        .add_plugins(client::ClientPlugin)
        .add_plugins(server::ServerPlugin)
        .init_state::<app_state::AppState>()
        .run();
}
