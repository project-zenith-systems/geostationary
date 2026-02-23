use std::collections::HashMap;

use bevy::prelude::*;
use bevy::state::state_scoped::DespawnOnExit;
use network::{
    Client, ClientEvent, ClientMessage, ControlledByClient, NETWORK_UPDATE_INTERVAL,
    NetClientSender, NetId, NetworkSet, ServerMessage,
};
use player::PlayerControlled;
use things::{DisplayName, InputDirection, SpawnThing, Thing};
use tiles::{Tilemap, TILES_STREAM_TAG, decode_tiles_message};

use crate::app_state::AppState;

pub struct ClientPlugin;

impl Plugin for ClientPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<InputSendTimer>();
        app.init_resource::<LastSentDirection>();
        app.init_resource::<NetIdIndex>();
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

/// Maps NetId to Entity for O(1) lookup during StateUpdate processing.
#[derive(Resource, Default)]
struct NetIdIndex(HashMap<NetId, Entity>);

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
    mut net_id_index: ResMut<NetIdIndex>,
) {
    for event in messages.read() {
        match event {
            ClientEvent::Connected => {
                info!("Connected to server");
                next_state.set(AppState::InGame);
            }
            ClientEvent::Disconnected { reason } => {
                info!("Disconnected: {reason}");
                net_id_index.0.clear();
                next_state.set(AppState::MainMenu);
            }
            ClientEvent::Error(msg) => {
                error!("Network error: {msg}");
                net_id_index.0.clear();
                next_state.set(AppState::MainMenu);
            }
            ClientEvent::ServerMessageReceived(message) => {
                handle_server_message(
                    message,
                    &mut commands,
                    &mut entities,
                    &mut client,
                    &mut net_id_index,
                );
            }
            ClientEvent::StreamFrame { tag, data } if *tag == TILES_STREAM_TAG => {
                match decode_tiles_message(data) {
                    Ok(msg) => {
                        let tilemap = Tilemap::from_stream_message(msg);
                        info!(
                            "Received tilemap {}×{} from server",
                            tilemap.width(),
                            tilemap.height()
                        );
                        commands.insert_resource(tilemap);
                    }
                    Err(e) => {
                        warn!("Failed to decode TilesStreamMessage on stream {TILES_STREAM_TAG}: {e}");
                    }
                }
            }
            ClientEvent::StreamFrame { tag, data: _ } => {
                debug!("Stream frame received on tag={}", tag);
            }
            ClientEvent::StreamReady { tag } => {
                // TODO: count toward the initial-sync barrier once expected_streams > 0.
                debug!("Stream {} ready", tag);
            }
        }
    }
}

fn handle_server_message(
    message: &ServerMessage,
    commands: &mut Commands,
    entities: &mut Query<(Entity, &NetId, &mut Transform), With<Thing>>,
    client: &mut ResMut<Client>,
    net_id_index: &mut ResMut<NetIdIndex>,
) {
    match message {
        ServerMessage::Welcome { client_id, expected_streams } => {
            info!(
                "Received Welcome, local ClientId assigned: {}, expecting {} module stream(s)",
                client_id.0, expected_streams
            );
            client.local_id = Some(*client_id);
        }
        ServerMessage::InitialStateDone => {
            debug!("Server initial state done");
            // TODO: use as part of the initial-sync barrier once domain streams are implemented.
        }
        ServerMessage::EntitySpawned {
            net_id,
            kind,
            position,
            velocity: _,
            owner,
        } => {
            // Skip if entity already exists (e.g. duplicate message)
            if net_id_index.0.contains_key(net_id) {
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
                .spawn((*net_id, DespawnOnExit(AppState::InGame)))
                .id();
            commands.trigger(SpawnThing {
                entity,
                kind: *kind,
                position: pos,
            });

            if controlled {
                commands.entity(entity).insert((
                    PlayerControlled,
                    // TODO(spike-billboard): name will come from EntitySpawned once
                    // the network protocol gains the name field (souls plan step).
                    DisplayName("Player".to_string()),
                ));
            }

            if let Some(owner_id) = owner {
                commands
                    .entity(entity)
                    .insert(ControlledByClient(*owner_id));
            }

            net_id_index.0.insert(*net_id, entity);
        }
        ServerMessage::EntityDespawned { net_id } => {
            info!("Despawning replica entity NetId({})", net_id.0);
            if let Some(entity) = net_id_index.0.remove(net_id) {
                commands.entity(entity).despawn();
            }
        }
        ServerMessage::StateUpdate { entities: states } => {
            for state in states.iter() {
                if let Some(&entity) = net_id_index.0.get(&state.net_id)
                    && let Ok((_, _, mut transform)) = entities.get_mut(entity)
                {
                    transform.translation = Vec3::from_array(state.position);
                }
            }
        }
    }
}

/// System that sends client input direction to the server.
/// Reads InputDirection component (written by player module) and sends
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
    if let Err(e) = sender.send(&ClientMessage::Input {
        direction: direction.into(),
    }) {
        error!("Failed to send client input: {e}");
    }
}
