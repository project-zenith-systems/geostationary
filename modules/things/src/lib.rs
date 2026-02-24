use std::collections::HashMap;

use bevy::prelude::*;
use network::{
    Client, ClientId, ClientJoined, ControlledByClient, EntityState, NetId, NetworkSet, Server,
    StreamDef, StreamDirection, StreamReader, StreamRegistry, StreamSender, NETWORK_UPDATE_INTERVAL,
};
use physics::{Collider, GravityScale, LinearVelocity, LockedAxes, RigidBody};
use serde::{Deserialize, Serialize};
use wincode::{SchemaRead, SchemaWrite};

/// Marker component for non-grid-bound world objects.
#[derive(Component, Debug, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct Thing;

/// Marker component for the entity controlled by the local player.
#[derive(Component, Debug, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct PlayerControlled;

/// Display name for an entity, shown as a billboard nameplate in world space.
#[derive(Component, Debug, Clone, Default, Reflect)]
#[reflect(Component)]
pub struct DisplayName(pub String);

/// Current input direction for an entity. Written by input systems (player module)
/// or from received network messages (server). Read by creatures module
/// to apply velocity.
#[derive(Component, Debug, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct InputDirection(pub Vec3);

/// Entity event to construct the visual and physical representation of a thing.
/// The observer adds base components (mesh, physics, Thing marker) then runs
/// the template registered for the given `kind` via [`ThingRegistry`].
#[derive(EntityEvent)]
pub struct SpawnThing {
    pub entity: Entity,
    pub kind: u16,
    pub position: Vec3,
}

pub type ThingBuilder = Box<dyn Fn(Entity, &SpawnThing, &mut Commands) + Send + Sync>;

/// Registry mapping `kind` values to template callbacks that insert
/// type-specific components on a spawned entity.
#[derive(Resource, Default)]
pub struct ThingRegistry {
    templates: HashMap<u16, ThingBuilder>,
}

impl ThingRegistry {
    pub fn register(
        &mut self,
        kind: u16,
        builder: impl Fn(Entity, &SpawnThing, &mut Commands) + Send + Sync + 'static,
    ) {
        self.templates.insert(kind, Box::new(builder));
    }
}

/// Maps NetId to Entity for O(1) lookup during state-update processing.
#[derive(Resource, Default)]
pub struct NetIdIndex(pub HashMap<NetId, Entity>);

/// Stream 3 wire format: server→client messages for the things module.
#[derive(Debug, Clone, Serialize, Deserialize, SchemaRead, SchemaWrite)]
pub enum ThingsStreamMessage {
    /// A replicated entity was spawned.
    EntitySpawned {
        net_id: NetId,
        kind: u16,
        position: [f32; 3],
        velocity: [f32; 3],
        /// If set, the receiving client with this ID should take control of this entity.
        owner: Option<ClientId>,
        /// Optional display name for the entity (e.g. player name).
        name: Option<String>,
    },
    /// A replicated entity was despawned.
    EntityDespawned { net_id: NetId },
    /// Authoritative spatial state update for all replicated things entities.
    StateUpdate { entities: Vec<EntityState> },
}

/// Timer for throttling state broadcasts from the server.
#[derive(Resource)]
pub struct StateBroadcastTimer(pub Timer);

impl Default for StateBroadcastTimer {
    fn default() -> Self {
        Self(Timer::from_seconds(
            NETWORK_UPDATE_INTERVAL,
            TimerMode::Repeating,
        ))
    }
}

/// Plugin that registers the thing spawning system and shared entity primitives.
///
/// Must be added before any plugin that calls [`ThingRegistry::register`]
/// (e.g. `CreaturesPlugin`).
#[derive(Default)]
pub struct ThingsPlugin;

impl Plugin for ThingsPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Thing>();
        app.register_type::<PlayerControlled>();
        app.register_type::<InputDirection>();
        app.register_type::<DisplayName>();
        app.init_resource::<ThingRegistry>();
        app.init_resource::<NetIdIndex>();
        app.init_resource::<StateBroadcastTimer>();
        app.add_observer(on_spawn_thing);

        // Register stream 3 (server→client) with StreamRegistry.
        let (sender, reader) = app
            .world_mut()
            .resource_mut::<StreamRegistry>()
            .register::<ThingsStreamMessage>(StreamDef {
                tag: 3,
                name: "things",
                direction: StreamDirection::ServerToClient,
            });
        app.insert_resource(sender);
        app.insert_resource(reader);

        app.add_systems(
            PreUpdate,
            (
                handle_entity_lifecycle
                    .run_if(resource_exists::<Client>),
                handle_client_joined
                    .run_if(resource_exists::<Server>),
            )
                .after(NetworkSet::Receive)
                .before(NetworkSet::Send),
        );
        app.add_systems(Update, broadcast_state.run_if(resource_exists::<Server>));
    }
}

fn on_spawn_thing(
    on: On<SpawnThing>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    registry: Res<ThingRegistry>,
) {
    let event = on.event();

    commands.entity(event.entity).insert((
        Mesh3d(meshes.add(Capsule3d::new(0.3, 1.0))),
        MeshMaterial3d(materials.add(StandardMaterial {
            base_color: Color::srgb(0.8, 0.5, 0.2),
            ..default()
        })),
        Transform::from_translation(event.position),
        RigidBody::Dynamic,
        Collider::capsule(0.3, 1.0),
        LockedAxes::ROTATION_LOCKED.lock_translation_y(),
        GravityScale(0.0),
        Thing,
    ));

    if let Some(builder) = registry.templates.get(&event.kind) {
        builder(event.entity, event, &mut commands);
    } else {
        warn!("No template registered for thing kind {}", event.kind);
    }
}

