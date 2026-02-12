use bevy::prelude::*;
use bevy::state::state_scoped::DespawnOnExit;
use network::{HostMessage, NetClientSender, NetEvent, NetServerSender, PeerId, PeerMessage, PeerState};
use physics::{Collider, GravityScale, LinearVelocity, LockedAxes, RigidBody};
use things::Thing;

use crate::app_state::AppState;
use crate::creatures::{Creature, MovementSpeed, PlayerControlled};

/// Resource to track the network role of this instance.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkRole {
    None,
    ListenServer,
    Client,
}

impl Default for NetworkRole {
    fn default() -> Self {
        Self::None
    }
}

/// Resource to store the client's own PeerId.
#[derive(Resource, Debug, Clone, Copy)]
pub struct LocalPeerId(pub PeerId);

/// Component to associate an entity with a network peer.
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NetworkPeerId(pub PeerId);

/// System that spawns the host player when entering InGame as a ListenServer.
/// Runs on OnEnter(InGame) when NetworkRole is ListenServer.
fn spawn_host_player(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    network_role: Res<NetworkRole>,
) {
    if *network_role != NetworkRole::ListenServer {
        return;
    }

    // Spawn host player with PeerId(0)
    let player_mesh = meshes.add(Capsule3d::new(0.3, 1.0));
    let player_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.2, 0.5, 0.8),
        ..default()
    });

    commands.spawn((
        Mesh3d(player_mesh),
        MeshMaterial3d(player_material),
        Transform::from_xyz(6.0, 0.86, 5.0),
        RigidBody::Dynamic,
        Collider::capsule(0.3, 1.0),
        LockedAxes::ROTATION_LOCKED.lock_translation_y(),
        GravityScale(0.0),
        PlayerControlled,
        Creature,
        MovementSpeed::default(),
        NetworkPeerId(PeerId(0)),
        Thing,
        DespawnOnExit(AppState::InGame),
    ));
}

/// System that handles new peer connections.
/// Spawns remote player entities and sends Welcome/PeerJoined messages.
fn handle_peer_connected(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut messages: MessageReader<NetEvent>,
    server_sender: Option<Res<NetServerSender>>,
    existing_peers: Query<(&NetworkPeerId, &Transform)>,
) {
    let Some(sender) = server_sender else {
        return;
    };

    for event in messages.read() {
        if let NetEvent::PeerConnected { id, .. } = event {
            // Spawn remote player entity
            let player_mesh = meshes.add(Capsule3d::new(0.3, 1.0));
            let player_material = materials.add(StandardMaterial {
                base_color: Color::srgb(0.8, 0.5, 0.2), // Different color for remote players
                ..default()
            });

            let spawn_pos = Vec3::new(8.0, 0.86, 5.0); // Spawn at a different position

            commands.spawn((
                Mesh3d(player_mesh),
                MeshMaterial3d(player_material),
                Transform::from_translation(spawn_pos),
                RigidBody::Dynamic,
                Collider::capsule(0.3, 1.0),
                LockedAxes::ROTATION_LOCKED.lock_translation_y(),
                GravityScale(0.0),
                Creature,
                MovementSpeed::default(),
                NetworkPeerId(*id),
                Thing,
                DespawnOnExit(AppState::InGame),
            ));

            // Send Welcome message to the new peer
            sender.send_to(*id, &HostMessage::Welcome { peer_id: *id });

            // Send PeerJoined for all existing peers to the new peer
            for (peer_id, transform) in existing_peers.iter() {
                sender.send_to(
                    *id,
                    &HostMessage::PeerJoined {
                        id: peer_id.0,
                        position: transform.translation.into(),
                    },
                );
            }

            // Broadcast PeerJoined for the new peer to all existing peers
            sender.broadcast(&HostMessage::PeerJoined {
                id: *id,
                position: spawn_pos.into(),
            });
        }
    }
}

/// System that handles peer disconnections.
/// Despawns the entity and broadcasts PeerLeft.
fn handle_peer_disconnected(
    mut commands: Commands,
    mut messages: MessageReader<NetEvent>,
    server_sender: Option<Res<NetServerSender>>,
    peers: Query<(Entity, &NetworkPeerId)>,
) {
    let Some(sender) = server_sender else {
        return;
    };

    for event in messages.read() {
        if let NetEvent::PeerDisconnected { id } = event {
            // Find and despawn the entity
            for (entity, peer_id) in peers.iter() {
                if peer_id.0 == *id {
                    commands.entity(entity).despawn_recursive();
                    break;
                }
            }

            // Broadcast PeerLeft to all remaining peers
            sender.broadcast(&HostMessage::PeerLeft { id: *id });
        }
    }
}

