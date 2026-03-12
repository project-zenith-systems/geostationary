use std::collections::HashMap;
use std::sync::Arc;

use bevy::prelude::*;
use bevy::state::state_scoped::DespawnOnExit;
use input::{PointerAction, WorldHit};
use network::{
    Client, ClientId, ControlledByClient, EntityState, Headless, ModuleReadySent,
    NETWORK_UPDATE_INTERVAL, NetId, NetworkReceive, NetworkSend, PlayerEvent, Server, StreamDef,
    StreamDirection, StreamReader, StreamRegistry, StreamSender,
};
use physics::{GravityScale, LinearVelocity, RigidBody, SpatialQuery, SpatialQueryFilter};
use ron::value::RawValue;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use wincode::{SchemaRead, SchemaWrite};
use world::{MapLayer, MapLayerRegistryExt, from_layer_value, to_layer_value};

use animation::{AnimState, HoldIk};

/// System set for the things module's server-side lifecycle systems.
/// Other modules can use this for explicit ordering relative to things systems.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum ThingsSet {
    /// Sends catch-up [`ThingsStreamMessage::EntitySpawned`] messages to a joining client.
    HandleClientJoined,
    /// Sends the [`StreamReady`] sentinel for stream 3 to a joining client.
    ///
    /// Runs after [`HandleClientJoined`] so that other modules can insert their
    /// own catch-up messages (e.g. `ItemEvent::Stored`) between the entity-spawn
    /// burst and the ready signal.
    SendStreamReady,
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
    anim_state: u8,
    holding: bool,
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

/// Marker component placed on every entity that was spawned by the `"spawns"`
/// map layer. Used by [`SpawnsLayer::save`] to identify which entities should
/// be written back out as spawn points.
#[derive(Component, Debug, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct SpawnMarker;

/// Opaque property bag stored on editor spawn marker entities.
///
/// The editor does not interpret property values — it preserves them as raw
/// RON across load/save round-trips.  [`SpawnsLayer::save`] merges these
/// with any live-serialized properties, giving precedence to real components.
#[derive(Component, Debug, Clone, Default)]
pub struct SpawnProperties(pub HashMap<String, Box<RawValue>>);

/// One entry in the `"spawns"` map layer.
///
/// Template names keep the file stable across registry-order changes.
/// Per-instance data is stored in [`properties`](SpawnPoint::properties) as
/// a map of registered [`ThingPropertyRegistry`] keys to RON values.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SpawnPoint {
    pub position: [f32; 3],
    /// Name of the [`ThingRegistry`] template (registered via
    /// [`ThingRegistry::register_named`]).
    pub template: String,
    /// Deprecated — kept for backward compatibility with old map files that
    /// may contain `contents: []`.  Never written.
    #[serde(default, skip_serializing)]
    #[allow(dead_code)]
    contents: Vec<String>,
    /// Per-instance property overrides keyed by [`ThingPropertyRegistry`] name.
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub properties: HashMap<String, Box<RawValue>>,
}

impl SpawnPoint {
    /// Create a new spawn point with no property overrides.
    pub fn new(position: [f32; 3], template: impl Into<String>) -> Self {
        Self {
            position,
            template: template.into(),
            contents: vec![],
            properties: HashMap::new(),
        }
    }

    /// Create a new spawn point with property overrides.
    pub fn with_properties(
        position: [f32; 3],
        template: impl Into<String>,
        properties: HashMap<String, Box<RawValue>>,
    ) -> Self {
        Self {
            position,
            template: template.into(),
            contents: vec![],
            properties,
        }
    }
}

/// Entity event to construct the full visual + functional representation of a thing.
/// The observer adds base components (mesh, physics, Thing marker) then runs
/// both the visual and functional builders registered for the given `kind`
/// via [`ThingRegistry`].
#[derive(EntityEvent)]
pub(crate) struct SpawnThing {
    pub entity: Entity,
    pub kind: u16,
    pub position: Vec3,
}

/// Entity event to construct only the visual representation of a thing.
///
/// Used by the editor to show what entities look like without adding physics,
/// AI, or other gameplay components.
#[derive(EntityEvent)]
pub struct SpawnThingVisual {
    pub entity: Entity,
    pub kind: u16,
    pub position: Vec3,
}

pub type ThingBuilder = Box<dyn Fn(Entity, &mut Commands) + Send + Sync>;
pub type ThingVisualBuilder = Box<dyn Fn(Entity, &mut Commands) + Send + Sync>;

