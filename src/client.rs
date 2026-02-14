use bevy::prelude::*;
use bevy::state::state_scoped::DespawnOnExit;
use network::{
    Client, ClientEvent, ClientMessage, ControlledByClient, InputDirection, NetClientSender, NetId,
    NetworkSet, ServerMessage, NETWORK_UPDATE_INTERVAL,
};
use things::{SpawnThing, Thing};

use crate::app_state::AppState;
use crate::camera::PlayerControlled;

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InputSendTimer>();
        app.init_resource::<LastSentDirection>();
        app.add_systems(
            PreUpdate,
            handle_client_events
                .run_if(resource_exists::<Client>)
                .after(NetworkSet::Receive)
                .before(NetworkSet::Send),
        );
        app.add_systems(Update, send_client_input.run_if(resource_exists::<Client>));
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

/// Tracks the last input direction sent to avoid redundant messages.
#[derive(Resource, Default)]
struct LastSentDirection(Vec3);

fn handle_client_events(
    mut commands: Commands,
    mut messages: MessageReader<ClientEvent>,
    mut next_state: ResMut<NextState<AppState>>,
    mut entities: Query<(Entity, &NetId, &mut Transform), With<Thing>>,
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
            }
            ClientEvent::Error(msg) => {
                error!("Network error: {msg}");
                next_state.set(AppState::MainMenu);
            }
            ClientEvent::ServerMessageReceived(message) => {
                handle_server_message(message, &mut commands, &mut entities, &mut client);
            }
        }
    }
}

fn handle_server_message(
    message: &ServerMessage,
    commands: &mut Commands,
    entities: &mut Query<(Entity, &NetId, &mut Transform), With<Thing>>,
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
            owner,
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

            let controlled = *owner == client.local_id && owner.is_some();
            let entity = commands
                .spawn((net_id.clone(), DespawnOnExit(AppState::InGame)))
                .id();
            commands.trigger(SpawnThing {
                entity,
                kind: *kind,
                position: pos,
                controlled,
            });

            if let Some(owner_id) = owner {
                commands
                    .entity(entity)
                    .insert(ControlledByClient(*owner_id));
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

/// System that sends client input direction to the server.
/// Reads InputDirection component (written by creatures module) and sends
/// via NetClientSender. Throttled to NETWORK_UPDATE_RATE to reduce traffic.
fn send_client_input(
    time: Res<Time>,
    mut timer: ResMut<InputSendTimer>,
    client_sender: Option<Res<NetClientSender>>,
    mut last_sent: ResMut<LastSentDirection>,
    query: Query<&InputDirection, With<PlayerControlled>>,
) {
    let Some(sender) = client_sender else {
        return;
    };

    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }

    let Ok(input) = query.single() else {
        return;
    };

    let direction = input.0;
    if direction == last_sent.0 {
        return;
    }

    last_sent.0 = direction;
    if !sender.send(&ClientMessage::Input {
        direction: direction.into(),
    }) {
        error!("Failed to send client input: send buffer full or closed");
    }
}
