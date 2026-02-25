use bevy::prelude::*;

// Re-export only the types other modules need.
pub use avian3d::prelude::{
    Collider, ConstantForce, GravityScale, LinearVelocity, LockedAxes, PhysicsDebugPlugin,
    Restitution, RigidBody, SpatialQuery,
};

pub struct PhysicsPlugin;

impl Plugin for PhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(avian3d::PhysicsPlugins::default().with_length_unit(1.0));

        // Standard downward gravity.
        app.insert_resource(avian3d::prelude::Gravity(Vec3::NEG_Y * 9.81));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bevy::time::TimeUpdateStrategy;
    use std::time::Duration;

    /// Spike result: `ConstantForce` semantics in avian3d 0.5.
    ///
    /// Findings:
    /// - `ConstantForce` **persists** across frames; it is NOT reset after each physics step.
    /// - To apply a varying force each tick, overwrite the component value each `FixedUpdate`.
    ///   Simply assigning a new value produces no accumulation.
    /// - `ConstantForce` can be inserted at runtime on entities that were spawned without it.
    ///   Avian picks it up on the next physics step.
    /// - For the pressure-force system: set `ConstantForce` to the gradient-derived force in
    ///   `FixedUpdate` each tick; no manual clearing is needed between ticks as long as we
    ///   always overwrite the component with the current value (including Vec3::ZERO when the
    ///   gradient is negligible).
    #[test]
    fn constant_force_persists_and_integrates() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            bevy::asset::AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
            bevy::scene::ScenePlugin,
            PhysicsPlugin,
        ))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
            1.0 / 60.0,
        )));
        // Disable gravity so only ConstantForce contributes.
        app.insert_resource(avian3d::prelude::Gravity(Vec3::ZERO));
        app.finish();

        // Spawn without ConstantForce initially.
        let entity = app
            .world_mut()
            .spawn((
                RigidBody::Dynamic,
                Collider::sphere(0.5),
                Transform::default(),
            ))
            .id();

        // Verify: runtime insertion – insert ConstantForce after spawn.
        app.world_mut()
            .entity_mut(entity)
            .insert(ConstantForce::new(10.0, 0.0, 0.0));

        // Step once; force should push the body in +X.
        app.update();
        app.update();

        let vel_after_force = app.world().get::<LinearVelocity>(entity).unwrap().x;
        assert!(
            vel_after_force > 0.0,
            "expected positive-x velocity from ConstantForce(10, 0, 0), got x = {}",
            vel_after_force
        );

        // Overwrite with zero force; velocity should stop increasing.
        app.world_mut()
            .entity_mut(entity)
            .insert(ConstantForce::new(0.0, 0.0, 0.0));

        app.update();
        app.update();

        let vel_after_zero = app.world().get::<LinearVelocity>(entity).unwrap().x;
        assert!(
            (vel_after_zero - vel_after_force).abs() < 0.01,
            "velocity should not change when ConstantForce is zero; before={}, after={}",
            vel_after_force,
            vel_after_zero
        );
    }

    /// Spike result: PhysicsPlugin works headless with the plugin set below.
    /// No window or rendering plugins are required for physics simulation.
    ///
    /// Minimal plugin set:
    ///   MinimalPlugins              – time, scheduling
    ///   TransformPlugin             – Transform / GlobalTransform propagation
    ///   AssetPlugin::default        – required by avian3d internals
    ///   bevy::mesh::MeshPlugin      – registers Mesh asset + AssetEvent<Mesh>
    ///                                 messages consumed by avian3d's ColliderCachePlugin
    ///                                 (avian3d default: collider-from-mesh feature)
    ///   bevy::scene::ScenePlugin    – provides SceneSpawner resource consumed by
    ///                                 avian3d's ColliderBackendPlugin
    ///                                 (avian3d default: bevy_scene feature)
    ///   PhysicsPlugin               – avian3d PhysicsPlugins + gravity resource
    #[test]
    fn headless_physics_steps_with_gravity() {
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            bevy::asset::AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
            bevy::scene::ScenePlugin,
            PhysicsPlugin,
        ))
        // Fix the timestep so physics advances deterministically in tests.
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
            1.0 / 60.0,
        )));
        app.finish();

        let entity = app
            .world_mut()
            .spawn((
                RigidBody::Dynamic,
                Collider::sphere(0.5),
                Transform::default(),
            ))
            .id();

        // Step the schedule twice.
        app.update();
        app.update();

        // After two fixed steps with 9.81 m/s² downward gravity the body
        // must have acquired a negative-y velocity.
        let vel = app.world().get::<LinearVelocity>(entity).unwrap();
        assert!(
            vel.y < 0.0,
            "expected downward velocity after two physics steps, got y = {}",
            vel.y
        );
    }
}
