use std::net::SocketAddr;

use bevy::prelude::*;
use bevy::state::state_scoped::DespawnOnExit;
use network::{
    Client, ClientEvent, ClientId, ClientMessage, ControlledByClient, EntityState,
    NETWORK_UPDATE_INTERVAL, NetCommand, NetId, NetServerSender, NetworkSet, Server, ServerEvent,
    ServerMessage,
};
use physics::{Collider, GravityScale, LinearVelocity, LockedAxes, RigidBody};
use things::Thing;

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
        app.add_systems(
            Update,
            (apply_remote_input, broadcast_state).run_if(resource_exists::<Server>),
        );
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
    existing_entities: Query<(&NetId, &Transform, &LinearVelocity)>,
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

                let sender = match sender.as_mut() {
                    Some(s) => s,
                    None => {
                        warn!(
                            "No NetServerSender available to send welcome message to ClientId({})",
                            id.0
                        );
                        continue;
                    }
                };

                // 1. Welcome
                sender.send_to(*id, &ServerMessage::Welcome { client_id: *id });

                // 2. Catch-up: send EntitySpawned for every existing replicated entity
                for (net_id, transform, velocity) in existing_entities.iter() {
                    sender.send_to(
                        *id,
                        &ServerMessage::EntitySpawned {
                            net_id: *net_id,
                            kind: 0,
                            position: transform.translation.into(),
                            velocity: [velocity.x, velocity.y, velocity.z],
                        },
                    );
                }

                // 3. Spawn player entity
                let net_id = server.next_net_id();
                let spawn_pos = Vec3::new(6.0, 0.86, 3.0);
                info!(
                    "Spawning player entity NetId({}) for ClientId({}) at {spawn_pos}",
                    net_id.0, id.0
                );

                // 4. Broadcast EntitySpawned to all clients
                sender.broadcast(&ServerMessage::EntitySpawned {
                    net_id: net_id,
                    kind: 0,
                    position: spawn_pos.into(),
                    velocity: [0.0, 0.0, 0.0],
                });

                // 5. Tell this client which entity they control
                sender.send_to(*id, &ServerMessage::AssignControl { net_id: net_id });
            }
            ServerEvent::ClientDisconnected { id } => {
                info!("Client {} disconnected", id.0);
            }
            ServerEvent::ClientMessageReceived { from, message } => {
                handle_client_message(from, message);
            }
        }
    }
}

fn handle_client_events(
    mut commands: Commands,
    mut messages: MessageReader<ClientEvent>,
    mut menu_events: MessageWriter<MenuEvent>,
    mut next_state: ResMut<NextState<AppState>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut entities: Query<(Entity, &NetId, &mut Transform), With<Creature>>,
) {
    for event in messages.read() {
        match event {
            ClientEvent::Connected => {
                info!("Connected to server");

                // if server.is_none() {
                //     info!("Connected to server, setting role to Client");
                //     commands.insert_resource(Client {
                //         local_net_id: NetId(0),
                //     });
                // } else {
                //     info!("Self-connection established (ListenServer)");
                // }

                next_state.set(AppState::InGame);
            }
            ClientEvent::Disconnected { reason } => {
                info!("Disconnected: {reason}");
                // if server.is_some() {
                //     net_commands.write(NetCommand::StopHosting);
                //     commands.remove_resource::<Server>();
                // }

                next_state.set(AppState::MainMenu);
                menu_events.write(MenuEvent::Title);
            }
            ClientEvent::Error(msg) => {
                error!("Network error: {msg}");
                menu_events.write(MenuEvent::Title);
            }
            ClientEvent::ServerMessageReceived(message) => {
                handle_server_message(
                    message,
                    &mut commands,
                    &mut entities,
                    &mut meshes,
                    &mut materials,
                );
            }
        }
    }
}

fn handle_client_message(from: &ClientId, message: &ClientMessage) {
    _ = from;
    _ = message;
    // TODO handle client messages on the server, e.g. update player velocity based on input
}

fn handle_server_message(
    message: &ServerMessage,
    commands: &mut Commands,
    entities: &mut Query<(Entity, &NetId, &mut Transform), With<Creature>>,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<StandardMaterial>>,
) {
    match message {
        ServerMessage::Welcome { client_id } => {
            info!("Received Welcome, local ClientId assigned: {}", client_id.0);
        }
        ServerMessage::EntitySpawned {
            net_id,
            kind: _,
            position,
            velocity: _,
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

            commands.spawn((
                Mesh3d(meshes.add(Capsule3d::new(0.3, 1.0))),
                MeshMaterial3d(materials.add(StandardMaterial {
                    base_color: Color::srgb(0.8, 0.5, 0.2),
                    ..default()
                })),
                Transform::from_translation(pos),
                RigidBody::Kinematic,
                Collider::capsule(0.3, 1.0),
                LockedAxes::ROTATION_LOCKED.lock_translation_y(),
                GravityScale(0.0),
                NetId(net_id.0),
                Creature,
                MovementSpeed::default(),
                Thing,
                DespawnOnExit(AppState::InGame),
            ));
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
        ServerMessage::AssignControl { net_id } => {
            info!(
                "AssignControl: NetId({}) is our controlled entity",
                net_id.0
            );
            for (entity, id, _) in entities.iter() {
                if id.0 == net_id.0 {
                    commands.entity(entity).insert(PlayerControlled);
                    break;
                }
            }
        }
    }
}

/// System that applies remote input to entities controlled by remote clients.
fn apply_remote_input(
    mut messages: MessageReader<ServerEvent>,
    mut players: Query<(&ControlledByClient, &mut LinearVelocity, &MovementSpeed), With<Creature>>,
) {
    for event in messages.read() {
        if let ServerEvent::ClientMessageReceived { from, message } = event {
            let ClientMessage::Input { direction } = message;
            for (controlled_by, mut velocity, movement_speed) in players.iter_mut() {
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
            net_id: network::NetId(net_id.0),
            position: transform.translation.into(),
            velocity: [velocity.x, velocity.y, velocity.z],
        })
        .collect::<Vec<EntityState>>();

    if !states.is_empty() {
        sender.broadcast(&ServerMessage::StateUpdate { entities: states });
    }
}
