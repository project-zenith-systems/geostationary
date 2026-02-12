use bevy::prelude::*;
use bevy::state::state_scoped::DespawnOnExit;
use network::{HostMessage, NetClientSender, NetEvent, NetServerSender, PeerId, PeerMessage, PeerState};
use physics::{Collider, GravityScale, LinearVelocity, LockedAxes, RigidBody};
use things::Thing;

use crate::app_state::AppState;
use crate::creatures::{Creature, MovementSpeed, PlayerControlled};

/// Network update rate in Hz (updates per second).
const NETWORK_UPDATE_RATE: f32 = 30.0;
const NETWORK_UPDATE_INTERVAL: f32 = 1.0 / NETWORK_UPDATE_RATE;

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

/// Timer for throttling state broadcasts from the server.
#[derive(Resource)]
struct StateBroadcastTimer(Timer);

impl Default for StateBroadcastTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(NETWORK_UPDATE_INTERVAL, TimerMode::Repeating))
    }
}

/// Timer for throttling client input sends.
#[derive(Resource)]
struct InputSendTimer(Timer);

impl Default for InputSendTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(NETWORK_UPDATE_INTERVAL, TimerMode::Repeating))
    }
}

/// Tracks whether we've already warned about missing host player to prevent log spam.
#[derive(Resource, Default)]
struct HostPlayerWarned(bool);

