use std::collections::HashMap;

use bevy::prelude::*;
use network::{
    Client, ClientEvent, ControlledByClient, EntityState, NetId, NetworkSet, ServerMessage,
    StreamDef, StreamDirection, StreamReader, StreamRegistry,
};
use physics::{Collider, GravityScale, LockedAxes, RigidBody};
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
    /// Authoritative spatial state update for all replicated things entities.
    StateUpdate { entities: Vec<EntityState> },
}

/// Fallback display name for entities whose [`ServerMessage::EntitySpawned`] carries no name.
const DEFAULT_DISPLAY_NAME: &str = "Unknown";

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
            (handle_entity_lifecycle, apply_state_updates)
                .run_if(resource_exists::<Client>)
                .after(NetworkSet::Receive)
                .before(NetworkSet::Send),
        );
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

/// Handles client-side entity lifecycle events from the control stream.
///
/// Processes: [`ServerMessage::Welcome`] (sets local client ID),
/// [`ServerMessage::EntitySpawned`] (spawns replica entity, inserts [`DisplayName`],
/// tracks in [`NetIdIndex`]), and [`ServerMessage::EntityDespawned`] (despawns,
/// removes from index).  Also clears the index and despawns all tracked entities
/// on disconnect.
fn handle_entity_lifecycle(
    mut commands: Commands,
    mut messages: MessageReader<ClientEvent>,
    mut net_id_index: ResMut<NetIdIndex>,
    mut client: ResMut<Client>,
) {
    for event in messages.read() {
        match event {
            ClientEvent::Disconnected { .. } => {
                for (_, entity) in net_id_index.0.drain() {
                    commands.entity(entity).despawn();
                }
            }
            ClientEvent::ServerMessageReceived(message) => match message {
                ServerMessage::Welcome {
                    client_id,
                    expected_streams,
                } => {
                    info!(
                        "Received Welcome, local ClientId: {}, expecting {} module stream(s)",
                        client_id.0, expected_streams
                    );
                    client.local_id = Some(*client_id);
                }
                ServerMessage::InitialStateDone => {
                    debug!("Server initial state done");
                }
                ServerMessage::EntitySpawned {
                    net_id,
                    kind,
                    position,
                    velocity: _,
                    owner,
                    name,
                } => {
                    if net_id_index.0.contains_key(net_id) {
                        debug!(
                            "EntitySpawned for NetId({}) already exists, skipping",
                            net_id.0
                        );
                        continue;
                    }

                    let pos = Vec3::from_array(*position);
                    info!("Spawning replica entity NetId({}) at {pos}", net_id.0);

                    let controlled = owner.is_some() && *owner == client.local_id;
                    let entity = commands.spawn(*net_id).id();
                    commands.trigger(SpawnThing {
                        entity,
                        kind: *kind,
                        position: pos,
                    });

                    let display_name = name
                        .as_deref()
                        .filter(|n| !n.is_empty())
                        .map(|n| DisplayName(n.to_string()))
                        .unwrap_or_else(|| DisplayName(DEFAULT_DISPLAY_NAME.to_string()));
                    commands.entity(entity).insert(display_name);

                    if controlled {
                        commands.entity(entity).insert(PlayerControlled);
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
                ServerMessage::StateUpdate { .. } => {
                    // State updates are now delivered on stream 3 via ThingsStreamMessage.
                }
            },
            _ => {}
        }
    }
}

/// Applies authoritative position updates from stream 3 to replicated thing entities.
fn apply_state_updates(
    mut reader: ResMut<StreamReader<ThingsStreamMessage>>,
    net_id_index: Res<NetIdIndex>,
    mut entities: Query<&mut Transform, With<Thing>>,
) {
    for msg in reader.drain() {
        match msg {
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
