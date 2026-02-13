use bevy::prelude::*;
use network::{
    ClientId, ClientMessage, EntityState, NetId, NetServerSender, ServerEvent, ServerMessage,
};
use physics::LinearVelocity;

use crate::app_state::AppState;
use crate::creatures::{Creature, MovementSpeed};

/// Network update rate in Hz (updates per second).
const NETWORK_UPDATE_RATE: f32 = 30.0;
const NETWORK_UPDATE_INTERVAL: f32 = 1.0 / NETWORK_UPDATE_RATE;

pub struct ServerPlugin;

impl Plugin for ServerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Server>();
        app.init_resource::<StateBroadcastTimer>();
        app.add_systems(
            PreUpdate,
            receive_client_messages
                .after(network::NetworkSet::Receive)
                .before(network::NetworkSet::Send)
                .run_if(resource_exists::<Server>),
        );
        app.add_systems(
            Update,
            (apply_remote_input, broadcast_state).run_if(resource_exists::<Server>),
        );
    }
}

/// Resource to track the state of the server.
#[derive(Resource, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Server {
    next_net_id: u64,
}

impl Default for Server {
    fn default() -> Self {
        Self { next_net_id: 1 }
    }
}

impl Server {
    fn next_net_id(&mut self) -> NetId {
        let id = self.next_net_id;
        self.next_net_id += 1;
        NetId(id)
    }
}

/// Component: which client's input controls this entity (server-side only).
#[derive(Component, Debug, Clone, Copy)]
pub struct ControlledByClient(pub ClientId);

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

fn receive_client_messages(
    mut commands: Commands,
    mut messages: MessageReader<ServerEvent>,
    server_sender: Option<Res<NetServerSender>>,
    mut server: ResMut<Server>,
    existing_entities: Query<(&NetId, &Transform, &LinearVelocity)>,
) {
    let Some(sender) = server_sender else {
        return;
    };

    for event in messages.read() {
        let ServerEvent::ClientConnected { id, addr } = event else {
            continue;
        };

        info!("Client {} connected from {addr}", id.0);

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

        // let is_first_client = local_client_id.is_none();
        // let mut entity_cmds = commands.spawn((
        //     Mesh3d(meshes.add(Capsule3d::new(0.3, 1.0))),
        //     MeshMaterial3d(materials.add(StandardMaterial {
        //         base_color: Color::srgb(0.2, 0.5, 0.8),
        //         ..default()
        //     })),
        //     Transform::from_translation(spawn_pos),
        //     RigidBody::Dynamic,
        //     Collider::capsule(0.3, 1.0),
        //     LockedAxes::ROTATION_LOCKED.lock_translation_y(),
        //     GravityScale(0.0),
        //     Creature,
        //     MovementSpeed::default(),
        //     net_id,
        //     ControlledByClient(*id),
        //     Thing,
        //     DespawnOnExit(AppState::InGame),
        // ));

        // First client on ListenServer = host self-connection → local control
        // if is_first_client {
        //     info!(
        //         "First client: setting LocalClientId({}) and PlayerControlled",
        //         id.0
        //     );
        //     commands.insert_resource(LocalClientId(*id));
        //     entity_cmds.insert(PlayerControlled);
        // }

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
