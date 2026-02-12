use bevy::prelude::*;
use physics::LinearVelocity;

use crate::app_state::AppState;
use crate::net_game::NetworkRole;

/// Marker component for creatures - entities that can move and act in the world.
#[derive(Component, Debug, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct Creature;

/// Marker component for player-controlled creatures.
#[derive(Component, Debug, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct PlayerControlled;

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
        app.register_type::<PlayerControlled>();
        app.add_systems(
            Update,
            creature_movement_system
                .run_if(in_state(AppState::InGame))
                .run_if(|role: Res<NetworkRole>| *role == NetworkRole::ListenServer),
        );
    }
}

#[allow(clippy::type_complexity)]
fn creature_movement_system(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut query: Query<
        (&mut LinearVelocity, &MovementSpeed),
        (With<Creature>, With<PlayerControlled>),
    >,
) {
    for (mut velocity, movement_speed) in query.iter_mut() {
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

        let desired = if direction.length_squared() > 0.0 {
            direction.normalize() * movement_speed.speed
        } else {
            Vec3::ZERO
        };

        velocity.x = desired.x;
        velocity.z = desired.z;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_creature_component_default() {
        let creature = Creature::default();
        assert!(matches!(creature, Creature));
    }

    #[test]
    fn test_movement_speed_default() {
        let speed = MovementSpeed::default();
        assert_eq!(speed.speed, 3.0);
    }

    #[test]
    fn test_movement_speed_custom() {
        let speed = MovementSpeed { speed: 5.0 };
        assert_eq!(speed.speed, 5.0);
    }

    #[test]
    fn test_player_controlled_component_default() {
        let player_controlled = PlayerControlled::default();
        assert!(matches!(player_controlled, PlayerControlled));
    }
}
