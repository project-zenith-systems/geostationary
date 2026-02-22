use bevy::prelude::*;

// Re-export only the types other modules need.
pub use avian3d::prelude::{
    Collider, GravityScale, LinearVelocity, LockedAxes, PhysicsDebugPlugin, Restitution, RigidBody,
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
