use bevy::prelude::*;
use tiles::Tilemap;

use crate::app_state::AppState;

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
        app.add_systems(Update, creature_movement_system.run_if(in_state(AppState::InGame)));
    }
}

fn creature_movement_system(
    keyboard: Res<ButtonInput<KeyCode>>,
    time: Res<Time>,
    tilemap: Option<Res<Tilemap>>,
    mut query: Query<(&mut Transform, &MovementSpeed), With<Creature>>,
) {
    let Some(tilemap) = tilemap else {
        return;
    };

    for (mut transform, movement_speed) in query.iter_mut() {
        let mut movement = Vec3::ZERO;

        // Read keyboard input (WASD)
        if keyboard.pressed(KeyCode::KeyW) {
            movement.z -= 1.0;
        }
        if keyboard.pressed(KeyCode::KeyS) {
            movement.z += 1.0;
        }
        if keyboard.pressed(KeyCode::KeyA) {
            movement.x -= 1.0;
        }
        if keyboard.pressed(KeyCode::KeyD) {
            movement.x += 1.0;
        }

        // Normalize and scale by speed and delta time
        if movement.length_squared() > 0.0 {
            movement = movement.normalize() * movement_speed.speed * time.delta_secs();

            // Check each axis independently for wall sliding behavior
            let current_pos = transform.translation;
            let target_x = current_pos.x + movement.x;
            let target_z = current_pos.z + movement.z;

            // Try X axis
            let tile_x = IVec2::new(target_x.floor() as i32, current_pos.z.floor() as i32);
            if tilemap.is_walkable(tile_x) {
                transform.translation.x = target_x;
            }

            // Try Z axis
            let tile_z = IVec2::new(transform.translation.x.floor() as i32, target_z.floor() as i32);
            if tilemap.is_walkable(tile_z) {
                transform.translation.z = target_z;
            }
        }
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
}
