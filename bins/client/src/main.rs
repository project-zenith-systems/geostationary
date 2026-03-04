use bevy::log::LogPlugin;
use bevy::prelude::*;
use editor::EditorPlugin;
use input::InputPlugin;
use interactions::{ContextMenuAction, InteractionsPlugin};
use items::{InteractionRange, ItemsPlugin};
use main_menu::{MainMenuPlugin, MenuEvent};
use network::NetworkPlugin;
use physics::{PhysicsDebugPlugin, PhysicsPlugin};
use shared::app_state::AppState;
use things::ThingsPlugin;
use tiles::TilesPlugin;
use ui::UiPlugin;

mod client;
mod editor;
mod main_menu;

fn main() {
    let app_config = shared::config::load_config();

    let mut app = App::new();
    app.insert_resource(app_config.clone());

    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin {
                primary_window: Some(Window {
                    title: app_config.window.title.clone(),
                    ..default()
                }),
                ..default()
            })
            .set(LogPlugin::from(&app_config)),
    )
    .add_plugins(UiPlugin::new().with_event::<MenuEvent>().with_event::<ContextMenuAction>())
    .add_plugins(MainMenuPlugin)
    .add_plugins(NetworkPlugin)
    .add_plugins(PhysicsPlugin);

    if app_config.debug.physics_debug {
        app.add_plugins(PhysicsDebugPlugin);
    }

    app.add_plugins(TilesPlugin)
        .add_plugins(ThingsPlugin::<AppState>::in_state(AppState::InGame))
        .add_plugins(atmospherics::AtmosphericsPlugin)
        .add_plugins(creatures::CreaturesPlugin)
        .add_plugins(souls::SoulsPlugin)
        .add_plugins(player::PlayerPlugin)
        .add_plugins(camera::CameraPlugin::<AppState>::in_state(AppState::InGame))
        .add_plugins(EditorPlugin)
        .add_plugins(shared::world_setup::WorldSetupPlugin)
        .add_plugins(shared::templates::TemplatesPlugin)
        .add_plugins(client::ClientPlugin)
        .add_plugins(shared::server::ServerPlugin)
        .add_plugins(InputPlugin::<AppState>::in_state(AppState::InGame))
        .add_plugins(InteractionsPlugin::<AppState>::in_state(AppState::InGame))
        .add_plugins(ItemsPlugin)
        .insert_resource(InteractionRange(app_config.items.interaction_range))
        .init_state::<AppState>();

    app.run();
}
