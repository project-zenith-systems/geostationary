use bevy::prelude::*;

// Re-export only the types other modules need.
pub use avian3d::prelude::{
    Collider, ConstantForce, GravityScale, LinearVelocity, LockedAxes, PhysicsDebugPlugin,
    Restitution, RigidBody, SpatialQuery, SpatialQueryFilter,
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

    /// Build a headless `App` with physics, transform propagation, and a fixed
    /// 60 Hz timestep. Uses default gravity (9.81 m/s² downward).
    fn test_app() -> App {
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
        app.finish();
        app
    }

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
        let mut app = test_app();
        // Disable gravity so only ConstantForce contributes.
        app.insert_resource(avian3d::prelude::Gravity(Vec3::ZERO));

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

    /// Spike result: reparenting and physics component removal in Avian — (a).
    ///
    /// Findings — transform propagation:
    /// - Removing `RigidBody`/`Collider`/`LinearVelocity`/`GravityScale` from a
    ///   dynamic entity and inserting `ChildOf(parent)` causes the entity to
    ///   follow its parent correctly.
    /// - After one `app.update()` the entity's `GlobalTransform` reflects the
    ///   parent's world position plus the child's local offset. No special
    ///   handling is required beyond the standard Bevy transform propagation.
    #[test]
    fn reparented_entity_follows_parent() {
        let mut app = test_app();
        app.insert_resource(avian3d::prelude::Gravity(Vec3::ZERO));

        // Spawn a stationary parent at (5, 3, 0) (no physics, no RigidBody).
        let parent = app
            .world_mut()
            .spawn(Transform::from_translation(Vec3::new(5.0, 3.0, 0.0)))
            .id();

        // Spawn a dynamic entity at the origin.
        let child = app
            .world_mut()
            .spawn((
                RigidBody::Dynamic,
                Collider::sphere(0.5),
                Transform::default(),
            ))
            .id();

        // Remove physics and reparent in one chain.
        app.world_mut()
            .entity_mut(child)
            .remove::<(RigidBody, Collider, LinearVelocity, GravityScale)>()
            .insert(ChildOf(parent));

        // Two updates: the first processes the ChildOf insertion; the second
        // runs transform propagation and settles the GlobalTransform.
        app.update();
        app.update();

        // The entity's world position must equal the parent's world position
        // (local offset is zero).
        let global = app.world().get::<GlobalTransform>(child).unwrap();
        let pos = global.translation();
        assert!(
            (pos.x - 5.0).abs() < 0.01,
            "expected x ≈ 5.0 (parent x), got x = {}",
            pos.x
        );
        assert!(
            (pos.y - 3.0).abs() < 0.01,
            "expected y ≈ 3.0 (parent y), got y = {}",
            pos.y
        );
        assert!(
            pos.z.abs() < 0.01,
            "expected z ≈ 0.0, got z = {}",
            pos.z
        );
    }

    /// Spike result: reparenting and physics component removal in Avian — (b).
    ///
    /// Findings — restoring physics after deparenting:
    /// - Deparenting via `remove::<ChildOf>()` and re-inserting `RigidBody`,
    ///   `Collider`, and `GravityScale` fully restores normal simulation.
    /// - The entity acquires downward velocity under gravity after two physics
    ///   steps, identical to a body that was never reparented.
    /// - Despawn/respawn is therefore unnecessary; reparenting with component
    ///   removal/re-insertion is sufficient to toggle physics on and off.
    #[test]
    fn deparented_entity_with_restored_physics_falls() {
        let mut app = test_app();

        // Spawn a dynamic entity elevated at y = 10.
        let entity = app
            .world_mut()
            .spawn((
                RigidBody::Dynamic,
                Collider::sphere(0.5),
                Transform::from_translation(Vec3::new(0.0, 10.0, 0.0)),
            ))
            .id();

        // Spawn a stationary parent at the origin.
        let parent = app
            .world_mut()
            .spawn(Transform::default())
            .id();

        // Remove physics and reparent — entity should stop falling.
        app.world_mut()
            .entity_mut(entity)
            .remove::<(RigidBody, Collider, LinearVelocity, GravityScale)>()
            .insert(ChildOf(parent));

        // Two updates confirm the entity does not fall while physics are absent.
        app.update();
        app.update();

        // Entity has no LinearVelocity component while physics are absent.
        assert!(
            app.world().get::<LinearVelocity>(entity).is_none(),
            "entity must not have LinearVelocity while physics components are removed"
        );

        // Deparent and restore physics.
        app.world_mut()
            .entity_mut(entity)
            .remove::<ChildOf>()
            .insert((
                RigidBody::Dynamic,
                Collider::sphere(0.5),
                GravityScale(1.0),
            ));

        // Two updates: Avian detects the new RigidBody on the first update and
        // advances the simulation by one fixed step on the second.
        app.update();
        app.update();

        let vel = app.world().get::<LinearVelocity>(entity).unwrap();
        assert!(
            vel.y < 0.0,
            "expected downward velocity after restoring physics, got y = {}",
            vel.y
        );
    }

    /// Spike result: reparenting and physics component removal in Avian — (c).
    ///
    /// Findings — no panics on remove and re-insert:
    /// - Removing `RigidBody`/`Collider`/`LinearVelocity`/`GravityScale` from a
    ///   live dynamic entity and then re-inserting them in a later frame causes
    ///   no panics and no ECS errors.
    /// - Avian picks up the re-inserted components on the next physics step and
    ///   immediately resumes simulation (the entity acquires downward velocity).
    #[test]
    fn removing_and_reinserting_physics_does_not_panic() {
        let mut app = test_app();

        let entity = app
            .world_mut()
            .spawn((
                RigidBody::Dynamic,
                Collider::sphere(0.5),
                Transform::default(),
            ))
            .id();

        // Step once with physics active.
        app.update();

        // Remove all physics components.
        app.world_mut()
            .entity_mut(entity)
            .remove::<(RigidBody, Collider, LinearVelocity, GravityScale)>();

        // Step twice with no physics — must not panic.
        app.update();
        app.update();

        // Re-insert physics.
        app.world_mut().entity_mut(entity).insert((
            RigidBody::Dynamic,
            Collider::sphere(0.5),
            GravityScale(1.0),
        ));

        // Two updates: Avian detects the new RigidBody on the first update and
        // advances the simulation by one fixed step on the second.
        app.update();
        app.update();

        let vel = app.world().get::<LinearVelocity>(entity).unwrap();
        assert!(
            vel.y < 0.0,
            "expected downward velocity after re-inserting physics, got y = {}",
            vel.y
        );
    }

    /// Spike result: reparenting and physics component removal in Avian — (d).
    ///
    /// Findings — `Visibility::Hidden` on a reparented entity:
    /// - `Visibility::Hidden` can be inserted on an entity after physics removal
    ///   and reparenting without errors.
    /// - Adding `VisibilityPlugin` enables visibility propagation; after
    ///   `app.update()` the entity's `InheritedVisibility` becomes `false`,
    ///   confirming the hidden flag propagates correctly even headlessly.
    /// - In a real scene this prevents the entity from being rendered while it
    ///   is held (e.g. stored inside a container).
    #[test]
    fn visibility_hidden_on_reparented_entity() {
        // Built manually (not via test_app) because VisibilityPlugin must be
        // added before app.finish().
        let mut app = App::new();
        app.add_plugins((
            MinimalPlugins,
            TransformPlugin,
            bevy::asset::AssetPlugin::default(),
            bevy::mesh::MeshPlugin,
            bevy::scene::ScenePlugin,
            PhysicsPlugin,
            // Enable visibility propagation so InheritedVisibility is computed.
            bevy::camera::visibility::VisibilityPlugin,
        ))
        .insert_resource(TimeUpdateStrategy::ManualDuration(Duration::from_secs_f32(
            1.0 / 60.0,
        )));
        app.insert_resource(avian3d::prelude::Gravity(Vec3::ZERO));
        app.finish();

        // Spawn a parent entity.
        let parent = app
            .world_mut()
            .spawn(Transform::default())
            .id();

        // Spawn a dynamic entity.
        let entity = app
            .world_mut()
            .spawn((
                RigidBody::Dynamic,
                Collider::sphere(0.5),
                Transform::default(),
            ))
            .id();

        // Remove physics, reparent, and hide — all in one chain.
        app.world_mut()
            .entity_mut(entity)
            .remove::<(RigidBody, Collider, LinearVelocity, GravityScale)>()
            .insert((ChildOf(parent), Visibility::Hidden));

        app.update();

        // Visibility::Hidden must survive the update.
        let vis = app.world().get::<Visibility>(entity).unwrap();
        assert_eq!(
            *vis,
            Visibility::Hidden,
            "expected Visibility::Hidden to be preserved, got {:?}",
            vis
        );

        // InheritedVisibility must be false — confirms the visibility system
        // processed the hidden flag and would exclude this entity from rendering.
        let inherited = app.world().get::<InheritedVisibility>(entity).unwrap();
        assert!(
            !inherited.get(),
            "expected InheritedVisibility to be false for a hidden entity"
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
        let mut app = test_app();

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
