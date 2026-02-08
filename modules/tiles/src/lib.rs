use bevy::prelude::*;

/// Represents the type of a tile in the game world.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Component)]
pub enum TileKind {
    Floor,
    Wall,
}

impl TileKind {
    /// Returns whether this tile type is walkable.
    pub fn is_walkable(&self) -> bool {
        match self {
            TileKind::Floor => true,
            TileKind::Wall => false,
        }
    }
}

/// A 2D grid-based tilemap storing tile kinds.
/// Uses integer coordinates (i32, i32) for tile positions.
#[derive(Debug, Clone, Resource)]
pub struct Tilemap {
    width: i32,
    height: i32,
    tiles: Vec<TileKind>,
}

impl Tilemap {
    /// Creates a new tilemap with the given dimensions, filled with the specified tile kind.
    pub fn new(width: i32, height: i32, fill: TileKind) -> Self {
        let size = (width * height) as usize;
        Self {
            width,
            height,
            tiles: vec![fill; size],
        }
    }

    /// Returns the width of the tilemap.
    pub fn width(&self) -> i32 {
        self.width
    }

    /// Returns the height of the tilemap.
    pub fn height(&self) -> i32 {
        self.height
    }

    /// Converts (x, y) coordinates to an index in the internal tile array.
    fn coord_to_index(&self, x: i32, y: i32) -> Option<usize> {
        if x >= 0 && x < self.width && y >= 0 && y < self.height {
            Some((y * self.width + x) as usize)
        } else {
            None
        }
    }

    /// Gets the tile kind at the given position.
    /// Returns None if the position is out of bounds.
    pub fn get(&self, x: i32, y: i32) -> Option<TileKind> {
        self.coord_to_index(x, y).map(|idx| self.tiles[idx])
    }

    /// Sets the tile kind at the given position.
    /// Returns true if the position was valid and the tile was set, false otherwise.
    pub fn set(&mut self, x: i32, y: i32, kind: TileKind) -> bool {
        if let Some(idx) = self.coord_to_index(x, y) {
            self.tiles[idx] = kind;
            true
        } else {
            false
        }
    }

    /// Returns whether the position is walkable (within bounds and the tile is walkable).
    pub fn is_walkable(&self, x: i32, y: i32) -> bool {
        self.get(x, y).map_or(false, |kind| kind.is_walkable())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tile_kind_walkability() {
        assert!(TileKind::Floor.is_walkable());
        assert!(!TileKind::Wall.is_walkable());
    }

    #[test]
    fn test_tilemap_creation() {
        let tilemap = Tilemap::new(10, 10, TileKind::Floor);
        assert_eq!(tilemap.width(), 10);
        assert_eq!(tilemap.height(), 10);
    }

    #[test]
    fn test_tilemap_get_set() {
        let mut tilemap = Tilemap::new(5, 5, TileKind::Floor);
        
        // Test getting a tile
        assert_eq!(tilemap.get(0, 0), Some(TileKind::Floor));
        assert_eq!(tilemap.get(4, 4), Some(TileKind::Floor));
        
        // Test setting a tile
        assert!(tilemap.set(2, 2, TileKind::Wall));
        assert_eq!(tilemap.get(2, 2), Some(TileKind::Wall));
        
        // Test out of bounds
        assert_eq!(tilemap.get(-1, 0), None);
        assert_eq!(tilemap.get(0, -1), None);
        assert_eq!(tilemap.get(5, 0), None);
        assert_eq!(tilemap.get(0, 5), None);
        assert!(!tilemap.set(-1, 0, TileKind::Wall));
        assert!(!tilemap.set(10, 10, TileKind::Wall));
    }

    #[test]
    fn test_tilemap_is_walkable() {
        let mut tilemap = Tilemap::new(5, 5, TileKind::Floor);
        
        // Floor tiles should be walkable
        assert!(tilemap.is_walkable(0, 0));
        assert!(tilemap.is_walkable(4, 4));
        
        // Wall tiles should not be walkable
        tilemap.set(2, 2, TileKind::Wall);
        assert!(!tilemap.is_walkable(2, 2));
        
        // Out of bounds should not be walkable
        assert!(!tilemap.is_walkable(-1, 0));
        assert!(!tilemap.is_walkable(5, 0));
        assert!(!tilemap.is_walkable(0, 5));
    }

    #[test]
    fn test_tilemap_coordinates() {
        let mut tilemap = Tilemap::new(3, 3, TileKind::Floor);
        
        // Set specific tiles to create a pattern
        tilemap.set(0, 0, TileKind::Wall);
        tilemap.set(1, 1, TileKind::Wall);
        tilemap.set(2, 2, TileKind::Wall);
        
        // Verify the pattern
        assert_eq!(tilemap.get(0, 0), Some(TileKind::Wall));
        assert_eq!(tilemap.get(1, 0), Some(TileKind::Floor));
        assert_eq!(tilemap.get(2, 0), Some(TileKind::Floor));
        assert_eq!(tilemap.get(0, 1), Some(TileKind::Floor));
        assert_eq!(tilemap.get(1, 1), Some(TileKind::Wall));
        assert_eq!(tilemap.get(2, 1), Some(TileKind::Floor));
        assert_eq!(tilemap.get(0, 2), Some(TileKind::Floor));
        assert_eq!(tilemap.get(1, 2), Some(TileKind::Floor));
        assert_eq!(tilemap.get(2, 2), Some(TileKind::Wall));
    }
}