/// Registry mapping `kind` values to template callbacks that insert
/// type-specific components on a spawned entity.
///
/// Templates may also be registered with a string name via
/// [`register_named`](ThingRegistry::register_named) to support name-based
/// lookup in the `"spawns"` map layer.
#[derive(Resource, Default)]
pub struct ThingRegistry {
    templates: HashMap<u16, ThingBuilder>,
    visual_builders: HashMap<u16, ThingVisualBuilder>,
    name_to_kind: HashMap<String, u16>,
    kind_to_name: HashMap<u16, String>,
}

impl ThingRegistry {
    /// Register a template builder for a numeric kind without a name.
    ///
    /// Use [`register_named`](Self::register_named) instead when the template
    /// should be reachable by name from a map file.
    pub fn register(
        &mut self,
        kind: u16,
        builder: impl Fn(Entity, &mut Commands) + Send + Sync + 'static,
    ) {
        self.templates.insert(kind, Box::new(builder));
    }

    /// Register a visual-only builder for a numeric kind.
    ///
    /// The visual builder adds meshes and materials without physics or gameplay
    /// components. It is called by [`SpawnThingVisual`] and also as part of
    /// the full [`SpawnThing`] flow.
    pub fn register_visual(
        &mut self,
        kind: u16,
        builder: impl Fn(Entity, &mut Commands) + Send + Sync + 'static,
    ) {
        self.visual_builders.insert(kind, Box::new(builder));
    }

    /// Register a named template with separate visual and functional builders.
    ///
    /// The `visual` builder adds meshes/materials. The `functional` builder
    /// adds physics, AI, item components, etc. [`SpawnThing`] runs both;
    /// [`SpawnThingVisual`] runs only the visual builder.
    ///
    /// # Panics
    ///
    /// Panics if `name` is already registered for a different `kind`, or if
    /// `kind` already has a different name registered. Duplicate registrations
    /// with the exact same `name` and `kind` are idempotent and do not panic.
    pub fn register_named(
        &mut self,
        name: impl Into<String>,
        kind: u16,
        visual: impl Fn(Entity, &mut Commands) + Send + Sync + 'static,
        functional: impl Fn(Entity, &mut Commands) + Send + Sync + 'static,
    ) {
        let name = name.into();
        if let Some(&existing_kind) = self.name_to_kind.get(&name) {
            assert_eq!(
                existing_kind, kind,
                "template name \"{name}\" is already registered for kind {existing_kind}, \
                 cannot re-register for kind {kind}"
            );
            return;
        }
        if let Some(existing_name) = self.kind_to_name.get(&kind) {
            assert_eq!(
                existing_name, &name,
                "kind {kind} is already registered under name \"{existing_name}\", \
                 cannot re-register under name \"{name}\""
            );
            return;
        }
        self.name_to_kind.insert(name.clone(), kind);
        self.kind_to_name.insert(kind, name);
        self.register_visual(kind, visual);
        self.register(kind, functional);
    }

    /// Look up the kind number for a template name, or `None` if unregistered.
    pub fn kind_by_name(&self, name: &str) -> Option<u16> {
        self.name_to_kind.get(name).copied()
    }

    /// Look up the template name for a kind number, or `None` if the kind has
    /// no registered name.
    pub fn name_by_kind(&self, kind: u16) -> Option<&str> {
        self.kind_to_name.get(&kind).map(String::as_str)
    }

    /// Returns an iterator over all named templates as `(name, kind)` pairs.
    pub fn named_templates(&self) -> impl Iterator<Item = (&str, u16)> {
        self.name_to_kind
            .iter()
            .map(|(name, &kind)| (name.as_str(), kind))
    }
}

// ---------------------------------------------------------------------------
// ThingPropertyRegistry — type-erased per-instance property storage
// ---------------------------------------------------------------------------

type SerializeFn = Arc<
    dyn Fn(
            Entity,
            &World,
        ) -> Option<Result<Box<RawValue>, Box<dyn std::error::Error + Send + Sync>>>
        + Send
        + Sync,
>;

type DeserializeFn = Arc<
    dyn Fn(Entity, &RawValue, &mut World) -> Result<(), Box<dyn std::error::Error + Send + Sync>>
        + Send
        + Sync,
>;

/// Type-erased operations for a single registered property.
pub struct PropertyEntry {
    /// Read a component from an entity and serialize it to a [`RawValue`].
    /// Returns `None` if the entity does not have the component.
    pub serialize: SerializeFn,
    /// Deserialize a [`RawValue`] and insert the component onto an entity.
    pub deserialize_and_apply: DeserializeFn,
}

