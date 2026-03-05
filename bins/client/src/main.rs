use std::net::SocketAddr;

use bevy::log::LogPlugin;
use bevy::prelude::*;
use input::InputPlugin;
use interactions::{ContextMenuAction, InteractionsPlugin};
use items::{InteractionRange, ItemsPlugin};
use main_menu::{MainMenuConfig, MainMenuPlugin, MenuEvent};
use network::{Headless, NetCommand, NetworkPlugin, NetworkSet, ServerEvent};
use physics::{PhysicsDebugPlugin, PhysicsPlugin};
use shared::app_state::AppState;
use shared::config::AppConfig;
use things::ThingsPlugin;
use tiles::TilesPlugin;
use ui::UiPlugin;

fn main() {
    let app_config = shared::config::load_config();

    let mut app = App::new();
    app.insert_resource(app_config.clone());

    app.add_plugins(
        DefaultPlugins
            .set(WindowPlugin::from(&app_config))
            .set(LogPlugin::from(&app_config)),
    )
    .add_plugins(UiPlugin::new().with_event::<MenuEvent>().with_event::<ContextMenuAction>())
    .insert_resource(MainMenuConfig {
        port: app_config.network.port,
        player_name: app_config.souls.player_name.clone(),
    })
    .add_plugins(MainMenuPlugin { state: AppState::MainMenu })
    .add_plugins(NetworkPlugin {
        in_game: AppState::InGame,
        disconnected: AppState::MainMenu,
    })
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
        .add_plugins(shared::world_setup::WorldSetupPlugin)
        .add_plugins(shared::templates::TemplatesPlugin)
        .add_plugins(InputPlugin::<AppState>::in_state(AppState::InGame))
        .add_plugins(InteractionsPlugin::<AppState>::in_state(AppState::InGame))
        .add_plugins(ItemsPlugin)
        .insert_resource(InteractionRange(app_config.items.interaction_range))
        .add_systems(
            PreUpdate,
            listen_server_self_connect
                .after(NetworkSet::Receive)
                .before(NetworkSet::Send),
        )
        .init_state::<AppState>();

    app.run();
}

/// On a listen-server (non-headless), automatically connects to self when hosting starts.
fn listen_server_self_connect(
    mut messages: MessageReader<ServerEvent>,
    mut net_commands: MessageWriter<NetCommand>,
    config: Res<AppConfig>,
    headless: Option<Res<Headless>>,
) {
    if headless.is_some() {
        return;
    }
    for event in messages.read() {
        if let ServerEvent::HostingStarted { port } = event {
            let addr: SocketAddr = ([127, 0, 0, 1], *port).into();
            info!("Listen-server: connecting to self at {addr}");
            net_commands.write(NetCommand::Connect {
                addr,
                name: config.souls.player_name.clone(),
            });
        }
    }
}