/// System that applies remote input to remote player entities.
/// Reads PeerMessageReceived events and sets LinearVelocity.
fn apply_remote_input(
    mut messages: MessageReader<NetEvent>,
    mut players: Query<(&NetworkPeerId, &mut LinearVelocity, &MovementSpeed), With<Creature>>,
) {
    for event in messages.read() {
        if let NetEvent::PeerMessageReceived { from, message } = event {
            if let PeerMessage::Input { direction } = message {
                // Find the player entity for this peer
                for (peer_id, mut velocity, movement_speed) in players.iter_mut() {
                    if peer_id.0 == *from {
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
}

/// System that broadcasts state updates to all peers.
/// Collects all player positions and sends StateUpdate via NetServerSender.
fn broadcast_state(
    server_sender: Option<Res<NetServerSender>>,
    players: Query<(&NetworkPeerId, &Transform, &LinearVelocity), With<Creature>>,
) {
    let Some(sender) = server_sender else {
        return;
    };

    let peers: Vec<PeerState> = players
        .iter()
        .map(|(peer_id, transform, velocity)| PeerState {
            id: peer_id.0,
            position: transform.translation.into(),
            velocity: [velocity.x, velocity.y, velocity.z],
        })
        .collect();

    if !peers.is_empty() {
        sender.broadcast(&HostMessage::StateUpdate { peers });
    }
}

/// System that sends client input to the server.
/// Reads keyboard and sends PeerMessage::Input via NetClientSender.
fn send_client_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    client_sender: Option<Res<NetClientSender>>,
) {
    let Some(sender) = client_sender else {
        return;
    };

    let mut direction = Vec3::ZERO;

    if keyboard.pressed(KeyCode::KeyW) {
        direction.z -= 1.0;
    }
    if keyboard.pressed(KeyCode::KeyS) {
        direction.z += 1.0;
    }
    if keyboard.pressed(KeyCode::KeyA) {
        direction.x -= 1.0;
    }
    if keyboard.pressed(KeyCode::KeyD) {
        direction.x += 1.0;
    }

    // Always send input, even if it's zero (to stop movement)
    sender.send(&PeerMessage::Input {
        direction: direction.into(),
    });
}

/// System that receives host messages and updates client state.
/// Reads HostMessageReceived and spawns/despawns/updates entities.
fn receive_host_messages(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut messages: MessageReader<NetEvent>,
    local_peer_id: Option<Res<LocalPeerId>>,
    mut players: Query<(Entity, &NetworkPeerId, &mut Transform), With<Creature>>,
) {
    let local_id = local_peer_id.map(|id| id.0);

    for event in messages.read() {
        if let NetEvent::HostMessageReceived(message) = event {
            match message {
                HostMessage::Welcome { peer_id } => {
                    // Store our local peer ID
                    if local_id.is_none() {
                        commands.insert_resource(LocalPeerId(*peer_id));
                    }
                }
                HostMessage::PeerJoined { id, position } => {
                    // Check if this entity already exists
                    let exists = players.iter().any(|(_, peer_id, _)| peer_id.0 == *id);
                    if !exists {
                        let player_mesh = meshes.add(Capsule3d::new(0.3, 1.0));
                        let player_material = materials.add(StandardMaterial {
                            base_color: if Some(*id) == local_id {
                                Color::srgb(0.2, 0.5, 0.8) // Own player
                            } else {
                                Color::srgb(0.8, 0.5, 0.2) // Other player
                            },
                            ..default()
                        });

                        let pos = Vec3::from_array(*position);
                        let mut entity_commands = commands.spawn((
                            Mesh3d(player_mesh),
                            MeshMaterial3d(player_material),
                            Transform::from_translation(pos),
                            // All client-side entities use kinematic bodies because the server
                            // is authoritative for physics. Positions are updated from server state.
                            RigidBody::Kinematic,
                            Collider::capsule(0.3, 1.0),
                            LockedAxes::ROTATION_LOCKED.lock_translation_y(),
                            GravityScale(0.0),
                            NetworkPeerId(*id),
                            Creature,
                            MovementSpeed::default(),
                            Thing,
                            DespawnOnExit(AppState::InGame),
                        ));

                        // If this is our own player, add PlayerControlled so camera follows
                        if Some(*id) == local_id {
                            entity_commands.insert(PlayerControlled);
                        }
                    }
                }
                HostMessage::PeerLeft { id } => {
                    // Find and despawn the entity
                    for (entity, peer_id, _) in players.iter() {
                        if peer_id.0 == *id {
                            commands.entity(entity).despawn_recursive();
                            break;
                        }
                    }
                }
                HostMessage::StateUpdate { peers } => {
                    // Update positions for all peers
                    for peer_state in peers.iter() {
                        for (_, peer_id, mut transform) in players.iter_mut() {
                            if peer_id.0 == peer_state.id {
                                transform.translation = Vec3::from_array(peer_state.position);
                                break;
                            }
                        }
                    }
                }
            }
        }
    }
}

pub struct NetGamePlugin;

impl Plugin for NetGamePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<NetworkRole>();
        app.add_systems(OnEnter(AppState::InGame), spawn_host_player);
        app.add_systems(
            Update,
            (
                handle_peer_connected,
                handle_peer_disconnected,
                apply_remote_input,
                broadcast_state,
                send_client_input,
                receive_host_messages,
            )
                .run_if(in_state(AppState::InGame)),
        );
    }
}