/// Registry mapping string keys to type-erased property operations.
///
/// Used by [`SpawnsLayer`] to persist per-instance data on things, and later
/// by network replication and scripting for the same purpose.
#[derive(Resource, Default)]
pub struct ThingPropertyRegistry {
    entries: HashMap<String, Arc<PropertyEntry>>,
}

impl ThingPropertyRegistry {
    /// Register a simple component as a named property.
    ///
    /// The component is serialized/deserialized via `ron` and inserted/read
    /// through the ECS.  The `key` is the string used in map files and on the
    /// wire.
    pub fn register_property<T>(&mut self, key: impl Into<String>)
    where
        T: Component + Serialize + DeserializeOwned + Clone,
    {
        let key = key.into();
        let entry = PropertyEntry {
            serialize: Arc::new(|entity, world| {
                let component = world.get::<T>(entity)?;
                Some(
                    to_layer_value(component)
                        .map_err(|e| Box::new(e) as Box<dyn std::error::Error + Send + Sync>),
                )
            }),
            deserialize_and_apply: Arc::new(|entity, raw, world| {
                let value: T = from_layer_value(raw)?;
                world.entity_mut(entity).insert(value);
                Ok(())
            }),
        };
        self.entries.insert(key, Arc::new(entry));
    }

    /// Register a property with custom serialize/deserialize logic.
    ///
    /// Use this for complex properties like container contents that need
    /// world traversal during serialization.
    pub fn register_custom(&mut self, key: impl Into<String>, entry: PropertyEntry) {
        self.entries.insert(key.into(), Arc::new(entry));
    }

    /// Returns an iterator over all registered property keys.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.entries.keys().map(String::as_str)
    }
}

/// Serialize all registered properties present on `entity` into a map.
///
/// Reads [`ThingPropertyRegistry`] from the world.  Returns an empty map
/// when no properties are present.
pub fn serialize_entity_properties(
    entity: Entity,
    world: &World,
) -> HashMap<String, Box<RawValue>> {
    let registry = world.resource::<ThingPropertyRegistry>();
    let mut properties = HashMap::new();
    for (key, entry) in &registry.entries {
        if let Some(Ok(raw)) = (entry.serialize)(entity, world) {
            properties.insert(key.clone(), raw);
        }
    }
    properties
}

/// Apply property overrides from a map onto an entity.
///
/// Clones [`Arc`] references out of the registry before mutating the world,
/// avoiding borrow conflicts.  Unknown keys are logged as warnings.
pub fn apply_properties(
    entity: Entity,
    properties: &HashMap<String, Box<RawValue>>,
    world: &mut World,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Clone Arc refs to avoid holding a borrow on the world.
    let entries: Vec<(String, Arc<PropertyEntry>)> = {
        let registry = world.resource::<ThingPropertyRegistry>();
        registry
            .entries
            .iter()
            .map(|(k, v)| (k.clone(), Arc::clone(v)))
            .collect()
    };

    for (key, raw_value) in properties {
        if let Some((_, entry)) = entries.iter().find(|(k, _)| k == key) {
            (entry.deserialize_and_apply)(entity, raw_value, world)?;
        } else {
            warn!("ThingPropertyRegistry: unknown property key \"{key}\", skipping");
        }
    }
    Ok(())
}

/// `MapLayer` implementation for the `"spawns"` layer.
///
/// **Load:** deserializes a list of [`SpawnPoint`]s and, for each one, spawns
/// an entity with [`SpawnMarker`] and triggers [`SpawnThing`] using the kind
/// looked up via the template name in [`ThingRegistry`].
///
/// **Save:** queries all entities that carry [`SpawnMarker`] + [`Thing`] +
/// [`Transform`], converts each to a [`SpawnPoint`], and serializes the list.
pub struct SpawnsLayer;

