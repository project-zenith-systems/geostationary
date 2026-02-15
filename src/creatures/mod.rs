use bevy::prelude::*;
use physics::LinearVelocity;
use things::ThingRegistry;

use crate::client::PlayerControlled;
use network::InputDirection;

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

pub struct CreaturesPlugin;

impl Plugin for CreaturesPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Creature>();
        app.register_type::<MovementSpeed>();
        app.add_systems(Update, (read_player_input, apply_input_velocity).chain());

        app.world_mut()
            .resource_mut::<ThingRegistry>()
            .register(0, |entity, event, commands| {
                let mut ec = commands.entity(entity);
                ec.insert((Creature, MovementSpeed::default(), InputDirection::default()));
                if event.controlled {
                    ec.insert(PlayerControlled);
                }
            });
    }
}

/// Reads keyboard input and writes InputDirection on PlayerControlled entities.
fn read_player_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut query: Query<&mut InputDirection, With<PlayerControlled>>,
) {
    for mut input in query.iter_mut() {
        let mut direction = Vec3::ZERO;
        if keyboard.pressed(KeyCode::KeyW) {
            direction.z -= 1.0;
        }
        if keyboard.pressed(KeyCode::KeyS) {
            direction.z += 1.0;
        }
        if keyboard.pressed(KeyCode::KeyA) {
            direction.x -= 1.0;
        }
        if keyboard.pressed(KeyCode::KeyD) {
            direction.x += 1.0;
        }
        input.0 = direction;
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
