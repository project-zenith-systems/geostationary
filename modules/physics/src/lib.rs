use bevy::prelude::*;

// Re-export only the types other modules need.
pub use avian3d::prelude::{
    Collider, LinearVelocity, LockedAxes, Restitution, RigidBody, SweptCcd,
};

pub struct PhysicsPlugin;

impl Plugin for PhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_plugins(
            avian3d::PhysicsPlugins::default()
                .with_length_unit(1.0),
        );

        // Standard downward gravity.
        app.insert_resource(avian3d::prelude::Gravity(Vec3::NEG_Y * 9.81));
    }
}

#[cfg(test)]
mod tests {
}
