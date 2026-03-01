use std::collections::HashMap;

use bevy::prelude::*;
use input::{PointerAction, WorldHit};
use network::{
    Client, ClientId, ControlledByClient, EntityState, Headless, ModuleReadySent, NetId,
    NetworkSet, PlayerEvent, Server, StreamDef, StreamDirection, StreamReader, StreamRegistry,
    StreamSender, NETWORK_UPDATE_INTERVAL,
};
use physics::{Collider, LinearVelocity, RigidBody, SpatialQuery, SpatialQueryFilter};
use serde::{Deserialize, Serialize};
use wincode::{SchemaRead, SchemaWrite};

/// System set for the things module's server-side lifecycle systems.
/// Other modules can use this for explicit ordering relative to things systems.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum ThingsSet {
    /// Sends catch-up [`ThingsStreamMessage::EntitySpawned`] messages and [`StreamReady`] to a joining client.
    HandleClientJoined,
}

/// Marker component for non-grid-bound world objects.
#[derive(Component, Debug, Clone, Copy, Reflect)]
#[reflect(Component)]
pub struct Thing {
    /// Kind index used to look up the registered [`ThingRegistry`] template.
    /// Kind 0 = creature, kind 1 = ball, etc.
    pub kind: u16,
}

/// Tracks the last position and velocity that was broadcast for this entity.
/// `broadcast_state` compares current values against these to skip unchanged
/// entities — Bevy's `Changed<Transform>` cannot be used because the physics
/// engine writes to `Transform` every frame even for resting bodies.
#[derive(Component, Default)]
struct LastBroadcast {
    position: Vec3,
    velocity: Vec3,
}

/// Which hand a [`HandSlot`] anchor belongs to.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect)]
pub enum HandSide {
    Left,
    Right,
}

/// Child entity anchor marking where a held item attaches.
///
/// Spawned as a child of every creature entity by the kind 0 [`ThingRegistry`]
/// template. The items module will later add a `Container { capacity: 1 }` to
/// this child once that component is defined.
#[derive(Component, Debug, Clone, Copy, Reflect)]
#[reflect(Component)]
pub struct HandSlot {
    pub side: HandSide,
}

/// Creature-local (local-space) offset from the creature origin to the hand anchor position.
pub const HAND_OFFSET: Vec3 = Vec3::new(0.4, 0.5, 0.0);

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

/// Bevy message fired by the items module after each successful item operation.
///
/// Defined here (in `things`) rather than in `items` to avoid a circular crate
/// dependency: `items` already depends on `things`.  Variants use only [`Entity`]
/// and [`Vec3`] — no item-specific types.
///
/// A future `broadcast_item_event` system in `things` will read this and
/// replicate the action to connected clients.
#[derive(Message, Clone, Debug)]
pub enum ItemActionEvent {
    /// An item was picked up and placed into a hand slot.
    PickedUp { item: Entity, hand: Entity },
    /// An item was dropped from a hand slot at the given world position.
    Dropped { item: Entity, position: Vec3 },
    /// An item was moved from a hand slot into a container.
    Stored { item: Entity, container: Entity },
    /// An item was taken from a container into a hand slot.
    Taken { item: Entity, hand: Entity },
}

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

/// Spawns a thing entity with a server-assigned [`NetId`] and triggers [`SpawnThing`]
/// so that the registered template for the given `kind` adds type-specific components.
///
/// Calls [`Server::next_net_id`] internally — callers must not pre-allocate the id.
///
/// Returns the spawned [`Entity`] and its assigned [`NetId`].
pub fn spawn_thing(
    commands: &mut Commands,
    server: &mut Server,
    kind: u16,
    position: Vec3,
) -> (Entity, NetId) {
    let net_id = server.next_net_id();
    let entity = commands.spawn(net_id).id();
    commands.trigger(SpawnThing {
        entity,
        kind,
        position,
    });
    // Register in NetIdIndex so that a listen-server's client-side
    // handle_entity_lifecycle sees this entity as already spawned and
    // skips the duplicate EntitySpawned from the catch-up burst.
    commands.queue(move |world: &mut World| {
        world.resource_mut::<NetIdIndex>().0.insert(net_id, entity);
    });
    (entity, net_id)
}

