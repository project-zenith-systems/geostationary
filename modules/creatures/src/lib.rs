use bevy::prelude::*;
use physics::{GravityScale, LinearVelocity, LockedAxes};
use things::{InputDirection, ThingRegistry};

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
///
/// Must be added after [`things::ThingsPlugin`] so that [`ThingRegistry`]
/// is available for kind registration.
pub struct CreaturesPlugin;

impl Plugin for CreaturesPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Creature>();
        app.register_type::<MovementSpeed>();
        app.add_systems(Update, apply_input_velocity);

        app.world_mut()
            .resource_mut::<ThingRegistry>()
            .register(0, |entity, _event, commands| {
                commands.entity(entity).insert((
                    Creature,
                    MovementSpeed::default(),
                    InputDirection::default(),
                    LockedAxes::ROTATION_LOCKED.lock_translation_y(),
                    GravityScale(0.0),
                ));
            });
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
