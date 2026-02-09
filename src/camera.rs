use bevy::prelude::*;
use bevy::state::state_scoped::DespawnOnExit;

use crate::app_state::AppState;
use crate::creatures::PlayerControlled;

/// Marker component for the follow camera.
#[derive(Component)]
pub struct FollowCamera;

/// Camera configuration for smooth following behavior.
#[derive(Resource)]
pub struct CameraConfig {
    /// How quickly the camera follows the target (0.0 = no follow, 1.0 = instant)
    pub follow_speed: f32,
    /// Fixed offset from the target position
    pub offset: Vec3,
}

impl Default for CameraConfig {
    fn default() -> Self {
        Self {
            follow_speed: 2.0,
            // Position camera at roughly 50 degree angle looking down
            // Offset: 8 units back, 10 units up from player position
            offset: Vec3::new(0.0, 10.0, 8.0),
        }
    }
}

pub struct CameraPlugin;

impl Plugin for CameraPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(CameraConfig::default());
        app.add_systems(OnEnter(AppState::InGame), spawn_camera);
        app.add_systems(
            Update,
            camera_follow_system.run_if(in_state(AppState::InGame)),
        );
    }
}

/// Spawns the 3D follow camera when entering InGame state.
fn spawn_camera(mut commands: Commands, _config: Res<CameraConfig>) {
    // Start camera at default position (will move to player in first frame)
    let camera_pos = Vec3::new(6.0, 10.0, 10.0);
    let look_target = Vec3::new(6.0, 0.0, 5.0);

    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(camera_pos).looking_at(look_target, Vec3::Y),
        FollowCamera,
        DespawnOnExit(AppState::InGame),
    ));
}

/// System that smoothly follows the PlayerControlled entity.
fn camera_follow_system(
    time: Res<Time>,
    config: Res<CameraConfig>,
    player_query: Query<&Transform, (With<PlayerControlled>, Without<FollowCamera>)>,
    mut camera_query: Query<&mut Transform, With<FollowCamera>>,
) {
    // Get player position
    let Ok(player_transform) = player_query.single() else {
        return;
    };

    // Get camera transform
    let Ok(mut camera_transform) = camera_query.single_mut() else {
        return;
    };

    // Calculate target camera position (player position + offset)
    let target_position = player_transform.translation + config.offset;

    // Smoothly interpolate camera position using lerp
    let lerp_factor = (config.follow_speed * time.delta_secs()).min(1.0);
    camera_transform.translation = camera_transform
        .translation
        .lerp(target_position, lerp_factor);

    // Always look at the player
    camera_transform.look_at(player_transform.translation, Vec3::Y);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_camera_config_default() {
        let config = CameraConfig::default();
        assert_eq!(config.follow_speed, 2.0);
        assert_eq!(config.offset, Vec3::new(0.0, 10.0, 8.0));
    }

    #[test]
    fn test_camera_config_custom() {
        let config = CameraConfig {
            follow_speed: 5.0,
            offset: Vec3::new(0.0, 15.0, 10.0),
        };
        assert_eq!(config.follow_speed, 5.0);
        assert_eq!(config.offset, Vec3::new(0.0, 15.0, 10.0));
    }
}
