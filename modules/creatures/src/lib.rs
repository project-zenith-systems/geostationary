use animation::{AnimState, HoldIk};
use bevy::prelude::*;
use items::Container;
use network::Server;
use physics::LinearVelocity;
use things::{HandSlot, InputDirection};

/// Marker component for creatures - entities that can move and act in the world.
#[derive(Component, Debug, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct Creature;

/// Component that defines how fast a creature can move (units per second).
#[derive(Component, Debug, Clone, Copy, Reflect)]
#[reflect(Component)]
pub struct MovementSpeed {
    pub speed: f32,
}

impl Default for MovementSpeed {
    fn default() -> Self {
        Self { speed: 3.0 }
    }
}

/// Plugin that registers creature components and movement systems.
pub struct CreaturesPlugin;

impl Plugin for CreaturesPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Creature>();
        app.register_type::<MovementSpeed>();
        app.add_systems(Update, apply_input_velocity);
        app.add_systems(Update, init_creature_state);
        app.add_systems(
            PostUpdate,
            (
                compute_anim_state.run_if(resource_exists::<Server>),
                compute_hold_state.run_if(resource_exists::<Server>),
            ),
        );
    }
}

/// Applies InputDirection to LinearVelocity using MovementSpeed.
/// Runs on both client (for local prediction) and server (authoritative).
fn apply_input_velocity(
    mut query: Query<(&InputDirection, &MovementSpeed, &mut LinearVelocity), With<Creature>>,
) {
    for (input, movement_speed, mut velocity) in query.iter_mut() {
        let desired = if input.0.length_squared() > 0.0 {
            input.0.normalize() * movement_speed.speed
        } else {
            Vec3::ZERO
        };
        velocity.x = desired.x;
        velocity.z = desired.z;
    }
}

/// Minimum velocity magnitude (units/s) to transition from Idle to Walk.
/// Prevents animation flicker when the creature is nearly at rest.
const VELOCITY_THRESHOLD: f32 = 0.1;

/// Squared velocity threshold for the comparison in [`compute_anim_state`].
const VELOCITY_THRESHOLD_SQ: f32 = VELOCITY_THRESHOLD * VELOCITY_THRESHOLD;

/// Ensures every creature always has [`AnimState`] and [`HoldIk`] components
/// so the derived-state systems can operate on them. Runs each frame and
/// catches both newly-spawned creatures and any entity that has had a
/// component removed. Uses `insert_if_new` to avoid clobbering existing state.
fn init_creature_state(
    mut commands: Commands,
    query: Query<Entity, (With<Creature>, Or<(Without<AnimState>, Without<HoldIk>)>)>,
) {
    for entity in query.iter() {
        commands
            .entity(entity)
            .insert_if_new(AnimState::default())
            .insert_if_new(HoldIk::default());
    }
}

/// Derives [`AnimState`] from [`LinearVelocity`] for every creature.
/// Runs on the server so the authoritative state can be replicated.
fn compute_anim_state(
    mut query: Query<(&LinearVelocity, &mut AnimState), With<Creature>>,
) {
    for (velocity, mut anim_state) in query.iter_mut() {
        let new_state = if velocity.length_squared() > VELOCITY_THRESHOLD_SQ {
            AnimState::Walk
        } else {
            AnimState::Idle
        };
        if *anim_state != new_state {
            *anim_state = new_state;
        }
    }
}

/// Derives [`HoldIk::active`] from whether the creature's [`HandSlot`] holds
/// an item. Uses descendant traversal so the system works whether the
/// `HandSlot` is a direct child or has been reparented under a hand bone.
fn compute_hold_state(
    mut creatures: Query<(Entity, &mut HoldIk), With<Creature>>,
    children_q: Query<&Children>,
    hand_containers: Query<&Container, With<HandSlot>>,
) {
    for (entity, mut hold_ik) in creatures.iter_mut() {
        let holding = has_held_item(entity, &children_q, &hand_containers);
        if hold_ik.active != holding {
            hold_ik.active = holding;
        }
    }
}