/// System that ensures the host player is correctly spawned/tagged for the local peer.
/// The host player uses the PeerId assigned by the server (typically PeerId(1)).
/// This system runs on the ListenServer whenever a LocalPeerId exists.
fn spawn_host_player(
    mut commands: Commands,
    network_role: Res<NetworkRole>,
    local_peer_id: Option<Res<LocalPeerId>>,
    players: Query<(Entity, &NetworkPeerId, Option<&PlayerControlled>)>,
    mut warned: ResMut<HostPlayerWarned>,
) {
    if *network_role != NetworkRole::ListenServer {
        return;
    }

    // Only proceed once we know our LocalPeerId.
    let Some(local_id) = local_peer_id else {
        return;
    };

    // Look for an existing entity for this peer and ensure it is player-controlled.
    for (entity, network_peer_id, player_controlled) in players.iter() {
        if network_peer_id.0 == local_id.0 {
            // If the entity exists but isn't yet marked as player-controlled, tag it now.
            if player_controlled.is_none() {
                commands.entity(entity).insert(PlayerControlled);
            }
            // Reset the warning flag if we found the entity
            warned.0 = false;
            return;
        }
    }

    // No existing entity for this peer. This shouldn't happen since handle_peer_connected
    // should have already spawned it when the peer connected.
    // Only warn once to avoid log spam.
    if !warned.0 {
        warn!(
            "spawn_host_player: No entity found for local peer {:?}. Waiting for handle_peer_connected to spawn it.",
            local_id.0
        );
        warned.0 = true;
    }
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

            // Send PeerJoined for the new peer itself so they spawn their own entity
            sender.send_to(
                *id,
                &HostMessage::PeerJoined {
                    id: *id,
                    position: spawn_pos.into(),
                },
            );

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

            // Send PeerJoined for the new peer to all existing peers (not the new peer itself)
            for (peer_id, _) in existing_peers.iter() {
                sender.send_to(
                    peer_id.0,
                    &HostMessage::PeerJoined {
                        id: *id,
                        position: spawn_pos.into(),
                    },
                );
            }
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
/// Throttled to NETWORK_UPDATE_RATE to reduce bandwidth usage.
fn broadcast_state(
    time: Res<Time>,
    mut timer: ResMut<StateBroadcastTimer>,
    server_sender: Option<Res<NetServerSender>>,
    players: Query<(&NetworkPeerId, &Transform, &LinearVelocity), With<Creature>>,
) {
    let Some(sender) = server_sender else {
        return;
    };

    // Throttle state broadcasts to NETWORK_UPDATE_RATE
    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }

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
/// Throttled to NETWORK_UPDATE_RATE to reduce network traffic.
fn send_client_input(
    time: Res<Time>,
    mut timer: ResMut<InputSendTimer>,
    keyboard: Res<ButtonInput<KeyCode>>,
    client_sender: Option<Res<NetClientSender>>,
) {
    let Some(sender) = client_sender else {
        return;
    };

    // Throttle input sends to NETWORK_UPDATE_RATE
    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }

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

    // Send input to server. Log if the send buffer is full.
    if !sender.send(&PeerMessage::Input {
        direction: direction.into(),
    }) {
        warn!("Failed to send client input: send buffer full or closed");
    }
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
    let mut local_id = local_peer_id.map(|id| id.0);

    for event in messages.read() {
        if let NetEvent::HostMessageReceived(message) = event {
            match message {
                HostMessage::Welcome { peer_id } => {
                    // Always update our local peer ID when receiving Welcome
                    // This ensures we have the correct ID even after reconnecting
                    commands.insert_resource(LocalPeerId(*peer_id));
                    // Update the local variable so subsequent messages in this frame see the new ID
                    local_id = Some(*peer_id);
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
        app.init_resource::<StateBroadcastTimer>();
        app.init_resource::<InputSendTimer>();
        app.init_resource::<HostPlayerWarned>();
        app.add_systems(
            Update,
            (
                // Tags the local player entity with PlayerControlled when running as ListenServer
                spawn_host_player,
                // Server-only systems: only run when acting as a listen server
                (
                    handle_peer_connected,
                    handle_peer_disconnected,
                    apply_remote_input,
                    broadcast_state,
                )
                    .run_if(resource_equals(NetworkRole::ListenServer)),
                // Client-only systems: only run when acting as a client
                (send_client_input, receive_host_messages)
                    .run_if(resource_equals(NetworkRole::Client)),
            )
                .run_if(in_state(AppState::InGame)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_network_role_default() {
        let role = NetworkRole::default();
        assert_eq!(role, NetworkRole::None);
    }

    #[test]
    fn test_network_role_equality() {
        assert_eq!(NetworkRole::None, NetworkRole::None);
        assert_eq!(NetworkRole::ListenServer, NetworkRole::ListenServer);
        assert_eq!(NetworkRole::Client, NetworkRole::Client);
        assert_ne!(NetworkRole::None, NetworkRole::ListenServer);
        assert_ne!(NetworkRole::ListenServer, NetworkRole::Client);
        assert_ne!(NetworkRole::Client, NetworkRole::None);
    }

    #[test]
    fn test_local_peer_id_construction() {
        let peer_id = PeerId(42);
        let local_peer_id = LocalPeerId(peer_id);
        assert_eq!(local_peer_id.0, peer_id);
    }

    #[test]
    fn test_network_peer_id_construction() {
        let peer_id = PeerId(123);
        let network_peer_id = NetworkPeerId(peer_id);
        assert_eq!(network_peer_id.0, peer_id);
    }

    #[test]
    fn test_network_peer_id_equality() {
        let peer_id1 = NetworkPeerId(PeerId(1));
        let peer_id2 = NetworkPeerId(PeerId(1));
        let peer_id3 = NetworkPeerId(PeerId(2));
        
        assert_eq!(peer_id1, peer_id2);
        assert_ne!(peer_id1, peer_id3);
    }

    #[test]
    fn test_state_broadcast_timer_default() {
        let timer = StateBroadcastTimer::default();
        assert!(!timer.0.finished());
        assert_eq!(timer.0.mode(), TimerMode::Repeating);
    }

    #[test]
    fn test_input_send_timer_default() {
        let timer = InputSendTimer::default();
        assert!(!timer.0.finished());
        assert_eq!(timer.0.mode(), TimerMode::Repeating);
    }

    #[test]
    fn test_network_update_rate_constants() {
        // Verify that NETWORK_UPDATE_INTERVAL is the inverse of NETWORK_UPDATE_RATE
        assert!((NETWORK_UPDATE_INTERVAL * NETWORK_UPDATE_RATE - 1.0).abs() < 0.0001);
    }
}
