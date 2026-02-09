use bevy::prelude::*;
use bevy::state::state_scoped::DespawnOnExit;
use things::Thing;
use tiles::{TileKind, Tilemap};

use crate::app_state::AppState;
use crate::creatures::{Creature, MovementSpeed, PlayerControlled};

/// System that sets up the world when entering InGame state.
/// Spawns a 12x10 room with walls and internal obstacles for collision testing.
pub fn setup_world(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    // Create a 12x10 tilemap with walls around the edges (absorbing issue #11)
    let mut tilemap = Tilemap::new(12, 10, TileKind::Floor);

    // Add walls around the perimeter
    for x in 0..12 {
        tilemap.set(IVec2::new(x, 0), TileKind::Wall);
        tilemap.set(IVec2::new(x, 9), TileKind::Wall);
    }
    for y in 0..10 {
        tilemap.set(IVec2::new(0, y), TileKind::Wall);
        tilemap.set(IVec2::new(11, y), TileKind::Wall);
    }

    // Add internal walls for collision testing (issue #11)
    // Vertical wall segment
    for y in 2..6 {
        tilemap.set(IVec2::new(4, y), TileKind::Wall);
    }
    // Horizontal wall segment
    for x in 7..10 {
        tilemap.set(IVec2::new(x, 5), TileKind::Wall);
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
        DespawnOnExit(AppState::InGame),
    ));

    // Spawn player character with a capsule mesh on a walkable floor tile
    // Capsule3d::new(0.3, 1.0) has total height = 1.0 + 2*0.3 = 1.6
    // Position at y = 0.8 (half of total height) to sit on floor
    let player_mesh = meshes.add(Capsule3d::new(0.3, 1.0));
    let player_material = materials.add(StandardMaterial {
        base_color: Color::srgb(0.2, 0.5, 0.8),
        ..default()
    });

    commands.spawn((
        Mesh3d(player_mesh),
        MeshMaterial3d(player_material),
        Transform::from_xyz(6.0, 0.8, 5.0), // Place at walkable floor tile, y=0.8 for capsule to sit on floor
        PlayerControlled,
        Creature,
        MovementSpeed::default(),
        Thing,
        DespawnOnExit(AppState::InGame),
    ));
}

/// System that cleans up the world when exiting InGame state.
fn cleanup_world(mut commands: Commands) {
    commands.remove_resource::<Tilemap>();
}

pub struct WorldSetupPlugin;

impl Plugin for WorldSetupPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(OnEnter(AppState::InGame), setup_world);
        app.add_systems(OnExit(AppState::InGame), cleanup_world);
    }
}
