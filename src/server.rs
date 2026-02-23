use std::net::SocketAddr;

use bevy::prelude::*;
use network::{
    ClientId, ClientJoined, ClientMessage, ControlledByClient, EntityState,
    NETWORK_UPDATE_INTERVAL, NetCommand, NetId, NetServerSender, NetworkSet, Server, ServerEvent,
    ServerMessage, StreamSender,
};
use physics::LinearVelocity;
use things::{InputDirection, ThingsStreamMessage};

pub struct ServerPlugin;

impl Plugin for ServerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StateBroadcastTimer>();
        app.add_systems(
            PreUpdate,
            handle_server_events
                .run_if(resource_exists::<Server>)
                .after(NetworkSet::Receive)
                .before(NetworkSet::Send),
        );
        app.add_systems(Update, broadcast_state.run_if(resource_exists::<Server>));
    }
}

/// Timer for throttling state broadcasts from the server.
#[derive(Resource)]
struct StateBroadcastTimer(Timer);

impl Default for StateBroadcastTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(
            NETWORK_UPDATE_INTERVAL,
            TimerMode::Repeating,
        ))
    }
}

fn handle_server_events(
    mut messages: MessageReader<ServerEvent>,
    mut net_commands: MessageWriter<NetCommand>,
    mut joined: MessageWriter<ClientJoined>,
    mut sender: Option<ResMut<NetServerSender>>,
    mut server: ResMut<Server>,
    mut entities: Query<(
        &NetId,
        &ControlledByClient,
        &Transform,
        &LinearVelocity,
        &mut InputDirection,
    )>,
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
                handle_client_message(
                    from,
                    message,
                    &mut joined,
                    &mut sender,
                    &mut server,
                    &mut entities,
                );
            }
        }
    }
}

fn handle_client_message(
    from: &ClientId,
    message: &ClientMessage,
    joined: &mut MessageWriter<ClientJoined>,
    sender: &mut Option<ResMut<NetServerSender>>,
    server: &mut ResMut<Server>,
    entities: &mut Query<(
        &NetId,
        &ControlledByClient,
        &Transform,
        &LinearVelocity,
        &mut InputDirection,
    )>,
) {
    match message {
        ClientMessage::Hello { name } => {
            info!(
                "Received client hello from ClientId({}), name: {:?}",
                from.0, name
            );
            // Welcome is now sent automatically by the network task; do not re-send it here.

            let sender = match sender.as_mut() {
                Some(s) => s,
                None => {
                    error!(
                        "No NetServerSender available to process hello for ClientId({})",
                        from.0
                    );
                    return;
                }
            };

            // Notify domain modules that this client has joined.
            info!("Emitting ClientJoined for ClientId({})", from.0);
            joined.write(ClientJoined { id: *from });

            // Catch-up: send EntitySpawned for every existing replicated entity
            for (net_id, controlled_by, transform, velocity, _) in entities.iter() {
                sender.send_to(
                    *from,
                    &ServerMessage::EntitySpawned {
                        net_id: *net_id,
                        kind: 0,
                        position: transform.translation.into(),
                        velocity: [velocity.x, velocity.y, velocity.z],
                        owner: if controlled_by.0 == *from {
                            Some(*from)
                        } else {
                            None
                        },
                        name: None,
                    },
                );
            }

            // Spawn player entity
            let net_id = server.next_net_id();
            let spawn_pos = Vec3::new(6.0, 0.81, 3.0);
            info!(
                "Spawning player entity NetId({}) for ClientId({}) at {spawn_pos}",
                net_id.0, from.0
            );

            sender.broadcast(&ServerMessage::EntitySpawned {
                net_id,
                kind: 0,
                position: spawn_pos.into(),
                velocity: [0.0, 0.0, 0.0],
                owner: Some(*from),
                name: None,
            });
        }
        ClientMessage::Input { direction } => {
            for (_, controlled_by, _, _, mut input_dir) in entities.iter_mut() {
                if controlled_by.0 == *from {
                    input_dir.0 = Vec3::from_array(*direction);
                    break;
                }
            }
        }
    }
}

/// System that broadcasts state updates to all clients on stream 3.
/// Collects all replicated entity positions and sends ThingsStreamMessage::StateUpdate.
/// Throttled to NETWORK_UPDATE_RATE to reduce bandwidth usage.
fn broadcast_state(
    time: Res<Time>,
    mut timer: ResMut<StateBroadcastTimer>,
    stream_sender: Option<Res<StreamSender<ThingsStreamMessage>>>,
    entities: Query<(&NetId, &Transform, &LinearVelocity)>,
) {
    let Some(sender) = stream_sender else {
        return;
    };

    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }

    let states = entities
        .iter()
        .map(|(net_id, transform, velocity)| EntityState {
            net_id: *net_id,
            position: transform.translation.into(),
            velocity: [velocity.x, velocity.y, velocity.z],
        })
        .collect::<Vec<EntityState>>();

    if !states.is_empty() {
        if let Err(e) = sender.broadcast(&ThingsStreamMessage::StateUpdate { entities: states }) {
            error!("Failed to broadcast entity state on things stream: {e}");
        }
    }
}
