use std::net::SocketAddr;

use bevy::prelude::*;
use bevy::state::state_scoped::DespawnOnExit;
use network::{
    Client, ClientEvent, ClientId, ClientMessage, ControlledByClient, EntityState,
    NETWORK_UPDATE_INTERVAL, NetCommand, NetId, NetServerSender, NetworkSet, Server, ServerEvent,
    ServerMessage,
};
use physics::LinearVelocity;
use things::SpawnThing;

use crate::app_state::AppState;
use crate::creatures::{Creature, MovementSpeed, PlayerControlled};
use crate::main_menu::MenuEvent;

pub struct NetworkEventsPlugin;

impl Plugin for NetworkEventsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<StateBroadcastTimer>();
        app.add_systems(
            PreUpdate,
            (
                handle_server_events.run_if(resource_exists::<Server>),
                handle_client_events.run_if(resource_exists::<Client>),
            )
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
    mut commands: Commands,
    mut messages: MessageReader<ServerEvent>,
    mut net_commands: MessageWriter<NetCommand>,
    mut menu_events: MessageWriter<MenuEvent>,
    mut sender: Option<ResMut<NetServerSender>>,
    mut server: ResMut<Server>,
    mut entities: Query<(
        &NetId,
        &ControlledByClient,
        &Transform,
        &mut LinearVelocity,
        &MovementSpeed,
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
                commands.remove_resource::<Server>();
            }
            ServerEvent::Error(msg) => {
                error!("Network error: {msg}");
                // TODO proper error handling and user feedback instead of just returning to main menu
                menu_events.write(MenuEvent::Title);
            }
            ServerEvent::ClientConnected { id, addr } => {
                info!("Client {} connected from {addr}", id.0);
            }
            ServerEvent::ClientDisconnected { id } => {
                info!("Client {} disconnected", id.0);
            }
            ServerEvent::ClientMessageReceived { from, message } => {
                handle_client_message(from, message, &mut sender, &mut server, &mut entities);
            }
        }
    }
}

fn handle_client_events(
    mut commands: Commands,
    mut messages: MessageReader<ClientEvent>,
    mut menu_events: MessageWriter<MenuEvent>,
    mut next_state: ResMut<NextState<AppState>>,
    mut entities: Query<(Entity, &NetId, &mut Transform), With<Creature>>,
    mut client: ResMut<Client>,
) {
    for event in messages.read() {
        match event {
            ClientEvent::Connected => {
                info!("Connected to server");
                next_state.set(AppState::InGame);
            }
            ClientEvent::Disconnected { reason } => {
                info!("Disconnected: {reason}");
                next_state.set(AppState::MainMenu);
                menu_events.write(MenuEvent::Title);
            }
            ClientEvent::Error(msg) => {
                error!("Network error: {msg}");
                menu_events.write(MenuEvent::Title);
            }
            ClientEvent::ServerMessageReceived(message) => {
                handle_server_message(message, &mut commands, &mut entities, &mut client);
            }
        }
    }
}

fn handle_client_message(
    from: &ClientId,
    message: &ClientMessage,
    sender: &mut Option<ResMut<NetServerSender>>,
    server: &mut ResMut<Server>,
    entities: &mut Query<(
        &NetId,
        &ControlledByClient,
        &Transform,
        &mut LinearVelocity,
        &MovementSpeed,
    )>,
) {
    match message {
        ClientMessage::Hello => {
            info!("Received client hello from ClientId({})", from.0);

            let sender = match sender.as_mut() {
                Some(s) => s,
                None => {
                    warn!(
                        "No NetServerSender available to process hello for ClientId({})",
                        from.0
                    );
                    return;
                }
            };

            sender.send_to(*from, &ServerMessage::Welcome { client_id: *from });

            // Catch-up: send EntitySpawned for every existing replicated entity
            for (net_id, _, transform, velocity, _) in entities.iter() {
                sender.send_to(
                    *from,
                    &ServerMessage::EntitySpawned {
                        net_id: *net_id,
                        kind: 0,
                        position: transform.translation.into(),
                        velocity: [velocity.x, velocity.y, velocity.z],
                        controlled: false,
                    },
                );
            }

            // Spawn player entity
            let net_id = server.next_net_id();
            let spawn_pos = Vec3::new(6.0, 0.86, 3.0);
            info!(
                "Spawning player entity NetId({}) for ClientId({}) at {spawn_pos}",
                net_id.0, from.0
            );

            // Tell the owning client first (with control flag)
            sender.send_to(
                *from,
                &ServerMessage::EntitySpawned {
                    net_id,
                    kind: 0,
                    position: spawn_pos.into(),
                    velocity: [0.0, 0.0, 0.0],
                    controlled: true,
                },
            );

            // Broadcast to all (owner will skip via duplicate check)
            sender.broadcast(&ServerMessage::EntitySpawned {
                net_id,
                kind: 0,
                position: spawn_pos.into(),
                velocity: [0.0, 0.0, 0.0],
                controlled: false,
            });
        }
        ClientMessage::Input { direction } => {
            for (_, controlled_by, _, mut velocity, movement_speed) in entities.iter_mut() {
                if controlled_by.0 == *from {
                    let dir = Vec3::from_array(*direction);
                    let desired = if dir.length_squared() > 0.0 {
                        dir.normalize() * movement_speed.speed
                    } else {
                        Vec3::ZERO
                    };
                    velocity.x = desired.x;
                    velocity.z = desired.z;
                    break;
                }
            }
        }
    }
}

