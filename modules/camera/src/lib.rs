use bevy::prelude::*;
use bevy::state::state_scoped::DespawnOnExit;
use player::PlayerControlled;

/// Marker component for the follow camera.
#[derive(Component)]
pub struct FollowCamera;

/// Camera configuration for smooth following behavior.
#[derive(Resource)]
pub struct CameraConfig {
    /// How quickly the camera follows the target (higher values = faster following)
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

/// Resource storing the active state value for `DespawnOnExit`.
#[derive(Resource, Clone, Copy)]
struct CameraActiveState<S: States>(S);

pub struct CameraPlugin<S: States + Copy> {
    state: S,
}

impl<S: States + Copy> CameraPlugin<S> {
    pub fn in_state(state: S) -> Self {
        Self { state }
    }
}

impl<S: States + Copy> Plugin for CameraPlugin<S> {
    fn build(&self, app: &mut App) {
        let state = self.state;
        app.insert_resource(CameraConfig::default());
        app.insert_resource(CameraActiveState(state));
        app.add_systems(OnEnter(state), spawn_camera::<S>);
        app.add_systems(Update, camera_follow_system.run_if(in_state(state)));
    }
}

/// Spawns the 3D follow camera when entering the active state.
fn spawn_camera<S: States + Copy>(mut commands: Commands, active_state: Res<CameraActiveState<S>>) {
    // Start camera at default position (will move to player in first frame)
    let camera_pos = Vec3::new(6.0, 10.0, 10.0);
    let look_target = Vec3::new(6.0, 0.0, 5.0);

    commands.spawn((
        Camera3d::default(),
        Transform::from_translation(camera_pos).looking_at(look_target, Vec3::Y),
        AmbientLight {
            color: Color::WHITE,
            brightness: 300.0,
            affects_lightmapped_meshes: true,
        },
        FollowCamera,
        DespawnOnExit(active_state.0),
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

    /// Verifies that camera_follow_system moves the camera toward a
    /// PlayerControlled entity's position + offset.
    #[test]
    fn test_camera_follows_player_controlled_entity() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);

        let offset = Vec3::new(0.0, 10.0, 8.0);
        app.insert_resource(CameraConfig {
            follow_speed: 10_000.0,
            offset,
        });
        app.add_systems(Update, camera_follow_system);

        // Player at a known position
        let player_pos = Vec3::new(5.0, 0.0, 5.0);
        app.world_mut()
            .spawn((Transform::from_translation(player_pos), PlayerControlled));

        // Camera starts far away
        let camera_start = Vec3::new(50.0, 50.0, 50.0);
        let camera_entity = app
            .world_mut()
            .spawn((Transform::from_translation(camera_start), FollowCamera))
            .id();

        // First update initialises Time (delta ≈ 0), second has real delta
        app.update();
        app.update();

        let camera_pos = app
            .world()
            .get::<Transform>(camera_entity)
            .unwrap()
            .translation;
        let target = player_pos + offset;

        // With follow_speed=10000, lerp_factor clamps to 1.0 → camera snaps
        let dist = camera_pos.distance(target);
        assert!(
            dist < 0.1,
            "Camera should snap to player + offset. \
             Distance: {dist}, camera: {camera_pos}, target: {target}"
        );
    }
}
