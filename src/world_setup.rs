use bevy::prelude::*;
use tiles::{TileKind, Tilemap};
use things::Thing;

use crate::app_state::AppState;
use crate::creatures::{Creature, MovementSpeed, PlayerControlled};

/// System that sets up the world when entering InGame state.
/// Spawns a simple room with walls and a player character.
pub fn setup_world(mut commands: Commands, mut meshes: ResMut<Assets<Mesh>>, mut materials: ResMut<Assets<StandardMaterial>>) {
    // Create a simple 10x10 tilemap with walls around the edges
    let mut tilemap = Tilemap::new(10, 10, TileKind::Floor);
    
    // Add walls around the perimeter
    for x in 0..10 {
        tilemap.set(IVec2::new(x, 0), TileKind::Wall);
        tilemap.set(IVec2::new(x, 9), TileKind::Wall);
    }
    for y in 0..10 {
        tilemap.set(IVec2::new(0, y), TileKind::Wall);
        tilemap.set(IVec2::new(9, y), TileKind::Wall);
    }
    
    // Insert the tilemap as a resource
    commands.insert_resource(tilemap);
    
    // Spawn a light
    commands.spawn((
        DirectionalLight {
            illuminance: 10000.0,
            shadows_enabled: true,
            ..default()
        },
        Transform::from_xyz(4.0, 8.0, 4.0).looking_at(Vec3::new(4.0, 0.0, 4.0), Vec3::Y),
    ));
    
    // Spawn player character with a capsule mesh on a walkable floor tile (e.g., position 5, 5)
    let player_mesh = meshes.add(Capsule3d::new(0.3, 1.0));
    let player_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.2, 0.5, 0.8),
        ..default()
    });
    
    commands.spawn((
        Mesh3d(player_mesh),
        MeshMaterial3d(player_material),
        Transform::from_xyz(5.0, 0.5, 5.0), // Place at walkable floor tile (5, 5), y=0.5 for capsule half-height
        PlayerControlled,
        Creature,
        MovementSpeed::default(),
        Thing,
    ));
    
    // Spawn a 3D camera
    commands.spawn((
        Camera3d::default(),
        Transform::from_xyz(5.0, 10.0, 10.0).looking_at(Vec3::new(5.0, 0.0, 5.0), Vec3::Y),
    ));
}

pub struct WorldSetupPlugin;

impl Plugin for WorldSetupPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppState::InGame), setup_world);
    }
}