impl MapLayer for SpawnsLayer {
    fn key(&self) -> &'static str {
        "spawns"
    }

    fn save(
        &self,
        world: &World,
    ) -> Result<Box<ron::value::RawValue>, Box<dyn std::error::Error + Send + Sync>> {
        let (Some(spawn_marker_id), Some(thing_id), Some(transform_id)) = (
            world.component_id::<SpawnMarker>(),
            world.component_id::<Thing>(),
            world.component_id::<Transform>(),
        ) else {
            // No spawn-marker entities exist yet (components not registered).
            return to_layer_value(&Vec::<SpawnPoint>::new()).map_err(Into::into);
        };

        let registry = world.resource::<ThingRegistry>();
        let mut spawn_points = Vec::new();

        for archetype in world.archetypes().iter() {
            if !archetype.contains(spawn_marker_id)
                || !archetype.contains(thing_id)
                || !archetype.contains(transform_id)
            {
                continue;
            }

            for archetype_entity in archetype.entities() {
                let entity = archetype_entity.id();
                let Ok(entity_ref) = world.get_entity(entity) else {
                    continue;
                };
                let transform = entity_ref
                    .get::<Transform>()
                    .expect("archetype guarantees Transform");
                let thing = entity_ref
                    .get::<Thing>()
                    .expect("archetype guarantees Thing");
                let Some(name) = registry.name_by_kind(thing.kind) else {
                    warn!(
                        "SpawnMarker entity {:?} has kind {} with no registered name; \
                         skipping during save",
                        entity, thing.kind
                    );
                    continue;
                };
                let mut properties = serialize_entity_properties(entity, world);
                // Merge in opaque editor properties for keys not already
                // covered by a live-serialized component.
                if let Some(stored) = entity_ref.get::<SpawnProperties>() {
                    for (key, raw) in &stored.0 {
                        properties.entry(key.clone()).or_insert_with(|| raw.clone());
                    }
                }
                spawn_points.push(SpawnPoint::with_properties(
                    transform.translation.to_array(),
                    name,
                    properties,
                ));
            }
        }

        to_layer_value(&spawn_points).map_err(Into::into)
    }

    fn load(
        &self,
        data: &RawValue,
        world: &mut World,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let spawn_points: Vec<SpawnPoint> = from_layer_value(data)?;

        // Phase 1: spawn all entities and trigger SpawnThing for each.
        // Collect (Entity, properties) pairs for phase 2.
        let mut pending: Vec<(Entity, HashMap<String, Box<RawValue>>)> =
            Vec::with_capacity(spawn_points.len());
        for sp in spawn_points {
            let kind = {
                let registry = world.resource::<ThingRegistry>();
                registry
                    .kind_by_name(&sp.template)
                    .ok_or_else(|| format!("unknown spawn template: \"{}\"", sp.template))?
            };
            let position = Vec3::from_array(sp.position);
            let entity = world.spawn(SpawnMarker).id();
            spawn_thing_world(world, entity, kind, position);
            if !sp.properties.is_empty() {
                pending.push((entity, sp.properties));
            }
        }

        // Phase 2: flush deferred commands so template components are applied,
        // then apply property overrides.
        if !pending.is_empty() {
            world.flush();
            for (entity, properties) in pending {
                apply_properties(entity, &properties, world)?;
            }
        }

        Ok(())
    }

    fn unload(&self, world: &mut World) {
        let mut to_despawn = Vec::new();
        let mut query = world.query_filtered::<Entity, With<SpawnMarker>>();
        for entity in query.iter(world) {
            to_despawn.push(entity);
        }
        for entity in to_despawn {
            world.despawn(entity);
        }
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
        /// Animation state (wire-encoded [`AnimState`] variant).
        anim_state: u8,
        /// Whether the entity is in a hold pose.
        holding: bool,
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

/// Triggers [`SpawnThing`] on an existing entity in a `&mut World` context.
///
/// If a [`Server`] resource exists, allocates a [`NetId`] and registers the
/// entity in [`NetIdIndex`] so it is visible to clients.
///
/// Used by map-loading and property deserialization code that needs synchronous
/// world access (e.g. to `flush()` and read back components immediately).
pub fn spawn_thing_world(world: &mut World, entity: Entity, kind: u16, position: Vec3) {
    if let Some(net_id) = world
        .get_resource_mut::<Server>()
        .map(|mut s| s.next_net_id())
    {
        world.entity_mut(entity).insert(net_id);
        world.resource_mut::<NetIdIndex>().0.insert(net_id, entity);
    }
    world.trigger(SpawnThing {
        entity,
        kind,
        position,
    });
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

/// Resource storing the active state value for replicated-entity cleanup.
#[derive(Resource, Clone, Copy)]
struct ThingsActiveState<S: States>(S);

impl<S: States + Copy> Plugin for ThingsPlugin<S> {
    fn build(&self, app: &mut App) {
        let state = self.state;
        app.register_type::<Thing>();
        app.register_type::<HandSide>();
        app.register_type::<HandSlot>();
        app.register_type::<PlayerControlled>();
        app.register_type::<InputDirection>();
        app.register_type::<DisplayName>();
        app.register_type::<SpawnMarker>();
        app.init_resource::<ThingRegistry>();
        app.init_resource::<ThingPropertyRegistry>();
        app.init_resource::<NetIdIndex>();
        app.init_resource::<StateBroadcastTimer>();
        app.insert_resource(ThingsActiveState(state));
        app.add_observer(on_spawn_thing);
        app.add_observer(on_spawn_thing_visual);
        app.register_map_layer(SpawnsLayer);
        app.add_observer(on_net_id_added::<S>);
        app.add_systems(OnExit(state), clear_net_id_index);

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
        app.configure_sets(
            NetworkReceive,
            ThingsSet::SendStreamReady.after(ThingsSet::HandleClientJoined),
        );
        app.add_systems(
            NetworkReceive,
            (
                handle_entity_lifecycle.run_if(resource_exists::<Client>),
                handle_client_joined
                    .run_if(resource_exists::<Server>)
                    .in_set(ThingsSet::HandleClientJoined),
                send_stream_ready_on_join
                    .run_if(resource_exists::<Server>)
                    .in_set(ThingsSet::SendStreamReady),
            ),
        );
        app.add_systems(
            NetworkSend,
            broadcast_state.run_if(resource_exists::<Server>),
        );

        // Register the messages raycast_things reads/writes so the resources
        // exist even when InputPlugin is not added (e.g. headless server mode).
        app.add_message::<PointerAction>();
        app.add_message::<WorldHit>();
        app.add_systems(
            Update,
            raycast_things
                .run_if(in_state(state))
                .run_if(not(resource_exists::<Headless>)),
        );

        app.add_systems(
            Update,
            (
                assign_missing_net_ids,
                despawn_fallen_things,
            )
                .run_if(resource_exists::<Server>),
        );
    }
}

/// Despawns any `Thing` entity that falls below Y = -50.
///
/// Serves as both a safety net (prevents entities drifting to -infinity) and a
/// debug aid — the info log makes it easy to spot physics-setup ordering issues.
const FALLEN_THRESHOLD_Y: f32 = -50.0;

fn despawn_fallen_things(
    mut commands: Commands,
    things: Query<(Entity, &Thing, &Transform, Option<&NetId>, Option<&Name>)>,
    mut net_id_index: ResMut<NetIdIndex>,
) {
    for (entity, thing, transform, net_id, name) in &things {
        if transform.translation.y < FALLEN_THRESHOLD_Y {
            let label = name.map(|n| n.as_str()).unwrap_or("unnamed");
            info!(
                "Despawning fallen thing kind={} ({label}) at Y={:.1} (entity={entity:?})",
                thing.kind, transform.translation.y
            );
            if let Some(nid) = net_id {
                net_id_index.0.remove(nid);
            }
            commands.entity(entity).despawn();
        }
    }
}

/// Assigns [`NetId`]s to any [`Thing`] entity that was spawned before the
/// [`Server`] resource existed (e.g. items from map load on headless startup).
///
/// Runs every frame but is a no-op once all things have ids.
fn assign_missing_net_ids(
    mut server: ResMut<Server>,
    mut net_index: ResMut<NetIdIndex>,
    query: Query<(Entity, Option<&Name>), (With<Thing>, Without<NetId>)>,
    mut commands: Commands,
) {
    for (entity, name) in query.iter() {
        let net_id = server.next_net_id();
        commands.entity(entity).insert(net_id);
        net_index.0.insert(net_id, entity);
        let label = name.map(|n| n.as_str()).unwrap_or("unnamed");
        warn!("Assigned {net_id:?} to pre-existing thing {entity:?} ({label})");
    }
}

/// Inserts [`DespawnOnExit`] on every replicated entity when it receives a [`NetId`].
///
/// This ensures replicated entities are automatically cleaned up when leaving the active
/// game state.
fn on_net_id_added<S: States + Copy>(
    trigger: On<Add, NetId>,
    mut commands: Commands,
    active_state: Res<ThingsActiveState<S>>,
) {
    commands
        .entity(trigger.event_target())
        .insert(DespawnOnExit(active_state.0));
}

/// Clears the [`NetIdIndex`] when leaving the active game state.
///
/// Entities are already despawned via [`DespawnOnExit`]; this removes the now-stale
/// mappings so a subsequent connection starts with a clean index.
fn clear_net_id_index(mut net_id_index: ResMut<NetIdIndex>) {
    net_id_index.0.clear();
}

fn on_spawn_thing(on: On<SpawnThing>, mut commands: Commands, registry: Res<ThingRegistry>) {
    let event = on.event();
    debug!(
        "on_spawn_thing: kind={} entity={:?} pos={:?}",
        event.kind, event.entity, event.position
    );

    // Insert only the shared base components. Collider, mesh, and material
    // are the template's responsibility — no defaults are applied here.
    commands.entity(event.entity).insert((
        Transform::from_translation(event.position),
        LinearVelocity::default(),
        Thing { kind: event.kind },
        LastBroadcast::default(),
    ));

    // Run visual builder first (meshes, materials), then functional builder
    // (physics, gameplay components).
    if let Some(visual) = registry.visual_builders.get(&event.kind) {
        visual(event.entity, &mut commands);
    }

    if let Some(builder) = registry.templates.get(&event.kind) {
        builder(event.entity, &mut commands);
    } else {
        warn!("No template registered for thing kind {}", event.kind);
    }
}

fn on_spawn_thing_visual(
    on: On<SpawnThingVisual>,
    mut commands: Commands,
    registry: Res<ThingRegistry>,
) {
    let event = on.event();
    debug!(
        "on_spawn_thing_visual: kind={} entity={:?} pos={:?}",
        event.kind, event.entity, event.position
    );

    commands.entity(event.entity).insert((
        Transform::from_translation(event.position),
        Thing { kind: event.kind },
    ));

    if let Some(visual) = registry.visual_builders.get(&event.kind) {
        visual(event.entity, &mut commands);
    } else {
        warn!("No visual builder registered for thing kind {}", event.kind);
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
                anim_state,
                holding,
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
                        commands
                            .entity(existing)
                            .insert(ControlledByClient(owner_id));
                    }
                    commands.entity(existing).insert((
                        AnimState::from(anim_state),
                        HoldIk { active: holding, ..Default::default() },
                    ));
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

                // The server is the physics authority — override the
                // template's Dynamic body so client-side gravity can't
                // pull replicated entities through the floor before tile
                // colliders are ready.
                commands.entity(entity).insert((
                    RigidBody::Kinematic,
                    GravityScale(0.0),
                ));

                if let Some(n) = name.as_deref().filter(|n| !n.is_empty()) {
                    commands.entity(entity).insert(DisplayName(n.to_string()));
                }

                if controlled {
                    commands.entity(entity).insert(PlayerControlled);
                }

                if let Some(owner_id) = owner {
                    commands.entity(entity).insert(ControlledByClient(owner_id));
                }

                commands.entity(entity).insert((
                    AnimState::from(anim_state),
                    HoldIk { active: holding, ..Default::default() },
                ));

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
                    if let Some(&entity) = net_id_index.0.get(&state.net_id)
                        && let Ok(mut transform) = entities.get_mut(entity)
                    {
                        transform.translation = Vec3::from_array(state.position);

                        let new_anim_state = AnimState::from(state.anim_state);
                        let holding_active = state.holding;

                        commands.queue(move |world: &mut World| {
                            if let Ok(mut entity_mut) = world.get_entity_mut(entity) {
                                // Only update AnimState if missing or actually changed.
                                let should_update = entity_mut
                                    .get::<AnimState>()
                                    .map_or(true, |existing| *existing != new_anim_state);
                                if should_update {
                                    entity_mut.insert(new_anim_state);
                                }

                                // Only update HoldIk when `active` changes; insert if missing.
                                match entity_mut.get_mut::<HoldIk>() {
                                    Some(mut hold_ik) => {
                                        if hold_ik.active != holding_active {
                                            hold_ik.active = holding_active;
                                        }
                                    }
                                    None => {
                                        entity_mut.insert(HoldIk {
                                            active: holding_active,
                                            ..Default::default()
                                        });
                                    }
                                }
                            }
                        });
                    }
                }
            }
        }
    }
}

