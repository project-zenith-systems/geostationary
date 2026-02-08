use bevy::prelude::*;

/// Marker component for non-grid-bound world objects.
#[derive(Component, Debug, Clone, Copy, Default, Reflect)]
#[reflect(Component)]
pub struct Thing;

/// Grid-aware position: tile coordinate + sub-tile offset [0.0, 1.0).
#[derive(Component, Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(Component)]
pub struct WorldPosition {
    pub tile: IVec2,
    pub offset: Vec2,
}

impl WorldPosition {
    pub fn new(tile: IVec2) -> Self {
        Self {
            tile,
            offset: Vec2::ZERO,
        }
    }

    pub fn with_offset(tile: IVec2, offset: Vec2) -> Self {
        debug_assert!(
            offset.x >= 0.0 && offset.x < 1.0 && offset.y >= 0.0 && offset.y < 1.0,
            "offset {:?} is outside [0.0, 1.0) range",
            offset
        );
        Self { tile, offset }
    }

    /// Normalizes offset, rolling overflow into tile.
    pub fn normalize(&mut self) {
        let tile_offset_x = self.offset.x.floor() as i32;
        let tile_offset_y = self.offset.y.floor() as i32;
        self.tile.x += tile_offset_x;
        self.tile.y += tile_offset_y;
        self.offset.x -= tile_offset_x as f32;
        self.offset.y -= tile_offset_y as f32;
    }

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

#[derive(Default)]
pub struct ThingsPlugin;

impl Plugin for ThingsPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<Thing>();
        app.register_type::<WorldPosition>();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalize_positive_overflow() {
        let mut pos = WorldPosition {
            tile: IVec2::new(5, 3),
            offset: Vec2::new(1.5, 2.3),
        };
        pos.normalize();
        assert_eq!(pos.tile, IVec2::new(6, 5));
        assert!((pos.offset.x - 0.5).abs() < f32::EPSILON);
        assert!((pos.offset.y - 0.3).abs() < 0.0001);
    }

    #[test]
    fn test_normalize_negative_overflow() {
        let mut pos = WorldPosition {
            tile: IVec2::new(5, 3),
            offset: Vec2::new(-0.5, -1.2),
        };
        pos.normalize();
        assert_eq!(pos.tile, IVec2::new(4, 1));
        assert!((pos.offset.x - 0.5).abs() < f32::EPSILON);
        assert!((pos.offset.y - 0.8).abs() < 0.0001);
    }

    #[test]
    fn test_normalize_no_overflow() {
        let mut pos = WorldPosition {
            tile: IVec2::new(5, 3),
            offset: Vec2::new(0.5, 0.3),
        };
        pos.normalize();
        assert_eq!(pos.tile, IVec2::new(5, 3));
        assert!((pos.offset.x - 0.5).abs() < f32::EPSILON);
        assert!((pos.offset.y - 0.3).abs() < f32::EPSILON);
    }

    #[test]
    fn test_to_world() {
        let pos = WorldPosition {
            tile: IVec2::new(3, 2),
            offset: Vec2::new(0.5, 0.25),
        };
        let world = pos.to_world();
        assert_eq!(world, Vec2::new(3.5, 2.25));
    }

    #[test]
    #[should_panic(expected = "offset")]
    #[cfg(debug_assertions)]
    fn test_with_offset_out_of_range() {
        WorldPosition::with_offset(IVec2::ZERO, Vec2::new(1.5, 0.0));
    }
}
