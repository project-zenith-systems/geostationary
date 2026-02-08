use std::net::SocketAddr;

use bevy::prelude::*;
use main_menu::{MainMenuPlugin, MenuEvent};
use network::{NetCommand, NetEvent, NetworkPlugin, NetworkSet};
use ui::UiPlugin;

mod app_state;
mod main_menu;

fn main() {
    App::new()
        .add_plugins(DefaultPlugins.set(WindowPlugin {
            primary_window: Some(Window {
                title: "Geostationary".into(),
                ..default()
            }),
            ..default()
        }))
        .add_plugins(UiPlugin::new().with_event::<MenuEvent>())
        .add_plugins(MainMenuPlugin)
        .add_plugins(NetworkPlugin)
        .init_state::<app_state::AppState>()
        .add_systems(Startup, spawn_camera)
        .add_systems(
            PreUpdate,
            handle_net_events
                .after(NetworkSet::Receive)
                .before(NetworkSet::Send),
        )
        .run();
}

fn spawn_camera(mut commands: Commands) {
    commands.spawn(Camera2d);
}

fn handle_net_events(
    mut messages: MessageReader<NetEvent>,
    mut net_commands: MessageWriter<NetCommand>,
    mut menu_events: MessageWriter<MenuEvent>,
    mut next_state: ResMut<NextState<app_state::AppState>>,
) {
    for event in messages.read() {
        match event {
            NetEvent::HostingStarted { port } => {
                let addr: SocketAddr = ([127, 0, 0, 1], *port).into();
                net_commands.write(NetCommand::Connect { addr });
            }
            NetEvent::Connected => {
                next_state.set(app_state::AppState::InGame);
            }
            NetEvent::Error(msg) => {
                warn!("Network error: {msg}");
                menu_events.write(MenuEvent::Title);
            }
            _ => {}
        }
    }
}
