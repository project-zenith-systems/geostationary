use bevy::prelude::*;
use bevy::state::state_scoped::DespawnOnExit;
use physics::{Collider, Restitution, RigidBody};
use tiles::Tilemap;
use atmospherics::GasGrid;

use crate::app_state::AppState;

/// System that sets up the world when entering InGame state.
pub fn setup_world(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let tilemap = Tilemap::test_room();
    
    // Initialize atmospheric gas grid with standard pressure (1.0 moles per cell)
    let mut gas_grid = GasGrid::new(tilemap.width(), tilemap.height());
    gas_grid.sync_walls(&tilemap);
    
    // Set all passable cells to standard pressure (1.0 moles)
    for y in 0..tilemap.height() {
        for x in 0..tilemap.width() {
            let pos = IVec2::new(x as i32, y as i32);
            if tilemap.is_walkable(pos) {
                gas_grid.set_moles(pos, 1.0);
            }
        }
    }
    
    commands.insert_resource(tilemap);
    commands.insert_resource(gas_grid);

    // Spawn a light
    commands.spawn((
        DirectionalLight {
            illuminance: 10000.0,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(4.0, 8.0, 4.0).looking_at(Vec3::new(4.0, 0.0, 4.0), Vec3::Y),
        DespawnOnExit(AppState::InGame),
    ));

    // Player capsule spawn removed - now handled by server.rs and client.rs

    // Spawn a bouncing ball above the floor
    // Position it at y=5.0 so it has room to fall and bounce
    const BALL_RADIUS: f32 = 0.3;
    let ball_mesh = meshes.add(Sphere::new(BALL_RADIUS));
    let ball_material = materials.add(StandardMaterial {
        base_color: Color::srgb(1.0, 0.8, 0.0), // Bright yellow
        ..default()
    });

    commands.spawn((
        Mesh3d(ball_mesh),
        MeshMaterial3d(ball_material),
        Transform::from_xyz(6.0, 5.0, 3.0), // Above the floor, centered in a walkable area
        RigidBody::Dynamic,
        Collider::sphere(BALL_RADIUS),
        Restitution::new(0.8),
        DespawnOnExit(AppState::InGame),
    ));
}

/// System that cleans up the world when exiting InGame state.
fn cleanup_world(mut commands: Commands) {
    commands.remove_resource::<Tilemap>();
    commands.remove_resource::<GasGrid>();
}

pub struct WorldSetupPlugin;

impl Plugin for WorldSetupPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppState::InGame), setup_world);
        app.add_systems(OnExit(AppState::InGame), cleanup_world);
    }
}
