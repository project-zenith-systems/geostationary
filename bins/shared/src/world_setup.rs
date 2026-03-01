use atmospherics::GasGrid;
use bevy::prelude::*;
use bevy::state::state_scoped::DespawnOnExit;
use items::{Container, StashedPhysics};
use network::Server;
use physics::{Collider, GravityScale, LinearVelocity, Restitution, RigidBody};
use things::ThingRegistry;
use tiles::Tilemap;

use crate::app_state::AppState;
use crate::config::AppConfig;

pub const BALL_RADIUS: f32 = 0.3;

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
        config.atmospherics.diffusion_rate,
    );

    // Insert resources
    commands.insert_resource(tilemap);
    commands.insert_resource(gas_grid);
    commands.insert_resource(atmospherics::PressureForceScale(
        config.atmospherics.pressure_force_scale,
    ));

    // Spawn a bouncing ball above the floor using the thing prefab system (kind 1).
    // Position it at y=5.0 so it has room to fall and bounce.
    let (ball, _net_id) =
        things::spawn_thing(&mut commands, &mut server, 1, Vec3::new(6.0, 5.0, 3.0));
    commands.entity(ball).insert(DespawnOnExit(AppState::InGame));

    // Spawn a free can (kind 2) in the pressurised chamber.
    let (can1, _) =
        things::spawn_thing(&mut commands, &mut server, 2, Vec3::new(4.0, 1.0, 3.0));
    commands.entity(can1).insert(DespawnOnExit(AppState::InGame));

    // Spawn the stashed can — it will be pre-loaded inside the toolbox.
    let (can_stashed, _) =
        things::spawn_thing(&mut commands, &mut server, 2, Vec3::new(5.0, 1.0, 3.0));
    commands
        .entity(can_stashed)
        .insert(DespawnOnExit(AppState::InGame));

    // Spawn toolbox (kind 3) in the pressurised chamber.
    let (toolbox, _) =
        things::spawn_thing(&mut commands, &mut server, 3, Vec3::new(5.0, 1.0, 3.0));
    commands
        .entity(toolbox)
        .insert(DespawnOnExit(AppState::InGame));

    // After all SpawnThing triggers have been applied, stash the can inside the toolbox.
    // The can gets StashedPhysics (capturing its Collider/GravityScale) and Visibility::Hidden,
    // and its dynamic-physics components (RigidBody, LinearVelocity) are removed; the toolbox
    // Container.slots[0] is set to point at the can entity.
    commands.queue(move |world: &mut World| {
        let (collider, gravity) = {
            let e = world.entity(can_stashed);
            (
                e.get::<Collider>()
                    .expect("can kind-2 template must insert Collider")
                    .clone(),
                *e.get::<GravityScale>()
                    .expect("can kind-2 template must insert GravityScale"),
            )
        };
        world.entity_mut(can_stashed).insert((
            StashedPhysics { collider, gravity },
            Visibility::Hidden,
        ));
        world
            .entity_mut(can_stashed)
            .remove::<(RigidBody, LinearVelocity)>();

        let mut toolbox_entity = world.entity_mut(toolbox);
        let mut container = toolbox_entity
            .get_mut::<Container>()
            .expect("toolbox entity must have a Container (kind 3 template) for stashing the can");
        container.slots[0] = Some(can_stashed);
    });
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
        // Register ball as thing kind 1 with physics-only components.
        // Client binaries can override this registration to add mesh + material.
        app.world_mut()
            .resource_mut::<ThingRegistry>()
            .register(1, |entity, _event, commands| {
                commands.entity(entity).insert((
                    Collider::sphere(BALL_RADIUS),
                    GravityScale(1.0),
                    Restitution::new(0.8),
                ));
            });

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
        let gas_grid = atmospherics::initialize_gas_grid(
            &tilemap,
            TEST_STANDARD_PRESSURE,
            None,
            atmospherics::DEFAULT_DIFFUSION_RATE,
        );

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
            atmospherics::DEFAULT_DIFFUSION_RATE,
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

    /// Verifies the stash logic used in `setup_world`:
    /// a can pre-loaded into a toolbox gets `StashedPhysics` + `Visibility::Hidden`
    /// with its dynamic-physics components (e.g. `RigidBody`, `LinearVelocity`, `Restitution`)
    /// removed while retaining `Collider` and `GravityScale`, and the toolbox
    /// `Container.slots[0]` holds the can's entity reference.
    #[test]
    fn test_toolbox_preloaded_with_stashed_can() {
        use bevy::time::TimeUpdateStrategy;
        use items::Item;
        use physics::PhysicsPlugin;
        use std::time::Duration;

        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            bevy::asset::AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
            bevy::scene::ScenePlugin,
            PhysicsPlugin,
            bevy::camera::visibility::VisibilityPlugin,
        ))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
            1.0 / 60.0,
        )));
        app.finish();

        // Spawn a can with the same components the kind-2 template inserts.
        let can = app
            .world_mut()
            .spawn((
                Transform::default(),
                RigidBody::Dynamic,
                Collider::cylinder(0.15, 0.1),
                GravityScale(1.0),
                LinearVelocity::default(),
                Item,
            ))
            .id();

        // Spawn a toolbox with the same components the kind-3 template inserts.
        let toolbox = app
            .world_mut()
            .spawn((Transform::default(), Container::with_capacity(6)))
            .id();

        // Apply the same stash logic as the commands.queue() closure in setup_world.
        let (collider, gravity) = {
            let e = app.world().entity(can);
            (
                e.get::<Collider>()
                    .expect("can kind-2 template must insert Collider")
                    .clone(),
                *e.get::<GravityScale>()
                    .expect("can kind-2 template must insert GravityScale"),
            )
        };
        {
            let world = app.world_mut();
            world
                .entity_mut(can)
                .insert((StashedPhysics { collider, gravity }, Visibility::Hidden));
            world
                .entity_mut(can)
                .remove::<(RigidBody, LinearVelocity)>();
            let mut toolbox_entity = world.entity_mut(toolbox);
            let mut container = toolbox_entity
                .get_mut::<Container>()
                .expect("toolbox entity must have a Container (kind 3 template)");
            container.slots[0] = Some(can);
        }

        // Toolbox slot 0 must reference the stashed can.
        let container = app.world().get::<Container>(toolbox).unwrap();
        assert_eq!(
            container.slots[0],
            Some(can),
            "toolbox slot 0 should be the stashed can"
        );

        // Stashed can must have StashedPhysics and be hidden.
        assert!(
            app.world().get::<StashedPhysics>(can).is_some(),
            "stashed can must have StashedPhysics"
        );
        assert_eq!(
            app.world().get::<Visibility>(can),
            Some(&Visibility::Hidden),
            "stashed can must have Visibility::Hidden"
        );

        // RigidBody and LinearVelocity must have been stripped (no live physics),
        // but Collider and GravityScale are kept so ItemTakeRequest validation passes.
        assert!(
            app.world().get::<RigidBody>(can).is_none(),
            "stashed can must have no RigidBody"
        );
        assert!(
            app.world().get::<LinearVelocity>(can).is_none(),
            "stashed can must have no LinearVelocity"
        );
        assert!(
            app.world().get::<Collider>(can).is_some(),
            "stashed can must retain Collider so ItemTakeRequest can validate it"
        );
    }
}