/// Recursively searches descendants of `root` for a [`HandSlot`] whose
/// [`Container`] holds at least one item.
fn has_held_item(
    root: Entity,
    children_q: &Query<&Children>,
    hand_containers: &Query<&Container, With<HandSlot>>,
) -> bool {
    let Ok(children) = children_q.get(root) else {
        return false;
    };
    for child in children.iter() {
        if let Ok(container) = hand_containers.get(child) {
            if container.slots.iter().any(|s| s.is_some()) {
                return true;
            }
        }
        if has_held_item(child, children_q, hand_containers) {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use things::HandSide;

    /// Build a minimal headless `App` with the init and compute systems
    /// registered directly (no `Server` run-condition) so they always execute.
    fn test_app() -> App {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_systems(Update, init_creature_state);
        app.add_systems(PostUpdate, (compute_anim_state, compute_hold_state));
        app.finish();
        app
    }

    // ── init_creature_state ──────────────────────────────────────────────

    #[test]
    fn init_creature_state_inserts_anim_and_hold() {
        let mut app = test_app();
        let creature = app
            .world_mut()
            .spawn((Creature, LinearVelocity::default()))
            .id();
        // Before update: no AnimState or HoldIk.
        assert!(app.world().get::<AnimState>(creature).is_none());
        assert!(app.world().get::<HoldIk>(creature).is_none());
        app.update();
        // After update: both present with defaults.
        assert_eq!(
            *app.world().get::<AnimState>(creature).unwrap(),
            AnimState::Idle
        );
        assert!(!app.world().get::<HoldIk>(creature).unwrap().active);
    }

    #[test]
    fn init_creature_state_inserts_hold_when_anim_already_present() {
        let mut app = test_app();
        // Spawn with AnimState::Walk already present, but no HoldIk.
        // Give non-zero velocity so compute_anim_state preserves Walk.
        let creature = app
            .world_mut()
            .spawn((
                Creature,
                LinearVelocity(Vec3::new(1.0, 0.0, 0.0)),
                AnimState::Walk,
            ))
            .id();
        assert!(app.world().get::<HoldIk>(creature).is_none());
        app.update();
        // AnimState preserved (not clobbered), HoldIk inserted.
        assert_eq!(
            *app.world().get::<AnimState>(creature).unwrap(),
            AnimState::Walk
        );
        assert!(!app.world().get::<HoldIk>(creature).unwrap().active);
    }

    #[test]
    fn init_creature_state_inserts_anim_when_hold_already_present() {
        let mut app = test_app();
        let mut hold = HoldIk::default();
        hold.active = true;
        // Spawn with HoldIk already present (active=true), but no AnimState.
        let creature = app
            .world_mut()
            .spawn((Creature, LinearVelocity::default(), hold))
            .id();
        // Give the creature a HandSlot with an item so compute_hold_state
        // keeps active=true.
        let item = app.world_mut().spawn_empty().id();
        app.world_mut().spawn((
            HandSlot {
                side: HandSide::Right,
            },
            Container {
                slots: vec![Some(item)],
            },
            ChildOf(creature),
        ));
        assert!(app.world().get::<AnimState>(creature).is_none());
        app.update();
        // HoldIk preserved (not clobbered), AnimState inserted.
        assert!(app.world().get::<HoldIk>(creature).unwrap().active);
        assert_eq!(
            *app.world().get::<AnimState>(creature).unwrap(),
            AnimState::Idle
        );
    }

    // ── compute_anim_state ────────────────────────────────────────────────

    #[test]
    fn anim_state_idle_when_stationary() {
        let mut app = test_app();
        app.world_mut().spawn((
            Creature,
            LinearVelocity::default(),
            AnimState::Walk, // start as Walk to prove it transitions
        ));
        app.update();
        let state = *app
            .world_mut()
            .query::<&AnimState>()
            .single(app.world())
            .unwrap();
        assert_eq!(state, AnimState::Idle);
    }

    #[test]
    fn anim_state_walk_when_moving() {
        let mut app = test_app();
        app.world_mut().spawn((
            Creature,
            LinearVelocity(Vec3::new(1.0, 0.0, 0.0)),
            AnimState::default(),
        ));
        app.update();
        let state = *app
            .world_mut()
            .query::<&AnimState>()
            .single(app.world())
            .unwrap();
        assert_eq!(state, AnimState::Walk);
    }

    #[test]
    fn anim_state_idle_below_threshold() {
        let mut app = test_app();
        // Velocity magnitude 0.05, below the 0.1 threshold.
        app.world_mut().spawn((
            Creature,
            LinearVelocity(Vec3::new(0.05, 0.0, 0.0)),
            AnimState::Walk,
        ));
        app.update();
        let state = *app
            .world_mut()
            .query::<&AnimState>()
            .single(app.world())
            .unwrap();
        assert_eq!(state, AnimState::Idle);
    }

    // ── compute_hold_state ────────────────────────────────────────────────

    /// Spawn a creature with a HoldIk component and a HandSlot child.
    /// Returns (creature_entity, hand_slot_entity).
    fn spawn_creature_with_hand(app: &mut App) -> (Entity, Entity) {
        let creature = app
            .world_mut()
            .spawn((Creature, HoldIk::default()))
            .id();
        let hand = app
            .world_mut()
            .spawn((
                HandSlot {
                    side: HandSide::Right,
                },
                Container::with_capacity(1),
                ChildOf(creature),
            ))
            .id();
        (creature, hand)
    }

    #[test]
    fn hold_state_inactive_when_hand_empty() {
        let mut app = test_app();
        let (creature, _hand) = spawn_creature_with_hand(&mut app);
        // Explicitly set active to true so we can verify it becomes false.
        app.world_mut()
            .entity_mut(creature)
            .get_mut::<HoldIk>()
            .unwrap()
            .active = true;
        app.update();
        let hold = app.world().get::<HoldIk>(creature).unwrap();
        assert!(!hold.active);
    }

    #[test]
    fn hold_state_active_when_hand_holds_item() {
        let mut app = test_app();
        let (creature, hand) = spawn_creature_with_hand(&mut app);
        // Place an item in the container.
        let item = app.world_mut().spawn_empty().id();
        app.world_mut()
            .get_mut::<Container>(hand)
            .unwrap()
            .insert(item);
        app.update();
        let hold = app.world().get::<HoldIk>(creature).unwrap();
        assert!(hold.active);
    }

    #[test]
    fn hold_state_active_when_hand_slot_nested_under_bone() {
        let mut app = test_app();
        let creature = app
            .world_mut()
            .spawn((Creature, HoldIk::default()))
            .id();
        // Simulate a bone entity between the creature and the HandSlot.
        let bone = app
            .world_mut()
            .spawn(ChildOf(creature))
            .id();
        let item = app.world_mut().spawn_empty().id();
        let _hand = app
            .world_mut()
            .spawn((
                HandSlot {
                    side: HandSide::Right,
                },
                Container {
                    slots: vec![Some(item)],
                },
                ChildOf(bone),
            ))
            .id();
        app.update();
        let hold = app.world().get::<HoldIk>(creature).unwrap();
        assert!(hold.active);
    }
}
