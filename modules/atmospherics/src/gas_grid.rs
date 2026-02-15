use bevy::prelude::*;
use tiles::{TileKind, Tilemap};

/// Constant for deriving pressure from moles.
/// pressure = moles * PRESSURE_CONSTANT
/// This simplifies the ideal gas law by assuming fixed temperature and unit cell volume.
const PRESSURE_CONSTANT: f32 = 1.0;

/// Represents a single cell in the gas grid.
#[derive(Debug, Clone, Copy, PartialEq, Reflect)]
#[reflect(Debug, PartialEq)]
pub struct GasCell {
    pub moles: f32,
}

impl Default for GasCell {
    fn default() -> Self {
        Self { moles: 0.0 }
    }
}

/// A grid-based gas simulation that tracks moles per cell and derives pressure.
/// Pure logic with no Bevy dependency in the core algorithm.
#[derive(Debug, Clone, Resource, Reflect)]
#[reflect(Debug, Resource)]
pub struct GasGrid {
    width: u32,
    height: u32,
    cells: Vec<GasCell>,
    passable: Vec<bool>,
}

impl GasGrid {
    /// Creates a new gas grid with the given dimensions.
    /// All cells are initialized with 0 moles and marked as passable.
    pub fn new(width: u32, height: u32) -> Self {
        let size = (width * height) as usize;
        Self {
            width,
            height,
            cells: vec![GasCell::default(); size],
            passable: vec![true; size],
        }
    }

    /// Converts a 2D position to a 1D index in the cells/passable arrays.
    /// Returns None if the position is out of bounds.
    fn coord_to_index(&self, pos: IVec2) -> Option<usize> {
        if pos.x >= 0 && pos.x < self.width as i32 && pos.y >= 0 && pos.y < self.height as i32 {
            Some((pos.y * self.width as i32 + pos.x) as usize)
        } else {
            None
        }
    }

    /// Updates the passability mask based on the current tilemap.
    /// Floor tiles are passable (allow gas flow), walls are not.
    /// When a cell becomes passable, its moles remain at their current value.
    /// When a cell becomes impassable, its moles are preserved for conservation.
    pub fn sync_walls(&mut self, tilemap: &Tilemap) {
        for y in 0..self.height {
            for x in 0..self.width {
                let pos = IVec2::new(x as i32, y as i32);
                if let Some(idx) = self.coord_to_index(pos) {
                    if let Some(tile_kind) = tilemap.get(pos) {
                        self.passable[idx] = matches!(tile_kind, TileKind::Floor);
                    } else {
                        // Out of tilemap bounds -> treat as impassable
                        self.passable[idx] = false;
                    }
                }
            }
        }
    }

    /// Returns the pressure at the given position.
    /// Pressure is derived from moles using: pressure = moles * PRESSURE_CONSTANT
    /// Returns None if the position is out of bounds.
    pub fn pressure_at(&self, pos: IVec2) -> Option<f32> {
        self.coord_to_index(pos)
            .map(|idx| self.cells[idx].moles * PRESSURE_CONSTANT)
    }

    /// Sets the moles at the given position.
    /// Returns true if successful, false if the position is out of bounds.
    pub fn set_moles(&mut self, pos: IVec2, moles: f32) -> bool {
        if let Some(idx) = self.coord_to_index(pos) {
            self.cells[idx].moles = moles;
            true
        } else {
            false
        }
    }

    /// Returns the total number of moles across all cells in the grid.
    /// This should remain constant (within floating-point epsilon) during diffusion
    /// to demonstrate conservation of mass.
    pub fn total_moles(&self) -> f32 {
        self.cells.iter().map(|cell| cell.moles).sum()
    }