/// Spawns a player-controlled thing entity with a server-assigned [`NetId`],
/// [`ControlledByClient`], [`InputDirection`], and [`DisplayName`], then triggers
/// [`SpawnThing`] so that the registered template (kind 0 = creature) adds physics
/// and type-specific components.
///
/// Returns the spawned [`Entity`] and its assigned [`NetId`].
///
/// Called by the `souls` module when binding a soul to a newly connected client.
pub fn spawn_player_creature(
    commands: &mut Commands,
    server: &mut Server,
    owner: ClientId,
    position: Vec3,
    display_name: &str,
) -> (Entity, NetId) {
    let (creature, net_id) = spawn_thing(commands, server, 0, position);
    commands.entity(creature).insert((
        ControlledByClient(owner),
        InputDirection::default(),
        DisplayName(display_name.to_string()),
    ));
    (creature, net_id)
}

/// Plugin that registers the thing spawning system and shared entity primitives.
///
/// Must be added before any plugin that calls [`ThingRegistry::register`]
/// (e.g. `CreaturesPlugin`).
pub struct ThingsPlugin<S: States + Copy> {
    state: S,
}

impl<S: States + Copy> ThingsPlugin<S> {
    pub fn in_state(state: S) -> Self {
        Self { state }
    }
}

impl<S: States + Copy> Plugin for ThingsPlugin<S> {
    fn build(&self, app: &mut App) {
        app.register_type::<Thing>();
        app.register_type::<HandSide>();
        app.register_type::<HandSlot>();
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
            .get_resource_mut::<StreamRegistry>()
            .expect("ThingsPlugin requires NetworkPlugin to be added before it (StreamRegistry not found)")
            .register::<ThingsStreamMessage>(StreamDef {
                tag: 3,
                name: "things",
                direction: StreamDirection::ServerToClient,
            });
        app.insert_resource(sender);
        app.insert_resource(reader);

        let state = self.state;
        app.add_systems(
            PreUpdate,
            (
                handle_entity_lifecycle
                    .run_if(resource_exists::<Client>),
                handle_client_joined
                    .run_if(resource_exists::<Server>)
                    .in_set(ThingsSet::HandleClientJoined),
            )
                .after(NetworkSet::Receive)
                .before(NetworkSet::Send),
        );
        app.add_systems(Update, broadcast_state.run_if(resource_exists::<Server>));

        // Register the messages raycast_things reads/writes so the resources
        // exist even when InputPlugin is not added (e.g. headless server mode).
        app.add_message::<PointerAction>();
        app.add_message::<WorldHit>();
        // ItemActionEvent is defined here so items can fire it without a circular dependency.
        app.add_message::<ItemActionEvent>();
        app.add_systems(
            Update,
            raycast_things
                .run_if(in_state(state))
                .run_if(not(resource_exists::<Headless>)),
        );
    }
}

