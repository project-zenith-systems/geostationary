use bevy::prelude::*;

/// Marker component for all non-grid-bound world objects.
/// 
/// This is the L1 convention that establishes the base category for entities
/// that exist in the world but are not bound to the grid itself. Higher layers
/// (creatures, items) build on top of this marker.
#[derive(Component, Debug, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct Thing;

/// Grid-aware position component that stores both tile coordinate and sub-tile offset.
/// 
/// - `tile`: The integer tile coordinate (grid cell) where the object is located.
/// - `offset`: The sub-tile offset within that cell, typically in the range [0.0, 1.0).
///
/// This allows entities to move smoothly within the grid while maintaining
/// awareness of their discrete tile location.
#[derive(Component, Debug, Clone, Copy, Reflect)]
#[reflect(Component)]
pub struct WorldPosition {
    pub tile: IVec2,
    pub offset: Vec2,
}

impl WorldPosition {
    /// Creates a new WorldPosition at the specified tile with zero offset.
    pub fn new(tile: IVec2) -> Self {
        Self {
            tile,
            offset: Vec2::ZERO,
        }
    }

    /// Creates a new WorldPosition with both tile and offset specified.
    pub fn with_offset(tile: IVec2, offset: Vec2) -> Self {
        Self { tile, offset }
    }

    /// Returns the continuous world position by combining tile and offset.
    /// Assumes 1 tile = 1 world unit.
    pub fn to_world(&self) -> Vec2 {
        self.tile.as_vec2() + self.offset
    }
}

impl Default for WorldPosition {
    fn default() -> Self {
        Self::new(IVec2::ZERO)
    }
}

/// Plugin that registers the Things module components.
/// 
/// This is deliberately minimal - it only registers the core components.
/// Higher-level systems and resources are added by layers above.
#[derive(Default)]
pub struct ThingsPlugin;

impl Plugin for ThingsPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Thing>();
        app.register_type::<WorldPosition>();
    }
}
