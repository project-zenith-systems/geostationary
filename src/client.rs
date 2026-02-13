use bevy::prelude::*;
use bevy::state::state_scoped::DespawnOnExit;
use network::{ClientMessage, NetClientSender, NetEvent, NetId, ServerMessage};
use physics::{Collider, GravityScale, LockedAxes, RigidBody};
use things::Thing;

use crate::app_state::AppState;
use crate::creatures::{Creature, MovementSpeed, PlayerControlled};

/// Network update rate in Hz (updates per second).
const NETWORK_UPDATE_RATE: f32 = 30.0;
const NETWORK_UPDATE_INTERVAL: f32 = 1.0 / NETWORK_UPDATE_RATE;

/// Resource to track the state of the client.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Client {
    pub local_net_id: NetId,
}

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InputSendTimer>();
        app.add_systems(
            Update,
            (send_client_input, receive_server_messages).run_if(resource_exists::<Client>),
        );
    }
}

/// Timer for throttling client input sends.
#[derive(Resource)]
struct InputSendTimer(Timer);

impl Default for InputSendTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(
            NETWORK_UPDATE_INTERVAL,
            TimerMode::Repeating,
        ))
    }
}

/// System that sends client input to the server.
/// Reads keyboard and sends ClientMessage::Input via NetClientSender.
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

    if !sender.send(&ClientMessage::Input {
        direction: direction.into(),
    }) {
        error!("Failed to send client input: send buffer full or closed");
    }
}

/// Client-side system that receives server messages and manages local entities.
fn receive_server_messages(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    mut messages: MessageReader<NetEvent>,
    mut entities: Query<(Entity, &NetId, &mut Transform), With<Creature>>,
) {
    for event in messages.read() {
        let NetEvent::ServerMessageReceived(message) = event else {
            continue;
        };

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
                    continue;
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
}