    /// Performs one diffusion step.
    /// This is currently a no-op stub - diffusion algorithm will be implemented later.
    pub fn step(&mut self, _dt: f32) {
        // No-op stub - diffusion comes later
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_coord_to_index_mapping() {
        let grid = GasGrid::new(5, 4);

        // Valid coordinates
        assert_eq!(grid.coord_to_index(IVec2::new(0, 0)), Some(0));
        assert_eq!(grid.coord_to_index(IVec2::new(4, 0)), Some(4));
        assert_eq!(grid.coord_to_index(IVec2::new(0, 1)), Some(5));
        assert_eq!(grid.coord_to_index(IVec2::new(4, 3)), Some(19));

        // Out of bounds
        assert_eq!(grid.coord_to_index(IVec2::new(-1, 0)), None);
        assert_eq!(grid.coord_to_index(IVec2::new(0, -1)), None);
        assert_eq!(grid.coord_to_index(IVec2::new(5, 0)), None);
        assert_eq!(grid.coord_to_index(IVec2::new(0, 4)), None);
    }

    #[test]
    fn test_bounds_checks() {
        let mut grid = GasGrid::new(3, 3);

        // Valid set/get
        assert!(grid.set_moles(IVec2::new(1, 1), 10.0));
        assert_eq!(grid.pressure_at(IVec2::new(1, 1)), Some(10.0));

        // Out of bounds
        assert!(!grid.set_moles(IVec2::new(-1, 0), 5.0));
        assert!(!grid.set_moles(IVec2::new(3, 0), 5.0));
        assert_eq!(grid.pressure_at(IVec2::new(-1, 0)), None);
        assert_eq!(grid.pressure_at(IVec2::new(3, 3)), None);
    }

    #[test]
    fn test_sync_walls_floor_passable() {
        let mut grid = GasGrid::new(5, 5);
        let tilemap = Tilemap::new(5, 5, TileKind::Floor);

        grid.sync_walls(&tilemap);

        // All floor tiles should be passable
        for i in 0..25 {
            assert!(grid.passable[i], "Cell {} should be passable", i);
        }
    }

    #[test]
    fn test_sync_walls_wall_impassable() {
        let mut grid = GasGrid::new(5, 5);
        let tilemap = Tilemap::new(5, 5, TileKind::Wall);

        grid.sync_walls(&tilemap);

        // All wall tiles should be impassable
        for i in 0..25 {
            assert!(!grid.passable[i], "Cell {} should be impassable", i);
        }
    }

    #[test]
    fn test_sync_walls_mixed_tiles() {
        let mut grid = GasGrid::new(3, 3);
        let mut tilemap = Tilemap::new(3, 3, TileKind::Floor);

        // Set some walls
        tilemap.set(IVec2::new(1, 1), TileKind::Wall);
        tilemap.set(IVec2::new(0, 2), TileKind::Wall);

        grid.sync_walls(&tilemap);

        // Check passability matches tilemap
        assert!(grid.passable[0]); // (0, 0) is floor
        assert!(grid.passable[1]); // (1, 0) is floor
        assert!(grid.passable[2]); // (2, 0) is floor
        assert!(grid.passable[3]); // (0, 1) is floor
        assert!(!grid.passable[4]); // (1, 1) is wall
        assert!(grid.passable[5]); // (2, 1) is floor
        assert!(!grid.passable[6]); // (0, 2) is wall
        assert!(grid.passable[7]); // (1, 2) is floor
        assert!(grid.passable[8]); // (2, 2) is floor
    }

    #[test]
    fn test_sync_walls_transitions() {
        let mut grid = GasGrid::new(3, 3);
        let mut tilemap = Tilemap::new(3, 3, TileKind::Floor);

        // Set some moles in cells
        grid.set_moles(IVec2::new(1, 1), 5.0);
        grid.set_moles(IVec2::new(0, 0), 3.0);

        // All should be passable initially
        grid.sync_walls(&tilemap);
        assert!(grid.passable[4]); // (1, 1)
        assert!(grid.passable[0]); // (0, 0)

        // Change tiles to walls
        tilemap.set(IVec2::new(1, 1), TileKind::Wall);
        grid.sync_walls(&tilemap);

        // Check passability updated
        assert!(!grid.passable[4]); // (1, 1) now impassable
        assert!(grid.passable[0]); // (0, 0) still passable

        // Moles should be preserved even when impassable
        assert_eq!(grid.pressure_at(IVec2::new(1, 1)), Some(5.0));
        assert_eq!(grid.pressure_at(IVec2::new(0, 0)), Some(3.0));

        // Change wall back to floor
        tilemap.set(IVec2::new(1, 1), TileKind::Floor);
        grid.sync_walls(&tilemap);
        assert!(grid.passable[4]); // (1, 1) passable again
        assert_eq!(grid.pressure_at(IVec2::new(1, 1)), Some(5.0)); // moles still preserved
    }

    #[test]
    fn test_pressure_at_formula() {
        let mut grid = GasGrid::new(3, 3);

        grid.set_moles(IVec2::new(0, 0), 0.0);
        grid.set_moles(IVec2::new(1, 0), 1.0);
        grid.set_moles(IVec2::new(2, 0), 2.5);
        grid.set_moles(IVec2::new(0, 1), 10.0);

        // pressure = moles * PRESSURE_CONSTANT (1.0)
        assert_eq!(grid.pressure_at(IVec2::new(0, 0)), Some(0.0));
        assert_eq!(grid.pressure_at(IVec2::new(1, 0)), Some(1.0));
        assert_eq!(grid.pressure_at(IVec2::new(2, 0)), Some(2.5));
        assert_eq!(grid.pressure_at(IVec2::new(0, 1)), Some(10.0));
    }

    #[test]
    fn test_total_moles_conservation() {
        let mut grid = GasGrid::new(4, 4);

        // Initially zero
        assert_eq!(grid.total_moles(), 0.0);

        // Add some moles
        grid.set_moles(IVec2::new(0, 0), 5.0);
        grid.set_moles(IVec2::new(1, 1), 3.0);
        grid.set_moles(IVec2::new(2, 2), 2.0);
        assert_eq!(grid.total_moles(), 10.0);

        // Modify moles
        grid.set_moles(IVec2::new(0, 0), 1.0);
        assert_eq!(grid.total_moles(), 6.0);

        // Add more
        grid.set_moles(IVec2::new(3, 3), 4.0);
        assert_eq!(grid.total_moles(), 10.0);
    }

    #[test]
    fn test_total_moles_with_step() {
        let mut grid = GasGrid::new(3, 3);
        let tilemap = Tilemap::new(3, 3, TileKind::Floor);

        grid.sync_walls(&tilemap);

        // Set initial moles
        grid.set_moles(IVec2::new(0, 0), 10.0);
        grid.set_moles(IVec2::new(1, 1), 5.0);
        grid.set_moles(IVec2::new(2, 2), 3.0);

        let initial_total = grid.total_moles();
        assert_eq!(initial_total, 18.0);

        // Run several step iterations (currently no-op, but tests the API)
        for _ in 0..10 {
            grid.step(0.1);
        }

        // Total moles should remain constant (within epsilon for floating point)
        let final_total = grid.total_moles();
        assert!((final_total - initial_total).abs() < 1e-6);
    }

    #[test]
    fn test_step_is_noop() {
        let mut grid = GasGrid::new(3, 3);
        grid.set_moles(IVec2::new(0, 0), 10.0);
        grid.set_moles(IVec2::new(1, 1), 5.0);

        let initial_moles_00 = grid.pressure_at(IVec2::new(0, 0)).unwrap();
        let initial_moles_11 = grid.pressure_at(IVec2::new(1, 1)).unwrap();

        // Step should not change anything (it's a stub)
        grid.step(0.1);

        assert_eq!(grid.pressure_at(IVec2::new(0, 0)).unwrap(), initial_moles_00);
        assert_eq!(grid.pressure_at(IVec2::new(1, 1)).unwrap(), initial_moles_11);
    }
}
