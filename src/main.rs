use std::net::SocketAddr;

use bevy::prelude::*;
use main_menu::{MainMenuPlugin, MenuEvent};
use network::{NetCommand, NetEvent, NetworkPlugin, NetworkSet};
use physics::{PhysicsDebugPlugin, PhysicsPlugin};
use things::ThingsPlugin;
use tiles::TilesPlugin;
use ui::UiPlugin;

mod app_state;
mod camera;
mod config;
mod creatures;
mod main_menu;
mod net_game;
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
        .add_plugins(net_game::NetGamePlugin)
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
    network_role: Option<Res<net_game::NetworkRole>>,
) {
    for event in messages.read() {
        match event {
            NetEvent::HostingStarted { port } => {
                // Set NetworkRole to ListenServer when hosting starts
                commands.insert_resource(net_game::NetworkRole::ListenServer);
                let addr: SocketAddr = ([127, 0, 0, 1], *port).into();
                net_commands.write(NetCommand::Connect { addr });
            }
            NetEvent::Connected => {
                // If we're not already a ListenServer, we're a Client
                if !network_role.is_some_and(|r| *r == net_game::NetworkRole::ListenServer) {
                    commands.insert_resource(net_game::NetworkRole::Client);
                }

                next_state.set(app_state::AppState::InGame);
            }
            NetEvent::Disconnected { .. } => {
                // If we were a listen server, stop hosting
                if network_role.is_some_and(|r| *r == net_game::NetworkRole::ListenServer) {
                    net_commands.write(NetCommand::StopHosting);
                }
                // Reset NetworkRole to None when disconnected and return to the main menu
                commands.insert_resource(net_game::NetworkRole::None);
                commands.remove_resource::<net_game::LocalPeerId>();
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