/// Handles server-side catch-up on client join for stream 3.
///
/// Sends catch-up [`ThingsStreamMessage::EntitySpawned`] messages for all currently
/// tracked entities to the joining client.
///
/// The [`StreamReady`] sentinel is sent separately by [`send_stream_ready_on_join`]
/// in [`ThingsSet::SendStreamReady`], which runs after both this system *and* any
/// other module catch-up systems (e.g. `broadcast_held_on_join` and
/// `broadcast_stored_on_join` in `items`).
///
/// Creature spawning and the EntitySpawned broadcast for the new player entity are
/// handled by the `souls` module's `bind_soul` system.
#[allow(clippy::type_complexity)]
fn handle_client_joined(
    mut messages: MessageReader<PlayerEvent>,
    stream_sender: Res<StreamSender<ThingsStreamMessage>>,
    entities: Query<(
        &NetId,
        Option<&ControlledByClient>,
        &Transform,
        Option<&LinearVelocity>,
        Option<&DisplayName>,
        &Thing,
        Option<&AnimState>,
        Option<&HoldIk>,
    )>,
) {
    for event in messages.read() {
        let PlayerEvent::Joined { id: from, .. } = event else {
            continue;
        };

        // Catch-up: send EntitySpawned on stream 3 for every existing Thing entity.
        for (net_id, opt_controlled_by, transform, opt_velocity, opt_name, thing, opt_anim, opt_hold) in entities.iter()
        {
            let owner = opt_controlled_by
                .map(|c| c.0)
                .filter(|&owner_id| owner_id == *from);
            let vel = opt_velocity
                .map(|lv| [lv.x, lv.y, lv.z])
                .unwrap_or([0.0, 0.0, 0.0]);
            let anim_state: u8 = opt_anim.copied().unwrap_or_default().into();
            let holding = opt_hold.map(|h| h.active).unwrap_or(false);
            if let Err(e) = stream_sender.send_to(
                *from,
                &ThingsStreamMessage::EntitySpawned {
                    net_id: *net_id,
                    kind: thing.kind,
                    position: transform.translation.into(),
                    velocity: vel,
                    owner,
                    name: opt_name.map(|n| n.0.clone()),
                    anim_state,
                    holding,
                },
            ) {
                error!(
                    "Failed to send EntitySpawned catch-up to ClientId({}): {e}",
                    from.0
                );
            }
        }
    }
}