/// Handles client-side entity lifecycle and state updates from stream 3.
///
/// Processes all [`ThingsStreamMessage`] frames:
/// - [`ThingsStreamMessage::EntitySpawned`]: spawns replica entity via [`SpawnThing`],
///   inserts [`DisplayName`], and tracks it in [`NetIdIndex`].
/// - [`ThingsStreamMessage::EntityDespawned`]: despawns the entity and removes it from
///   the index. [`DespawnOnExit`] provides additional state-transition cleanup.
/// - [`ThingsStreamMessage::StateUpdate`]: applies authoritative position updates.
fn handle_entity_lifecycle(
    mut commands: Commands,
    mut reader: ResMut<StreamReader<ThingsStreamMessage>>,
    mut net_id_index: ResMut<NetIdIndex>,
    client: Res<Client>,
    mut entities: Query<&mut Transform, With<Thing>>,
) {
    for msg in reader.drain() {
        match msg {
            ThingsStreamMessage::EntitySpawned {
                net_id,
                kind,
                position,
                velocity: _,
                owner,
                name,
            } => {
                if net_id_index.0.contains_key(&net_id) {
                    warn!(
                        "EntitySpawned for NetId({}) already exists, skipping",
                        net_id.0
                    );
                    continue;
                }

                let pos = Vec3::from_array(position);
                info!("Spawning entity NetId({}) at {pos}", net_id.0);

                let controlled = owner.is_some() && owner == client.local_id;
                let entity = commands.spawn(net_id).id();
                commands.trigger(SpawnThing {
                    entity,
                    kind,
                    position: pos,
                });

                if let Some(n) = name.as_deref().filter(|n| !n.is_empty()) {
                    commands.entity(entity).insert(DisplayName(n.to_string()));
                }

                if controlled {
                    commands.entity(entity).insert(PlayerControlled);
                }

                if let Some(owner_id) = owner {
                    commands.entity(entity).insert(ControlledByClient(owner_id));
                }

                net_id_index.0.insert(net_id, entity);
            }
            ThingsStreamMessage::EntityDespawned { net_id } => {
                info!("Despawning entity NetId({})", net_id.0);
                if let Some(entity) = net_id_index.0.remove(&net_id) {
                    commands.entity(entity).despawn();
                }
            }
            ThingsStreamMessage::StateUpdate { entities: states } => {
                for state in &states {
                    if let Some(&entity) = net_id_index.0.get(&state.net_id) {
                        if let Ok(mut transform) = entities.get_mut(entity) {
                            transform.translation = Vec3::from_array(state.position);
                        }
                    }
                }
            }
        }
    }
}

/// Handles server-side entity spawning in response to a client joining.
///
/// Sends catch-up [`ThingsStreamMessage::EntitySpawned`] messages for all currently
/// tracked entities to the joining client, then assigns a new [`NetId`] and broadcasts
/// the new player entity to all clients.
fn handle_client_joined(
    mut messages: MessageReader<ClientJoined>,
    mut server: ResMut<Server>,
    stream_sender: Res<StreamSender<ThingsStreamMessage>>,
    entities: Query<(&NetId, &ControlledByClient, &Transform, &LinearVelocity)>,
) {
    for joined in messages.read() {
        let from = joined.id;

        // Catch-up: send EntitySpawned on stream 3 for every existing entity.
        for (net_id, controlled_by, transform, velocity) in entities.iter() {
            if let Err(e) = stream_sender.send_to(
                from,
                &ThingsStreamMessage::EntitySpawned {
                    net_id: *net_id,
                    kind: 0,
                    position: transform.translation.into(),
                    velocity: [velocity.x, velocity.y, velocity.z],
                    owner: if controlled_by.0 == from {
                        Some(from)
                    } else {
                        None
                    },
                    name: None,
                },
            ) {
                error!(
                    "Failed to send EntitySpawned catch-up to ClientId({}): {e}",
                    from.0
                );
            }
        }

        // Spawn the new player entity and broadcast it to all clients.
        let net_id = server.next_net_id();
        let spawn_pos = Vec3::new(6.0, 0.81, 3.0);
        info!(
            "Spawning player entity NetId({}) for ClientId({}) at {spawn_pos}",
            net_id.0, from.0
        );

        if let Err(e) = stream_sender.broadcast(&ThingsStreamMessage::EntitySpawned {
            net_id,
            kind: 0,
            position: spawn_pos.into(),
            velocity: [0.0, 0.0, 0.0],
            owner: Some(from),
            name: None,
        }) {
            error!(
                "Failed to broadcast EntitySpawned for NetId({}): {e}",
                net_id.0
            );
        }
    }
}

/// Broadcasts authoritative position updates for all tracked entities on stream 3.
///
/// Throttled to [`NETWORK_UPDATE_INTERVAL`] to reduce bandwidth.
fn broadcast_state(
    time: Res<Time>,
    mut timer: ResMut<StateBroadcastTimer>,
    stream_sender: Res<StreamSender<ThingsStreamMessage>>,
    entities: Query<(&NetId, &Transform, &LinearVelocity)>,
) {
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
        .collect::<Vec<_>>();

    if !states.is_empty() {
        if let Err(e) =
            stream_sender.broadcast(&ThingsStreamMessage::StateUpdate { entities: states })
        {
            error!("Failed to broadcast entity state on things stream: {e}");
        }
    }
}