fn on_spawn_thing(
    on: On<SpawnThing>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: Option<ResMut<Assets<StandardMaterial>>>,
    registry: Res<ThingRegistry>,
) {
    let event = on.event();

    let mut entity = commands.entity(event.entity);
    entity.insert((
        Transform::from_translation(event.position),
        RigidBody::Dynamic,
        LinearVelocity::default(),
        Collider::capsule(0.3, 1.0),
        Thing { kind: event.kind },
        LastBroadcast::default(),
    ));

    // Insert visual components only when the renderer (PbrPlugin) is available.
    // In headless server mode, Assets<StandardMaterial> is not registered.
    if let Some(ref mut materials) = materials {
        entity.insert((
            Mesh3d(meshes.add(Capsule3d::new(0.3, 1.0))),
            MeshMaterial3d(materials.add(StandardMaterial {
                base_color: Color::srgb(0.8, 0.5, 0.2),
                ..default()
            })),
        ));
    }

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
    server: Option<Res<Server>>,
    mut entities: Query<&mut Transform, With<Thing>>,
) {
    let is_listen_server = server.is_some();
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
                let controlled = owner.is_some() && owner == client.local_id;

                // On a listen-server the entity was already spawned server-side
                // and pre-registered in NetIdIndex. Skip the spawn but still
                // apply client-only components to the existing entity.
                if let Some(&existing) = net_id_index.0.get(&net_id) {
                    debug!(
                        "EntitySpawned for NetId({}) already exists, applying client components",
                        net_id.0
                    );
                    if let Some(n) = name.as_deref().filter(|n| !n.is_empty()) {
                        commands.entity(existing).insert(DisplayName(n.to_string()));
                    }
                    if controlled {
                        commands.entity(existing).insert(PlayerControlled);
                    }
                    if let Some(owner_id) = owner {
                        commands.entity(existing).insert(ControlledByClient(owner_id));
                    }
                    continue;
                }

                let pos = Vec3::from_array(position);
                info!("Spawning entity NetId({}) at {pos}", net_id.0);

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
                // On a listen-server the transforms are already authoritative;
                // re-applying them would trigger Changed<Transform> and re-dirty
                // every entity each frame.
                if is_listen_server {
                    continue;
                }
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

/// Handles server-side catch-up on client join for stream 3.
///
/// Sends catch-up [`ThingsStreamMessage::EntitySpawned`] messages for all currently
/// tracked entities to the joining client, then sends the [`StreamReady`] sentinel for stream 3
/// so the client can count toward its initial-sync barrier.
///
/// Creature spawning and the EntitySpawned broadcast for the new player entity are handled
/// by the `souls` module's `bind_soul` system.
fn handle_client_joined(
    mut messages: MessageReader<PlayerEvent>,
    stream_sender: Res<StreamSender<ThingsStreamMessage>>,
    entities: Query<(
        &NetId,
        Option<&ControlledByClient>,
        &Transform,
        &LinearVelocity,
        Option<&DisplayName>,
        &Thing,
    )>,
    mut module_ready: MessageWriter<ModuleReadySent>,
) {
    for event in messages.read() {
        let PlayerEvent::Joined { id: from, .. } = event else {
            continue;
        };

        // Catch-up: send EntitySpawned on stream 3 for every existing Thing entity.
        for (net_id, opt_controlled_by, transform, velocity, opt_name, thing) in entities.iter() {
            let owner = opt_controlled_by
                .map(|c| c.0)
                .filter(|&owner_id| owner_id == *from);
            if let Err(e) = stream_sender.send_to(
                *from,
                &ThingsStreamMessage::EntitySpawned {
                    net_id: *net_id,
                    kind: thing.kind,
                    position: transform.translation.into(),
                    velocity: [velocity.x, velocity.y, velocity.z],
                    owner,
                    name: opt_name.map(|n| n.0.clone()),
                },
            ) {
                error!(
                    "Failed to send EntitySpawned catch-up to ClientId({}): {e}",
                    from.0
                );
            }
        }

        // Signal that the initial burst for stream 3 is complete.
        // The new player entity's EntitySpawned is broadcast by the souls module after this.
        if let Err(e) = stream_sender.send_stream_ready_to(*from) {
            error!(
                "Failed to send StreamReady for things stream to ClientId({}): {e}",
                from.0
            );
        } else {
            module_ready.write(ModuleReadySent { client: *from });
        }
    }
}

/// Broadcasts authoritative position updates on stream 3 for entities whose
/// state has changed since the last broadcast.
///
/// Throttled to [`NETWORK_UPDATE_INTERVAL`] to reduce bandwidth.
/// Compares current position/velocity against [`LastBroadcast`] to skip
/// unchanged entities.
const POSITION_EPSILON_SQ: f32 = 1e-6;
const VELOCITY_EPSILON_SQ: f32 = 1e-6;

fn broadcast_state(
    time: Res<Time>,
    mut timer: ResMut<StateBroadcastTimer>,
    stream_sender: Res<StreamSender<ThingsStreamMessage>>,
    mut entities: Query<(&NetId, &Transform, Option<&LinearVelocity>, &mut LastBroadcast)>,
) {
    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }

    let states: Vec<EntityState> = entities
        .iter_mut()
        .filter_map(|(net_id, transform, velocity, mut last)| {
            let pos = transform.translation;
            let vel = velocity.map(|lv| lv.0).unwrap_or(Vec3::ZERO);

            let pos_changed = (pos - last.position).length_squared() > POSITION_EPSILON_SQ;
            let vel_changed = (vel - last.velocity).length_squared() > VELOCITY_EPSILON_SQ;

            if !pos_changed && !vel_changed {
                return None;
            }

            last.position = pos;
            last.velocity = vel;

            Some(EntityState {
                net_id: *net_id,
                position: pos.into(),
                velocity: [vel.x, vel.y, vel.z],
            })
        })
        .collect();

    if !states.is_empty() {
        if let Err(e) =
            stream_sender.broadcast(&ThingsStreamMessage::StateUpdate { entities: states })
        {
            error!("Failed to broadcast entity state on things stream: {e}");
        }
    }
}

