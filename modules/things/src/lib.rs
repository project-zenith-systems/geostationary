use std::collections::HashMap;

use bevy::prelude::*;
use physics::{Collider, GravityScale, LockedAxes, RigidBody};

/// Marker component for non-grid-bound world objects.
#[derive(Component, Debug, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct Thing;

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

/// Plugin that registers the thing spawning system and shared entity primitives.
///
/// Must be added before any plugin that calls [`ThingRegistry::register`]
/// (e.g. `CreaturesPlugin`).
#[derive(Default)]
pub struct ThingsPlugin;

impl Plugin for ThingsPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Thing>();
        app.register_type::<InputDirection>();
        app.init_resource::<ThingRegistry>();

        #[cfg(feature = "client")]
        app.add_observer(on_spawn_thing_client);

        #[cfg(not(feature = "client"))]
        app.add_observer(on_spawn_thing_headless);
    }
}

#[cfg(feature = "client")]
fn on_spawn_thing_client(
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

#[cfg(not(feature = "client"))]
fn on_spawn_thing_headless(
    on: On<SpawnThing>,
    mut commands: Commands,
    registry: Res<ThingRegistry>,
) {
    let event = on.event();

    commands.entity(event.entity).insert((
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
