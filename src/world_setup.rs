use atmospherics::GasGrid;
use bevy::prelude::*;
use bevy::state::state_scoped::DespawnOnExit;
use network::{Headless, Server};
use physics::{Collider, GravityScale, Restitution};
use things::ThingRegistry;
use tiles::Tilemap;

use crate::app_state::AppState;
use crate::config::AppConfig;

const BALL_RADIUS: f32 = 0.3;
const BALL_COLOR: (f32, f32, f32) = (1.0, 0.8, 0.0); // Bright yellow

/// System that sets up the world when entering InGame state (server only).
pub fn setup_world(
    mut commands: Commands,
    config: Res<AppConfig>,
    mut server: ResMut<Server>,
) {
    let tilemap = Tilemap::test_room();
    let gas_grid = atmospherics::initialize_gas_grid(
        &tilemap,
        config.atmospherics.standard_pressure,
        Some((IVec2::new(11, 1), IVec2::new(14, 8))),
    );

    // Insert resources
    commands.insert_resource(tilemap);
    commands.insert_resource(gas_grid);
    commands.insert_resource(atmospherics::PressureForceScale(
        config.atmospherics.pressure_force_scale,
    ));

    // Player capsule spawn removed - now handled by server.rs and client.rs

    // Spawn a bouncing ball above the floor using the thing prefab system (kind 1).
    // Position it at y=5.0 so it has room to fall and bounce.
    let (ball, _net_id) =
        things::spawn_thing(&mut commands, &mut server, 1, Vec3::new(6.0, 5.0, 3.0));
    commands.entity(ball).insert(DespawnOnExit(AppState::InGame));
}

/// System that cleans up the world when exiting InGame state.
fn cleanup_world(mut commands: Commands) {
    commands.remove_resource::<Tilemap>();
    commands.remove_resource::<GasGrid>();
    commands.remove_resource::<atmospherics::PressureForceScale>();
}

pub struct WorldSetupPlugin;

impl Plugin for WorldSetupPlugin {
    fn build(&self, app: &mut App) {
        let headless = app.world().contains_resource::<Headless>();

        if headless {
            // In headless mode, no rendering is available: register the ball with physics only.
            app.world_mut()
                .resource_mut::<ThingRegistry>()
                .register(1, |entity, _event, commands| {
                    commands.entity(entity).insert((
                        Collider::sphere(BALL_RADIUS),
                        GravityScale(1.0),
                        Restitution::new(0.8),
                    ));
                });
        } else {
            // Pre-load ball assets once at startup so every spawned ball reuses the same handles.
            let ball_mesh = app
                .world_mut()
                .resource_mut::<Assets<Mesh>>()
                .add(Sphere::new(BALL_RADIUS));
            let ball_mat = app
                .world_mut()
                .resource_mut::<Assets<StandardMaterial>>()
                .add(StandardMaterial {
                    base_color: Color::srgb(BALL_COLOR.0, BALL_COLOR.1, BALL_COLOR.2),
                    ..default()
                });

            // Register ball as thing kind 1: overrides the default capsule from on_spawn_thing
            // with a sphere mesh, sphere collider, standard gravity, and bounciness.
            app.world_mut()
                .resource_mut::<ThingRegistry>()
                .register(1, move |entity, _event, commands| {
                    commands.entity(entity).insert((
                        Mesh3d(ball_mesh.clone()),
                        MeshMaterial3d(ball_mat.clone()),
                        Collider::sphere(BALL_RADIUS),
                        GravityScale(1.0),
                        Restitution::new(0.8),
                    ));
                });
        }

        // Run setup_world as soon as Server exists and Tilemap hasn't been created yet.
        // This decouples world creation from the InGame state transition, which is
        // necessary on a listen-server: the client sync barrier requires tiles/atmos
        // StreamReady sentinels, but those need the Tilemap/GasGrid resources that
        // setup_world creates. Running here breaks the deadlock.
        app.add_systems(
            Update,
            setup_world.run_if(
                resource_exists::<Server>.and(not(resource_exists::<Tilemap>)),
            ),
        );
        app.add_systems(OnExit(AppState::InGame), cleanup_world);
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
        let gas_grid = atmospherics::initialize_gas_grid(&tilemap, TEST_STANDARD_PRESSURE, None);

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

    #[test]
    fn test_atmosphere_vacuum_region() {
        const TEST_STANDARD_PRESSURE: f32 = 101.325;

        let tilemap = Tilemap::test_room();

        // Initialize with the right chamber (cols 11–14, rows 1–8) as vacuum
        let vacuum_min = IVec2::new(11, 1);
        let vacuum_max = IVec2::new(14, 8);
        let gas_grid = atmospherics::initialize_gas_grid(
            &tilemap,
            TEST_STANDARD_PRESSURE,
            Some((vacuum_min, vacuum_max)),
        );

        // Left chamber floor cells (cols 1–8) should have standard pressure
        for y in 1..9i32 {
            for x in 1..9i32 {
                let pos = IVec2::new(x, y);
                assert_eq!(
                    gas_grid.pressure_at(pos),
                    Some(TEST_STANDARD_PRESSURE),
                    "Left chamber cell at {:?} should have standard pressure",
                    pos
                );
            }
        }

        // Right chamber floor cells (cols 11–14, rows 1–8) should be vacuum (0.0)
        for y in 1..9i32 {
            for x in 11..15i32 {
                let pos = IVec2::new(x, y);
                assert_eq!(
                    gas_grid.pressure_at(pos),
                    Some(0.0),
                    "Right chamber cell at {:?} should be vacuum (0.0 pressure)",
                    pos
                );
            }
        }
    }
}
