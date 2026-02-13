use std::net::SocketAddr;

use bevy::prelude::*;
use main_menu::{MainMenuPlugin, MenuEvent};
use network::{NetCommand, NetEvent, NetId, NetworkPlugin, NetworkSet};
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
        .add_plugins(server::ServerPlugin)
        .add_plugins(client::ClientPlugin)
        .init_state::<app_state::AppState>()
        .add_systems(
            PreUpdate,
            handle_net_events
                .after(NetworkSet::Receive)
                .before(NetworkSet::Send),
        )
        .run();
}

fn handle_net_events(
    mut commands: Commands,
    mut messages: MessageReader<NetEvent>,
    mut net_commands: MessageWriter<NetCommand>,
    mut menu_events: MessageWriter<MenuEvent>,
    mut next_state: ResMut<NextState<app_state::AppState>>,
    server: Option<Res<server::Server>>,
) {
    for event in messages.read() {
        match event {
            NetEvent::HostingStarted { port } => {
                info!("Hosting started on port {port}, setting role to ListenServer");
                let addr: SocketAddr = ([127, 0, 0, 1], *port).into();

                info!("Connecting to self at {addr}");
                net_commands.write(NetCommand::Connect { addr });
            }
            NetEvent::Connected => {
                if server.is_none() {
                    info!("Connected to server, setting role to Client");
                    commands.insert_resource(client::Client {
                        local_net_id: NetId(0),
                    });
                } else {
                    info!("Self-connection established (ListenServer)");
                }

                next_state.set(app_state::AppState::InGame);
            }
            NetEvent::Disconnected { reason } => {
                info!("Disconnected: {reason}");
                if server.is_some() {
                    net_commands.write(NetCommand::StopHosting);
                    commands.remove_resource::<server::Server>();
                }

                next_state.set(app_state::AppState::MainMenu);
                menu_events.write(MenuEvent::Title);
            }
            NetEvent::Error(msg) => {
                warn!("Network error: {msg}");
                menu_events.write(MenuEvent::Title);
            }
            _ => {}
        }
    }
}
