use bevy::prelude::*;
use network::{
    Client, ClientId, ClientInputReceived, ClientJoined, ClientLeft, ControlledByClient,
    NetClientSender, NetworkSet, Server, StreamSender, NETWORK_UPDATE_INTERVAL,
};
use things::{DisplayName, InputDirection, SpawnThing, ThingsSet, ThingsStreamMessage};

/// Component placed on a dedicated soul entity to bind a client to a creature.
///
/// A soul is not a world entity — it carries no `Transform`, no physics, and no mesh.
/// It exists purely as the server-side binding between a [`ClientId`] and the creature
/// [`Entity`] it controls.  All three fields are always required; they are never set
/// independently.
#[derive(Component, Debug)]
pub struct Soul {
    /// Display name sent by the client in its `Hello` message.
    pub name: String,
    /// The network client this soul belongs to.
    pub client_id: ClientId,
    /// The creature entity this soul is currently bound to.
    pub bound_to: Entity,
}

/// The things stream sender resource type, used by the souls module to broadcast
/// `EntitySpawned` for newly bound creatures.
type ThingsStreamSenderRes = StreamSender<ThingsStreamMessage>;

pub struct SoulsPlugin;

impl Plugin for SoulsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            PreUpdate,
            (
                bind_soul
                    .run_if(resource_exists::<Server>)
                    .after(ThingsSet::HandleClientJoined),
                unbind_soul.run_if(resource_exists::<Server>),
                route_input.run_if(resource_exists::<Server>),
            )
                .after(NetworkSet::Receive)
                .before(NetworkSet::Send),
        );
        app.add_systems(Update, send_input.run_if(resource_exists::<Client>));
        app.init_resource::<InputSendTimer>();
        app.init_resource::<LastSentDirection>();
    }
}

/// Server-side system: on `ClientJoined`, spawn a soul entity and a creature entity,
/// set `DisplayName` and `ControlledByClient` on the creature, then broadcast
/// `EntitySpawned` on stream 3 so all clients (including the joining one) see the new creature.
///
/// Runs after [`ThingsSet::HandleClientJoined`] so the initial `StreamReady` for stream 3
/// has already been sent to the joining client before this broadcasts the new entity.
fn bind_soul(
    mut commands: Commands,
    mut joined: MessageReader<ClientJoined>,
    mut server: ResMut<Server>,
    stream_sender: Res<ThingsStreamSenderRes>,
) {
    for ClientJoined { id, name } in joined.read() {
        let net_id = server.next_net_id();
        let spawn_pos = Vec3::new(6.0, 0.81, 3.0);

        info!(
            "Binding soul for ClientId({}) '{}': spawning creature NetId({})",
            id.0, name, net_id.0
        );

        // Spawn the creature entity.
        let creature = commands
            .spawn((net_id, ControlledByClient(*id), InputDirection::default()))
            .id();
        commands.trigger(SpawnThing {
            entity: creature,
            kind: 0,
            position: spawn_pos,
        });
        commands.entity(creature).insert(DisplayName(name.clone()));

        // Spawn the soul entity that binds this client to the creature.
        commands.spawn(Soul {
            name: name.clone(),
            client_id: *id,
            bound_to: creature,
        });

        // Broadcast EntitySpawned to all clients so they see the new creature.
        if let Err(e) = stream_sender.broadcast(&ThingsStreamMessage::EntitySpawned {
            net_id,
            kind: 0,
            position: spawn_pos.into(),
            velocity: [0.0, 0.0, 0.0],
            owner: Some(*id),
            name: Some(name.clone()),
        }) {
            error!(
                "Failed to broadcast EntitySpawned for NetId({}): {e}",
                net_id.0
            );
        }
    }
}

/// Server-side system: on [`ClientLeft`], despawn the soul entity and clear
/// `InputDirection` on the bound creature so it stops moving.
///
/// The creature entity itself remains in the world — it keeps its `Thing`, `Creature`,
/// `DisplayName`, `NetId`, and physics components and will continue to appear in
/// `StateUpdate` broadcasts (standing still because `InputDirection` is zeroed).
fn unbind_soul(
    mut commands: Commands,
    mut events: MessageReader<ClientLeft>,
    souls: Query<(Entity, &Soul)>,
    mut input_dirs: Query<&mut InputDirection>,
) {
    for ClientLeft { id } in events.read() {
        for (soul_entity, soul) in souls.iter() {
            if soul.client_id == *id {
                info!(
                    "Unbinding soul for ClientId({}): despawning soul entity, clearing InputDirection",
                    id.0
                );

                // Clear the creature's input so it stops moving.
                if let Ok(mut input_dir) = input_dirs.get_mut(soul.bound_to) {
                    input_dir.0 = Vec3::ZERO;
                }

                commands.entity(soul_entity).despawn();
                break;
            }
        }
    }
}

/// Server-side system: routes [`ClientInputReceived`] messages to the `InputDirection`
/// component on the creature bound to that client's soul.
fn route_input(
    mut events: MessageReader<ClientInputReceived>,
    souls: Query<&Soul>,
    mut input_dirs: Query<&mut InputDirection>,
) {
    for ClientInputReceived { from, direction } in events.read() {
        for soul in souls.iter() {
            if soul.client_id == *from {
                if let Ok(mut input_dir) = input_dirs.get_mut(soul.bound_to) {
                    input_dir.0 = Vec3::from_array(*direction);
                }
                break;
            }
        }
    }
}

/// Timer for throttling outbound `Input` messages from the client.
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

/// Last direction sent to avoid redundant `Input` messages.
#[derive(Resource, Default)]
struct LastSentDirection(Vec3);

/// Client-side system: reads `InputDirection` from the `PlayerControlled` creature and
/// sends `ClientMessage::Input` to the server via the control stream.
///
/// Throttled to [`NETWORK_UPDATE_INTERVAL`] and skips sends when the direction is unchanged.
fn send_input(
    time: Res<Time>,
    mut timer: ResMut<InputSendTimer>,
    client_sender: Option<Res<NetClientSender>>,
    mut last_sent: ResMut<LastSentDirection>,
    query: Query<&InputDirection, With<things::PlayerControlled>>,
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
    if let Err(e) = sender.send(&network::ClientMessage::Input {
        direction: direction.into(),
    }) {
        error!("Failed to send client input: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that [`Soul`] can be constructed with all required fields.
    #[test]
    fn soul_component_fields() {
        use network::ClientId;

        // Just verify the Soul struct can be constructed with all required fields.
        let dummy_entity = Entity::from_raw(1);
        let soul = Soul {
            name: "Alice".to_string(),
            client_id: ClientId(42),
            bound_to: dummy_entity,
        };

        assert_eq!(soul.name, "Alice");
        assert_eq!(soul.client_id, ClientId(42));
        assert_eq!(soul.bound_to, dummy_entity);
    }
}