/// Listens for left-click and right-click [`PointerAction`] events, raycasts against entity
/// colliders via [`SpatialQuery`], and emits [`WorldHit`] for the nearest hit thing entity.
fn raycast_things(
    mut pointer_action_reader: MessageReader<PointerAction>,
    spatial_query: SpatialQuery,
    camera: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    things: Query<&Thing>,
    mut hit_writer: MessageWriter<WorldHit>,
) {
    let Ok((camera, camera_transform)) = camera.single() else {
        return;
    };

    for action in pointer_action_reader.read() {
        if !matches!(action.button, MouseButton::Left | MouseButton::Right) {
            continue;
        }

        let Ok(ray) = camera.viewport_to_world(camera_transform, action.screen_pos) else {
            continue;
        };

        if let Some(hit) = spatial_query.cast_ray(
            ray.origin,
            ray.direction,
            f32::MAX,
            true,
            &SpatialQueryFilter::default(),
        ) {
            if things.get(hit.entity).is_ok() {
                let world_pos = ray.origin + *ray.direction * hit.distance;
                hit_writer.write(WorldHit {
                    button: action.button,
                    entity: hit.entity,
                    world_pos,
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifies that when a kind-0 SpawnThing event is triggered with a builder
    /// that spawns a HandSlot child (as CreaturesPlugin does), the creature entity
    /// ends up with a child entity carrying HandSlot { side: Right }.
    #[test]
    fn spawn_thing_kind_0_produces_hand_slot_child() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        // on_spawn_thing requires Assets<Mesh>; insert a bare resource to satisfy it.
        app.insert_resource(Assets::<Mesh>::default());
        app.init_resource::<ThingRegistry>();
        app.add_observer(on_spawn_thing);

        // Register a kind-0 builder that mirrors what CreaturesPlugin registers:
        // spawn a HandSlot child on the creature entity.
        app.world_mut()
            .resource_mut::<ThingRegistry>()
            .register(0, |entity, _event, commands| {
                commands.entity(entity).with_children(|parent| {
                    parent.spawn((
                        HandSlot {
                            side: HandSide::Right,
                        },
                        Transform::from_translation(HAND_OFFSET),
                    ));
                });
            });

        let creature = app.world_mut().spawn_empty().id();
        app.world_mut().trigger(SpawnThing {
            entity: creature,
            kind: 0,
            position: Vec3::ZERO,
        });
        app.update();

        let children = app
            .world()
            .entity(creature)
            .get::<Children>()
            .expect("creature should have at least one child after SpawnThing");

        let hand_slot_child = children
            .iter()
            .find_map(|child| app.world().get::<HandSlot>(child));

        let slot = hand_slot_child.expect("creature should have a HandSlot child entity");
        assert_eq!(
            slot.side,
            HandSide::Right,
            "HandSlot side should be Right"
        );
    }
}

