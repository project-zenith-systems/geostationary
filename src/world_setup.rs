use bevy::prelude::*;
use tiles::{TileKind, Tilemap};

/// Creates a small hardcoded test map: a rectangular room (12x10) with floor tiles
/// in the interior and wall tiles around the perimeter. Includes a couple of internal
/// walls to test collision from multiple angles.
pub fn create_test_map() -> Tilemap {
    const WIDTH: u32 = 12;
    const HEIGHT: u32 = 10;
    
    // Start with all floor tiles
    let mut tilemap = Tilemap::new(WIDTH, HEIGHT, TileKind::Floor);
    
    // Add perimeter walls
    // Top and bottom walls
    for x in 0..WIDTH {
        tilemap.set(IVec2::new(x as i32, 0), TileKind::Wall);
        tilemap.set(IVec2::new(x as i32, (HEIGHT - 1) as i32), TileKind::Wall);
    }
    
    // Left and right walls
    for y in 0..HEIGHT {
        tilemap.set(IVec2::new(0, y as i32), TileKind::Wall);
        tilemap.set(IVec2::new((WIDTH - 1) as i32, y as i32), TileKind::Wall);
    }
    
    // Add a couple of internal walls for collision testing
    
    // Vertical wall segment on the left side
    tilemap.set(IVec2::new(3, 3), TileKind::Wall);
    tilemap.set(IVec2::new(3, 4), TileKind::Wall);
    tilemap.set(IVec2::new(3, 5), TileKind::Wall);
    
    // Horizontal wall segment on the right side
    tilemap.set(IVec2::new(7, 6), TileKind::Wall);
    tilemap.set(IVec2::new(8, 6), TileKind::Wall);
    tilemap.set(IVec2::new(9, 6), TileKind::Wall);
    
    tilemap
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_map_dimensions() {
        let tilemap = create_test_map();
        assert_eq!(tilemap.width(), 12);
        assert_eq!(tilemap.height(), 10);
    }

    #[test]
    fn test_perimeter_walls() {
        let tilemap = create_test_map();
        
        // Test top wall
        for x in 0..12 {
            assert_eq!(
                tilemap.get(IVec2::new(x, 0)),
                Some(TileKind::Wall),
                "Top wall should be present at x={}", x
            );
        }
        
        // Test bottom wall
        for x in 0..12 {
            assert_eq!(
                tilemap.get(IVec2::new(x, 9)),
                Some(TileKind::Wall),
                "Bottom wall should be present at x={}", x
            );
        }
        
        // Test left wall
        for y in 0..10 {
            assert_eq!(
                tilemap.get(IVec2::new(0, y)),
                Some(TileKind::Wall),
                "Left wall should be present at y={}", y
            );
        }
        
        // Test right wall
        for y in 0..10 {
            assert_eq!(
                tilemap.get(IVec2::new(11, y)),
                Some(TileKind::Wall),
                "Right wall should be present at y={}", y
            );
        }
    }

    #[test]
    fn test_interior_floor() {
        let tilemap = create_test_map();
        
        // Test some interior floor tiles (excluding internal walls)
        assert_eq!(tilemap.get(IVec2::new(1, 1)), Some(TileKind::Floor));
        assert_eq!(tilemap.get(IVec2::new(5, 5)), Some(TileKind::Floor));
        assert_eq!(tilemap.get(IVec2::new(10, 8)), Some(TileKind::Floor));
    }

    #[test]
    fn test_internal_walls() {
        let tilemap = create_test_map();
        
        // Test vertical wall segment
        assert_eq!(tilemap.get(IVec2::new(3, 3)), Some(TileKind::Wall));
        assert_eq!(tilemap.get(IVec2::new(3, 4)), Some(TileKind::Wall));
        assert_eq!(tilemap.get(IVec2::new(3, 5)), Some(TileKind::Wall));
        
        // Test horizontal wall segment
        assert_eq!(tilemap.get(IVec2::new(7, 6)), Some(TileKind::Wall));
        assert_eq!(tilemap.get(IVec2::new(8, 6)), Some(TileKind::Wall));
        assert_eq!(tilemap.get(IVec2::new(9, 6)), Some(TileKind::Wall));
    }

    #[test]
    fn test_internal_wall_collision_angles() {
        let tilemap = create_test_map();
        
        // Verify areas around internal walls for collision testing
        
        // Around vertical wall - should be floor on sides
        assert_eq!(tilemap.get(IVec2::new(2, 4)), Some(TileKind::Floor)); // Left of vertical wall
        assert_eq!(tilemap.get(IVec2::new(4, 4)), Some(TileKind::Floor)); // Right of vertical wall
        
        // Around horizontal wall - should be floor above/below
        assert_eq!(tilemap.get(IVec2::new(8, 5)), Some(TileKind::Floor)); // Above horizontal wall
        assert_eq!(tilemap.get(IVec2::new(8, 7)), Some(TileKind::Floor)); // Below horizontal wall
    }

    #[test]
    fn test_walkability() {
        let tilemap = create_test_map();
        
        // Walls should not be walkable
        assert!(!tilemap.is_walkable(IVec2::new(0, 0))); // Corner wall
        assert!(!tilemap.is_walkable(IVec2::new(3, 4))); // Internal wall
        
        // Floor should be walkable
        assert!(tilemap.is_walkable(IVec2::new(5, 5))); // Interior floor
        assert!(tilemap.is_walkable(IVec2::new(1, 1))); // Corner floor
    }
}
