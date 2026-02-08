use bevy::prelude::*;

/// Marker component for non-grid-bound world objects.
#[derive(Component, Debug, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct Thing;

#[derive(Default)]
pub struct ThingsPlugin;

impl Plugin for ThingsPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Thing>();
    }
}
