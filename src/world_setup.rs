use bevy::{prelude::*, state::state_scoped::DespawnOnExit};

use crate::app_state::AppState;

pub struct WorldSetupPlugin;

impl Plugin for WorldSetupPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppState::InGame), (setup_camera, setup_lighting));
    }
}

fn setup_camera(mut commands: Commands) {
    // 3D camera with ambient light component
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(5.0, 8.0, 10.0).looking_at(Vec3::new(0.0, 0.0, 0.0), Vec3::Y),
        AmbientLight {
            color: Color::WHITE,
            brightness: 300.0,
            affects_lightmapped_meshes: true,
        },
        DespawnOnExit(AppState::InGame),
    ));
}

fn setup_lighting(mut commands: Commands) {
    // Directional light (sun-like)
    commands.spawn((
        DirectionalLight {
            illuminance: 10000.0,
            shadows_enabled: false,
            ..default()
        },
        Transform::from_rotation(Quat::from_euler(EulerRot::XYZ, -0.8, 0.5, 0.0)),
        DespawnOnExit(AppState::InGame),
    ));
}
