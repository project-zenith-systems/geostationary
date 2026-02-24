use std::net::SocketAddr;

use bevy::prelude::*;
use network::{
    ClientId, ClientJoined, ClientMessage, ControlledByClient, NetCommand, NetworkSet, Server,
    ServerEvent,
};
use things::InputDirection;

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
    }
}

fn handle_server_events(
    mut messages: MessageReader<ServerEvent>,
    mut net_commands: MessageWriter<NetCommand>,
    mut joined: MessageWriter<ClientJoined>,
    mut entities: Query<(&ControlledByClient, &mut InputDirection)>,
) {
    for event in messages.read() {
        match event {
            ServerEvent::HostingStarted { port } => {
                info!("Hosting started on port {port}");
                let addr: SocketAddr = ([127, 0, 0, 1], *port).into();
                info!("Connecting to self at {addr}");
                net_commands.write(NetCommand::Connect { addr });
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
            }
            ServerEvent::ClientMessageReceived { from, message } => {
                handle_client_message(from, message, &mut joined, &mut entities);
            }
        }
    }
}

fn handle_client_message(
    from: &ClientId,
    message: &ClientMessage,
    joined: &mut MessageWriter<ClientJoined>,
    entities: &mut Query<(&ControlledByClient, &mut InputDirection)>,
) {
    match message {
        ClientMessage::Hello { name } => {
            info!(
                "Received client hello from ClientId({}), name: {:?}",
                from.0, name
            );
            // Welcome is sent automatically by the network task.
            // Entity catch-up and player spawning are handled by ThingsPlugin on ClientJoined.
            joined.write(ClientJoined { id: *from });
        }
        ClientMessage::Input { direction } => {
            for (controlled_by, mut input_dir) in entities.iter_mut() {
                if controlled_by.0 == *from {
                    input_dir.0 = Vec3::from_array(*direction);
                    break;
                }
            }
        }
    }
}

