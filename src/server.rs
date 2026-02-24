use std::net::SocketAddr;

use bevy::prelude::*;
use network::{
    ClientId, ClientMessage, NetCommand, NetServerSender, NetworkSet, PlayerEvent, Server,
    ServerEvent, ServerMessage,
};
use souls::ClientInputReceived;

use crate::config::AppConfig;

pub struct ServerPlugin;

impl Plugin for ServerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            handle_server_events
                .run_if(resource_exists::<Server>)
                .after(NetworkSet::Receive)
                .before(NetworkSet::Send),
        );
        app.add_systems(
            PostUpdate,
            send_initial_state_done.run_if(resource_exists::<Server>),
        );
    }
}

fn handle_server_events(
    mut messages: MessageReader<ServerEvent>,
    mut net_commands: MessageWriter<NetCommand>,
    mut player: MessageWriter<PlayerEvent>,
    mut input: MessageWriter<ClientInputReceived>,
    config: Res<AppConfig>,
) {
    for event in messages.read() {
        match event {
            ServerEvent::HostingStarted { port } => {
                info!("Hosting started on port {port}");
                let addr: SocketAddr = ([127, 0, 0, 1], *port).into();
                info!("Connecting to self at {addr}");
                net_commands.write(NetCommand::Connect {
                    addr,
                    name: config.souls.player_name.clone(),
                });
            }
            ServerEvent::HostingStopped => {
                // Server resource removal handled by NetworkPlugin
            }
            ServerEvent::Error(msg) => {
                error!("Network error: {msg}");
            }
            ServerEvent::ClientConnected { id, addr, name } => {
                info!("Client {} ('{}') connected from {addr}", id.0, name);
            }
            ServerEvent::ClientDisconnected { id } => {
                info!("Client {} disconnected", id.0);
                player.write(PlayerEvent::Left { id: *id });
            }
            ServerEvent::ClientMessageReceived { from, message } => {
                handle_client_message(from, message, &mut player, &mut input);
            }
        }
    }
}

fn handle_client_message(
    from: &ClientId,
    message: &ClientMessage,
    player: &mut MessageWriter<PlayerEvent>,
    input: &mut MessageWriter<ClientInputReceived>,
) {
    match message {
        ClientMessage::Hello { name } => {
            info!(
                "Received client hello from ClientId({}), name: {:?}",
                from.0, name
            );
            // Entity catch-up and player spawning are handled by SoulsPlugin on PlayerEvent::Joined.
            player.write(PlayerEvent::Joined {
                id: *from,
                name: name.clone(),
            });
        }
        ClientMessage::Input { direction } => {
            input.write(ClientInputReceived {
                from: *from,
                direction: *direction,
            });
        }
    }
}

/// Server-side system: sends [`ServerMessage::InitialStateDone`] on the control stream to each
/// client that joined this frame, after all module stream handlers have had a chance to write
/// their initial data burst in the same frame.
///
/// Runs in [`PostUpdate`] so that module systems in [`Update`] (tiles, atmospherics) complete
/// their initial sends before this sentinel is queued.
fn send_initial_state_done(
    mut events: MessageReader<PlayerEvent>,
    sender: Option<Res<NetServerSender>>,
) {
    let Some(sender) = sender else {
        return;
    };
    for event in events.read() {
        let PlayerEvent::Joined { id, .. } = event else {
            continue;
        };
        sender.send_to(*id, &ServerMessage::InitialStateDone);
        info!("Sent InitialStateDone to ClientId({})", id.0);
    }
}

