use animation::{AnimState, HoldIk};
use bevy::prelude::*;
use bevy::scene::SceneInstanceReady;
use creatures::{Creature, MovementSpeed};
use items::{Container, Item};
use network::Headless;
use physics::{Collider, GravityScale, LockedAxes, Restitution, RigidBody};
use things::{HAND_OFFSET, HandSide, HandSlot, InputDirection, ThingRegistry};

pub const BALL_RADIUS: f32 = 0.3;

/// Default IK target for the creature's hold pose, in creature-local space.
/// Right-hand side, slightly forward and below the shoulder.
pub const CREATURE_HOLD_IK_TARGET: Vec3 = Vec3::new(0.3, 0.7, -0.3);

/// The bone name in the GLTF model that the [`HandSlot`] should be reparented to.
const HAND_BONE_NAME: &str = "hand.R";

/// Marker component inserted on a creature entity after its GLTF scene has
/// been fully loaded and scene-ready initialisation (bone reparenting, etc.)
/// has been performed.
#[derive(Component, Debug, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct SceneReady;

pub struct TemplatesPlugin;

impl Plugin for TemplatesPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<SceneReady>();

        let headless = app.world().contains_resource::<Headless>();

        // Ensure material assets are available even on headless servers.
        // Guard: only register the asset type when PbrPlugin hasn't done it already,
        // to avoid double-registering asset events/systems.
        if !app.world().contains_resource::<Assets<StandardMaterial>>() {
            app.init_asset::<StandardMaterial>();
        }

        // Load GLTF scene handle for the creature model (client only).
        // On the headless server we skip `asset_server.load()` entirely to
        // avoid pulling the `.glb` file into memory.
        let creature_scene = if !headless {
            let asset_server = app.world().resource::<AssetServer>();
            Some(asset_server.load("models/creature.glb#Scene0"))
        } else {
            None
        };

        let mut meshes = app.world_mut().resource_mut::<Assets<Mesh>>();
        let ball_mesh = meshes.add(Sphere::new(BALL_RADIUS));
        let can_mesh = meshes.add(Cylinder::new(0.15, 0.1));
        let toolbox_mesh = meshes.add(Cuboid::new(0.6, 0.3, 0.4));

        let mut materials = app.world_mut().resource_mut::<Assets<StandardMaterial>>();
        let ball_mat = materials.add(StandardMaterial {
            base_color: Color::srgb(1.0, 0.8, 0.0),
            ..default()
        });
        let can_mat = materials.add(StandardMaterial {
            base_color: Color::srgb(0.7, 0.7, 0.75),
            metallic: 0.8,
            ..default()
        });
        let toolbox_mat = materials.add(StandardMaterial {
            base_color: Color::srgb(0.8, 0.2, 0.2),
            ..default()
        });

        let mut registry = app.world_mut().resource_mut::<ThingRegistry>();

        // Kind 0: Creature — player-controlled entity with locked axes, hand slot.
        //
        // Visual builder: GLTF scene on client, no-op on headless server.
        // `register_named` sets a no-op visual; the real GLTF visual is
        // registered afterwards (client only) so `asset_server.load()` is
        // never called on the server.
        registry.register_named(
            "creature",
            0,
            // Visual: placeholder no-op, overwritten below on client.
            |_entity, _commands| {},
            // Functional: physics, movement, hand slot, anim/IK state.
            |entity, commands| {
                debug!("Template kind 0 (creature) functional: applying to {entity:?}");
                commands.entity(entity).insert((
                    Creature,
                    MovementSpeed::default(),
                    InputDirection::default(),
                    AnimState::Idle,
                    HoldIk {
                        active: false,
                        target: CREATURE_HOLD_IK_TARGET,
                    },
                    RigidBody::Dynamic,
                    Collider::capsule(0.3, 1.0),
                    LockedAxes::ROTATION_LOCKED.lock_translation_y(),
                    GravityScale(0.0),
                ));
                commands.entity(entity).with_children(|parent| {
                    parent.spawn((
                        HandSlot {
                            side: HandSide::Right,
                        },
                        Transform::from_translation(HAND_OFFSET),
                    ));
                });
            },
        );

        // Overwrite the creature visual builder with the GLTF scene loader
        // when not running headless. This keeps `asset_server.load()` out of
        // the headless server path entirely.
        if let Some(scene) = creature_scene {
            registry.register_visual(0, move |entity, commands| {
                debug!("Template kind 0 (creature) visual: applying SceneRoot to {entity:?}");
                commands
                    .entity(entity)
                    .insert(SceneRoot(scene.clone()))
                    .observe(on_creature_scene_ready);
            });
        }

        // Kind 1: Ball — bouncy physics object.
        registry.register_named(
            "ball",
            1,
            move |entity, commands| {
                debug!("Template kind 1 (ball) visual: applying to {entity:?}");
                commands
                    .entity(entity)
                    .insert((Mesh3d(ball_mesh.clone()), MeshMaterial3d(ball_mat.clone())));
            },
            |entity, commands| {
                debug!("Template kind 1 (ball) functional: applying to {entity:?}");
                commands.entity(entity).insert((
                    Collider::sphere(BALL_RADIUS),
                    RigidBody::Dynamic,
                    GravityScale(1.0),
                    Restitution::new(0.8),
                ));
            },
        );

        // Kind 2: Can — pickable item.
        registry.register_named(
            "can",
            2,
            move |entity, commands| {
                debug!("Template kind 2 (can) visual: applying to {entity:?}");
                commands
                    .entity(entity)
                    .insert((Mesh3d(can_mesh.clone()), MeshMaterial3d(can_mat.clone())));
            },
            |entity, commands| {
                debug!("Template kind 2 (can) functional: applying to {entity:?}");
                commands.entity(entity).insert((
                    Collider::cylinder(0.15, 0.1),
                    RigidBody::Dynamic,
                    GravityScale(1.0),
                    Item,
                    Name::new("Can"),
                ));
            },
        );

        // Kind 3: Toolbox — pickable container.
        registry.register_named(
            "toolbox",
            3,
            move |entity, commands| {
                debug!("Template kind 3 (toolbox) visual: applying to {entity:?}");
                commands.entity(entity).insert((
                    Mesh3d(toolbox_mesh.clone()),
                    MeshMaterial3d(toolbox_mat.clone()),
                ));
            },
            |entity, commands| {
                debug!("Template kind 3 (toolbox) functional: applying to {entity:?}");
                commands.entity(entity).insert((
                    Collider::cuboid(0.3, 0.15, 0.2),
                    RigidBody::Dynamic,
                    GravityScale(1.0),
                    Item,
                    Name::new("Toolbox"),
                    Container::with_capacity(6),
                ));
            },
        );
    }
}

