use bevy::prelude::*;
use things::InputDirection;

/// Marker component for player-controlled entities (camera target, input receiver).
#[derive(Component, Debug, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct PlayerControlled;

pub struct PlayerPlugin;

impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<PlayerControlled>();
        app.add_systems(Update, read_player_input);
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
