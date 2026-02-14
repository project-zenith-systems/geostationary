use bevy::prelude::*;
use main_menu::{MainMenuPlugin, MenuEvent};
use network::NetworkPlugin;
use physics::{PhysicsDebugPlugin, PhysicsPlugin};
use things::ThingsPlugin;
use tiles::TilesPlugin;
use ui::UiPlugin;

mod app_state;
mod camera;
mod client;
mod config;
mod creatures;
mod main_menu;
mod server;
mod world_setup;

fn main() {
    let app_config = config::load_config();

    App::new()
        .insert_resource(app_config.clone())
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
        .add_plugins(PhysicsPlugin)
        .add_plugins(PhysicsDebugPlugin)
        .add_plugins(TilesPlugin)
        .add_plugins(ThingsPlugin)
        .add_plugins(creatures::CreaturesPlugin)
        .add_plugins(camera::CameraPlugin)
        .add_plugins(world_setup::WorldSetupPlugin)
        .add_plugins(client::ClientPlugin)
        .add_plugins(server::ServerPlugin)
        .init_state::<app_state::AppState>()
        .run();
}