/// Observer triggered by [`SceneInstanceReady`] on a creature entity.
///
/// Walks the entity's descendants to find the hand bone (`Name("hand.R")`) and
/// the existing [`HandSlot`] child, then reparents the hand slot to the bone
/// with an identity local transform so the held item follows the bone during
/// animation and IK.
///
/// Inserts the [`SceneReady`] marker on the creature entity so other systems
/// can detect that the GLTF scene hierarchy is fully initialised.
fn on_creature_scene_ready(
    trigger: On<SceneInstanceReady>,
    mut commands: Commands,
    children_q: Query<&Children>,
    name_q: Query<&Name>,
    hand_slot_q: Query<Entity, With<HandSlot>>,
) {
    let creature = trigger.entity;
    debug!("on_creature_scene_ready: creature {creature:?}");

    commands.entity(creature).insert(SceneReady);

    // Find the hand bone by walking all descendants.
    let bone_entity = find_named_descendant(creature, HAND_BONE_NAME, &children_q, &name_q);

    // Find the HandSlot among descendants (still a direct child at this point,
    // but use descendant traversal for robustness).
    let hand_slot_entity = find_descendant_with::<HandSlot>(creature, &children_q, &hand_slot_q);

    if let (Some(bone), Some(hand_slot)) = (bone_entity, hand_slot_entity) {
        debug!(
            "on_creature_scene_ready: reparenting HandSlot {hand_slot:?} to bone {bone:?}"
        );
        commands
            .entity(hand_slot)
            .insert((ChildOf(bone), Transform::IDENTITY));
    } else {
        if bone_entity.is_none() {
            warn!(
                "on_creature_scene_ready: bone '{}' not found among descendants of {creature:?}",
                HAND_BONE_NAME
            );
        }
        if hand_slot_entity.is_none() {
            warn!(
                "on_creature_scene_ready: HandSlot not found among descendants of {creature:?}"
            );
        }
    }
}

/// BFS search for a descendant entity with a [`Name`] component matching `target`.
fn find_named_descendant(
    root: Entity,
    target: &str,
    children_q: &Query<&Children>,
    name_q: &Query<&Name>,
) -> Option<Entity> {
    let mut queue = std::collections::VecDeque::new();
    if let Ok(children) = children_q.get(root) {
        for &child in children {
            queue.push_back(child);
        }
    }
    while let Some(entity) = queue.pop_front() {
        if let Ok(name) = name_q.get(entity) {
            if name.as_str() == target {
                return Some(entity);
            }
        }
        if let Ok(children) = children_q.get(entity) {
            for &child in children {
                queue.push_back(child);
            }
        }
    }
    None
}

/// BFS search for the first descendant entity that matches the given query filter.
fn find_descendant_with<T: Component>(
    root: Entity,
    children_q: &Query<&Children>,
    filter_q: &Query<Entity, With<T>>,
) -> Option<Entity> {
    let mut queue = std::collections::VecDeque::new();
    if let Ok(children) = children_q.get(root) {
        for &child in children {
            queue.push_back(child);
        }
    }
    while let Some(entity) = queue.pop_front() {
        if filter_q.get(entity).is_ok() {
            return Some(entity);
        }
        if let Ok(children) = children_q.get(entity) {
            for &child in children {
                queue.push_back(child);
            }
        }
    }
    None
}
