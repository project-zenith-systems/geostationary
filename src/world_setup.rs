use atmospherics::GasGrid;
use bevy::prelude::*;
use bevy::state::state_scoped::DespawnOnExit;
use network::Server;
use physics::{Collider, GravityScale, LockedAxes, Restitution, RigidBody};
use things::{Thing, ThingKind, ThingRegistry};
use tiles::Tilemap;

use crate::app_state::AppState;
use crate::config::AppConfig;

const BALL_RADIUS: f32 = 0.3;
const BALL_COLOR: (f32, f32, f32) = (1.0, 0.8, 0.0); // Bright yellow

/// System that sets up the world when entering InGame state.
pub fn setup_world(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
    config: Res<AppConfig>,
    mut server: ResMut<Server>,
) {
    let tilemap = Tilemap::test_room();
    let gas_grid =
        atmospherics::initialize_gas_grid(&tilemap, config.atmospherics.standard_pressure);

    // Insert resources
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
    let ball_net_id = server.next_net_id();
    let ball_mesh = meshes.add(Sphere::new(BALL_RADIUS));
    let ball_material = materials.add(StandardMaterial {
        base_color: Color::srgb(BALL_COLOR.0, BALL_COLOR.1, BALL_COLOR.2),
        ..default()
    });

    commands.spawn((
        ball_net_id,
        Thing,
        ThingKind(1),
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
        app.add_systems(
            OnEnter(AppState::InGame),
            setup_world.run_if(resource_exists::<Server>),
        );
        app.add_systems(OnExit(AppState::InGame), cleanup_world);

        // Register ball as thing kind 1 so clients can spawn the correct visual
        // when they receive EntitySpawned { kind: 1 } on stream 3.
        app.world_mut()
            .resource_mut::<ThingRegistry>()
            .register(1, |entity, _event, commands| {
                // `on_spawn_thing` inserts creature defaults (capsule mesh, locked axes,
                // zero gravity) for all kinds. Override them here with ball-appropriate
                // values. Uses commands.queue to access Assets inside an observer.
                commands.queue(move |world: &mut World| {
                    let mesh = world
                        .resource_mut::<Assets<Mesh>>()
                        .add(Sphere::new(BALL_RADIUS));
                    let mat = world
                        .resource_mut::<Assets<StandardMaterial>>()
                        .add(StandardMaterial {
                            base_color: Color::srgb(BALL_COLOR.0, BALL_COLOR.1, BALL_COLOR.2),
                            ..default()
                        });
                    if let Ok(mut e) = world.get_entity_mut(entity) {
                        e.insert((
                            Mesh3d(mesh),
                            MeshMaterial3d(mat),
                            Collider::sphere(BALL_RADIUS),
                            // Unlock all axes (on_spawn_thing locks rotation+y for creatures).
                            LockedAxes::default(),
                            GravityScale(1.0),
                            Restitution::new(0.8),
                        ));
                    }
                });
            });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_atmosphere_initialization() {
        const TEST_STANDARD_PRESSURE: f32 = 101.325;

        // Create a test tilemap
        let tilemap = Tilemap::test_room();

        // Initialize GasGrid using the atmospherics module function
        let gas_grid = atmospherics::initialize_gas_grid(&tilemap, TEST_STANDARD_PRESSURE);

        // Verify that floor cells have standard pressure
        let mut floor_cells_checked = 0;
        let mut wall_cells_checked = 0;

        for y in 0..tilemap.height() {
            for x in 0..tilemap.width() {
                let pos = IVec2::new(x as i32, y as i32);
                if tilemap.is_walkable(pos) {
                    // Floor cells should have standard pressure
                    assert_eq!(
                        gas_grid.pressure_at(pos),
                        Some(TEST_STANDARD_PRESSURE),
                        "Floor cell at {:?} should have standard pressure",
                        pos
                    );
                    floor_cells_checked += 1;
                } else {
                    // Wall cells should have zero pressure (not filled)
                    assert_eq!(
                        gas_grid.pressure_at(pos),
                        Some(0.0),
                        "Wall cell at {:?} should have zero pressure",
                        pos
                    );
                    wall_cells_checked += 1;
                }
            }
        }

        // Verify we checked both types of cells
        assert!(floor_cells_checked > 0, "Should have some floor cells");
        assert!(wall_cells_checked > 0, "Should have some wall cells");

        // Verify total moles equals floor cells * standard pressure
        let expected_total_moles = floor_cells_checked as f32 * TEST_STANDARD_PRESSURE;
        let actual_total_moles = gas_grid.total_moles();
        assert!(
            (actual_total_moles - expected_total_moles).abs() < 0.1,
            "Total moles {} should be close to expected {}",
            actual_total_moles,
            expected_total_moles
        );
    }
}