/// Sends the [`StreamReady`] sentinel for stream 3 to every client that joined
/// this frame.
///
/// Runs in [`ThingsSet::SendStreamReady`], which is ordered after
/// [`ThingsSet::HandleClientJoined`].  This guarantees that all initial-burst
/// messages — including any catch-up frames inserted by other modules between the
/// two sets (e.g. `broadcast_stored_on_join` in `items`) — have been enqueued
/// before the client sees the ready signal.
fn send_stream_ready_on_join(
    mut messages: MessageReader<PlayerEvent>,
    stream_sender: Res<StreamSender<ThingsStreamMessage>>,
    mut module_ready: MessageWriter<ModuleReadySent>,
) {
    for event in messages.read() {
        let PlayerEvent::Joined { id: from, .. } = event else {
            continue;
        };
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
    mut entities: Query<
        (
            &NetId,
            &Transform,
            Option<&LinearVelocity>,
            Option<&AnimState>,
            Option<&HoldIk>,
            &mut LastBroadcast,
        ),
        Without<ChildOf>,
    >,
) {
    if !timer.0.tick(time.delta()).just_finished() {
        return;
    }

    let states: Vec<EntityState> = entities
        .iter_mut()
        .filter_map(|(net_id, transform, velocity, opt_anim, opt_hold, mut last)| {
            let pos = transform.translation;
            let vel = velocity.map(|lv| lv.0).unwrap_or(Vec3::ZERO);
            let anim_state: u8 = opt_anim.copied().unwrap_or_default().into();
            let holding = opt_hold.map(|h| h.active).unwrap_or(false);

            let pos_changed = (pos - last.position).length_squared() > POSITION_EPSILON_SQ;
            let vel_changed = (vel - last.velocity).length_squared() > VELOCITY_EPSILON_SQ;
            let anim_changed = anim_state != last.anim_state;
            let hold_changed = holding != last.holding;

            if !pos_changed && !vel_changed && !anim_changed && !hold_changed {
                return None;
            }

            last.position = pos;
            last.velocity = vel;
            last.anim_state = anim_state;
            last.holding = holding;

            Some(EntityState {
                net_id: *net_id,
                position: pos.into(),
                velocity: [vel.x, vel.y, vel.z],
                anim_state,
                holding,
            })
        })
        .collect();

    if !states.is_empty()
        && let Err(e) =
            stream_sender.broadcast(&ThingsStreamMessage::StateUpdate { entities: states })
    {
        error!("Failed to broadcast entity state on things stream: {e}");
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
        ) && things.get(hit.entity).is_ok()
        {
            let world_pos = ray.origin + *ray.direction * hit.distance;
            hit_writer.write(WorldHit {
                button: action.button,
                entity: hit.entity,
                world_pos,
            });
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
        app.init_resource::<ThingRegistry>();
        app.add_observer(on_spawn_thing);

        // Register a kind-0 builder that mirrors what CreaturesPlugin registers:
        // spawn a HandSlot child on the creature entity.
        app.world_mut()
            .resource_mut::<ThingRegistry>()
            .register(0, |entity, commands| {
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
        assert_eq!(slot.side, HandSide::Right, "HandSlot side should be Right");
    }

    /// Verifies that SpawnsLayer::load deserializes spawn points and triggers
    /// SpawnThing for each, producing entities with SpawnMarker + Thing +
    /// Transform at the correct positions.
    #[test]
    fn spawns_layer_load_spawns_entities_at_correct_positions() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<ThingRegistry>();
        app.add_observer(on_spawn_thing);

        app.world_mut()
            .resource_mut::<ThingRegistry>()
            .register_named(
                "crate",
                1,
                |_entity, _commands| {},
                |_entity, _commands| {},
            );

        let data = world::to_layer_value(&vec![
            SpawnPoint::new([1.0, 0.0, 2.0], "crate"),
            SpawnPoint::new([3.0, 0.0, 4.0], "crate"),
        ])
        .expect("to_layer_value");

        SpawnsLayer
            .load(&data, app.world_mut())
            .expect("SpawnsLayer::load");

        app.update();

        let mut positions: Vec<[f32; 3]> = {
            let world = app.world();
            let Some(spawn_marker_id) = world.component_id::<SpawnMarker>() else {
                panic!("SpawnMarker component not registered");
            };
            world
                .archetypes()
                .iter()
                .filter(|a| a.contains(spawn_marker_id))
                .flat_map(|a| a.entities())
                .filter_map(|ae| {
                    world
                        .get_entity(ae.id())
                        .ok()?
                        .get::<Transform>()
                        .map(|t| t.translation.to_array())
                })
                .collect()
        };
        positions.sort_by(|a, b| a[0].partial_cmp(&b[0]).unwrap());

        assert_eq!(positions.len(), 2, "expected 2 spawned entities");
        assert_eq!(positions[0], [1.0, 0.0, 2.0]);
        assert_eq!(positions[1], [3.0, 0.0, 4.0]);
    }

    /// Verifies that SpawnsLayer::save serializes all SpawnMarker entities back
    /// to the correct SpawnPoint list and that the round-trip is lossless.
    #[test]
    fn spawns_layer_save_round_trips() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<ThingRegistry>();
        app.init_resource::<ThingPropertyRegistry>();
        app.add_observer(on_spawn_thing);

        app.world_mut()
            .resource_mut::<ThingRegistry>()
            .register_named(
                "barrel",
                2,
                |_entity, _commands| {},
                |_entity, _commands| {},
            );

        // Manually place a spawn-marker entity (simulating what load() does).
        app.world_mut().spawn((
            SpawnMarker,
            Transform::from_translation(Vec3::new(5.0, 0.0, 7.0)),
            Thing { kind: 2 },
        ));

        let raw = SpawnsLayer.save(app.world()).expect("SpawnsLayer::save");
        let loaded: Vec<SpawnPoint> =
            world::from_layer_value(&raw).expect("round-trip deserialize");

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].template, "barrel");
        assert_eq!(loaded[0].position, [5.0, 0.0, 7.0]);
        assert!(loaded[0].properties.is_empty());
    }

    /// Verifies that SpawnsLayer::load returns an error for an unregistered
    /// template name rather than silently ignoring or panicking.
    #[test]
    fn spawns_layer_load_errors_on_unknown_template() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.init_resource::<ThingRegistry>();
        app.add_observer(on_spawn_thing);

        let data = world::to_layer_value(&vec![SpawnPoint::new([0.0, 0.0, 0.0], "no_such_thing")])
            .expect("to_layer_value");

        let result = SpawnsLayer.load(&data, app.world_mut());
        assert!(
            result.is_err(),
            "load should return Err for an unknown template"
        );
    }

    #[test]
    fn named_templates_returns_registered_name_kind_pairs() {
        let mut registry = ThingRegistry::default();

        registry.register_named("foo", 1, |_, _| {}, |_, _| {});
        registry.register_named("bar", 2, |_, _| {}, |_, _| {});

        let mut named: Vec<(&str, u16)> = registry.named_templates().collect();
        named.sort_by_key(|(_, kind)| *kind);

        assert_eq!(named, vec![("foo", 1), ("bar", 2)]);
    }

    #[test]
    fn named_templates_excludes_unnamed_registrations() {
        let mut registry = ThingRegistry::default();

        registry.register(0, |_, _| {});
        registry.register_named("named", 1, |_, _| {}, |_, _| {});

        let named: Vec<(&str, u16)> = registry.named_templates().collect();
        assert_eq!(named.len(), 1, "only named templates should appear");
        assert_eq!(named[0], ("named", 1));
    }
}