fn handle_server_message(
    message: &ServerMessage,
    commands: &mut Commands,
    entities: &mut Query<(Entity, &NetId, &mut Transform), With<Creature>>,
    client: &mut ResMut<Client>,
) {
    match message {
        ServerMessage::Welcome { client_id } => {
            info!("Received Welcome, local ClientId assigned: {}", client_id.0);
            client.local_id = Some(*client_id);
        }
        ServerMessage::EntitySpawned {
            net_id,
            kind,
            position,
            velocity: _,
            controlled,
        } => {
            // Skip if entity already exists (e.g. duplicate message)
            let already_exists = entities.iter().any(|(_, id, _)| id.0 == net_id.0);
            if already_exists {
                debug!(
                    "EntitySpawned for NetId({}) but already exists, skipping",
                    net_id.0
                );
                return;
            }

            let pos = Vec3::from_array(*position);
            info!("Spawning replica entity NetId({}) at {pos}", net_id.0);

            let entity = commands
                .spawn((net_id.clone(), DespawnOnExit(AppState::InGame)))
                .id();
            commands.trigger(SpawnThing {
                entity,
                kind: *kind,
                position: pos,
            });

            if *controlled {
                if let Some(local_client_id) = client.local_id {
                    info!("Taking control of NetId({})", net_id.0);
                    commands
                        .entity(entity)
                        .insert((PlayerControlled, ControlledByClient(local_client_id)));
                } else {
                    error!("EntitySpawned with controlled=true but local_id is not set");
                }
            }
        }
        ServerMessage::EntityDespawned { net_id } => {
            info!("Despawning replica entity NetId({})", net_id.0);
            for (entity, id, _) in entities.iter() {
                if id.0 == net_id.0 {
                    commands.entity(entity).despawn();
                    break;
                }
            }
        }
        ServerMessage::StateUpdate { entities: states } => {
            for state in states.iter() {
                for (_, id, mut transform) in entities.iter_mut() {
                    if id.0 == state.net_id.0 {
                        transform.translation = Vec3::from_array(state.position);
                        break;
                    }
                }
            }
        }
    }
}

/// System that broadcasts state updates to all clients.
/// Collects all replicated entity positions and sends StateUpdate.
/// Throttled to NETWORK_UPDATE_RATE to reduce bandwidth usage.
fn broadcast_state(
    time: Res<Time>,
    mut timer: ResMut<StateBroadcastTimer>,
    server_sender: Option<Res<NetServerSender>>,
    entities: Query<(&NetId, &Transform, &LinearVelocity)>,
) {
    let Some(sender) = server_sender else {
        return;
    };

    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }

    let states = entities
        .iter()
        .map(|(net_id, transform, velocity)| EntityState {
            net_id: net_id.clone(),
            position: transform.translation.into(),
            velocity: [velocity.x, velocity.y, velocity.z],
        })
        .collect::<Vec<EntityState>>();

    if !states.is_empty() {
        sender.broadcast(&ServerMessage::StateUpdate { entities: states });
    }
}
