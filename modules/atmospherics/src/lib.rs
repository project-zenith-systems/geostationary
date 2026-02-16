use bevy::prelude::*;
use tiles::{TileKind, Tilemap};

/// Gas cell storing the amount of gas in moles
#[derive(Debug, Clone, Copy)]
pub struct GasCell {
    pub moles: f32,
}

impl Default for GasCell {
    fn default() -> Self {
        Self { moles: 0.0 }
    }
}

/// Pure Rust struct for gas simulation with no Bevy dependency in core logic
#[derive(Debug, Clone, Resource)]
pub struct GasGrid {
    width: u32,
    height: u32,
    cells: Vec<GasCell>,
    passable: Vec<bool>, // true = floor (passable), false = wall (impassable)
}

impl GasGrid {
    /// Create a new GasGrid with the given dimensions
    pub fn new(width: u32, height: u32) -> Self {
        let size = (width * height) as usize;
        Self {
            width,
            height,
            cells: vec![GasCell::default(); size],
            passable: vec![false; size],
        }
    }

    fn coord_to_index(&self, pos: IVec2) -> Option<usize> {
        if pos.x >= 0 && pos.x < self.width as i32 && pos.y >= 0 && pos.y < self.height as i32 {
            Some((pos.y * self.width as i32 + pos.x) as usize)
        } else {
            None
        }
    }

    /// Sync wall passability from the tilemap.
    /// When a cell transitions from wall to floor, it starts at 0.0 moles (vacuum).
    /// When a cell transitions from floor to wall, it becomes impassable but preserves its moles.
    pub fn sync_walls(&mut self, tilemap: &Tilemap) {
        for (pos, kind) in tilemap.iter() {
            if let Some(idx) = self.coord_to_index(pos) {
                let was_passable = self.passable[idx];
                let is_passable = kind == TileKind::Floor;
                
                self.passable[idx] = is_passable;
                
                // If transitioning from wall to floor, reset moles to 0.0 (vacuum)
                if !was_passable && is_passable {
                    self.cells[idx].moles = 0.0;
                }
                // If transitioning from floor to wall, preserve existing moles
                // (they remain stored in the sealed cell)
            }
        }
    }

    /// Get pressure at a given position
    pub fn pressure_at(&self, pos: IVec2) -> Option<f32> {
        self.coord_to_index(pos).map(|idx| {
            // Simplified pressure calculation: pressure = moles * constant
            // For now, we use a simple 1:1 ratio
            self.cells[idx].moles
        })
    }

    /// Set moles at a given position
    pub fn set_moles(&mut self, pos: IVec2, moles: f32) {
        if let Some(idx) = self.coord_to_index(pos) {
            self.cells[idx].moles = moles;
        }
    }

    /// Calculate total moles across all cells (for conservation checks)
    pub fn total_moles(&self) -> f32 {
        self.cells.iter().map(|cell| cell.moles).sum()
    }

    /// Check if a cell is passable
    pub fn is_passable(&self, pos: IVec2) -> bool {
        self.coord_to_index(pos)
            .map(|idx| self.passable[idx])
            .unwrap_or(false)
    }
}

pub struct AtmosphericsPlugin;

impl Plugin for AtmosphericsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (wall_sync_system, wall_toggle_input));
    }
}

/// System that syncs wall passability when the tilemap changes
fn wall_sync_system(
    tilemap: Option<Res<Tilemap>>,
    mut gas_grid: Option<ResMut<GasGrid>>,
) {
    let Some(tilemap) = tilemap else { return };
    let Some(ref mut gas_grid) = gas_grid else { return };
    
    // Only sync when tilemap has changed
    if tilemap.is_changed() {
        gas_grid.sync_walls(&tilemap);
    }
}

/// System that toggles walls on keypress for debugging/testing
fn wall_toggle_input(
    keyboard: Res<ButtonInput<KeyCode>>,
    mut tilemap: Option<ResMut<Tilemap>>,
    _camera_query: Query<&Transform, With<Camera>>,
) {
    let Some(ref mut tilemap) = tilemap else { return };
    
    // Toggle wall on 'T' key press
    if keyboard.just_pressed(KeyCode::KeyT) {
        // For now, toggle the tile at (5, 5) as a simple test
        // In a real implementation, this would raycast from camera to find the tile
        let test_pos = IVec2::new(5, 5);
        
        if let Some(current_kind) = tilemap.get(test_pos) {
            let new_kind = match current_kind {
                TileKind::Floor => TileKind::Wall,
                TileKind::Wall => TileKind::Floor,
            };
            tilemap.set(test_pos, new_kind);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gas_grid_creation() {
        let grid = GasGrid::new(10, 10);
        assert_eq!(grid.width, 10);
        assert_eq!(grid.height, 10);
        assert_eq!(grid.cells.len(), 100);
        assert_eq!(grid.passable.len(), 100);
    }

    #[test]
    fn test_sync_walls_after_wall_removal() {
        let mut grid = GasGrid::new(5, 5);
        let mut tilemap = Tilemap::new(5, 5, TileKind::Wall);
        
        // Set some moles in a cell
        grid.set_moles(IVec2::new(2, 2), 100.0);
        
        // Sync with wall tilemap - cell should be impassable
        grid.sync_walls(&tilemap);
        assert!(!grid.is_passable(IVec2::new(2, 2)));
        
        // Remove the wall (change to floor)
        tilemap.set(IVec2::new(2, 2), TileKind::Floor);
        grid.sync_walls(&tilemap);
        
        // Cell should now be passable with 0.0 moles (vacuum)
        assert!(grid.is_passable(IVec2::new(2, 2)));
        assert_eq!(grid.pressure_at(IVec2::new(2, 2)), Some(0.0));
    }

    #[test]
    fn test_sync_walls_after_wall_addition() {
        let mut grid = GasGrid::new(5, 5);
        let mut tilemap = Tilemap::new(5, 5, TileKind::Floor);
        
        // Sync with floor tilemap - cell should be passable
        grid.sync_walls(&tilemap);
        assert!(grid.is_passable(IVec2::new(2, 2)));
        
        // Set some moles in the cell
        grid.set_moles(IVec2::new(2, 2), 100.0);
        assert_eq!(grid.pressure_at(IVec2::new(2, 2)), Some(100.0));
        
        // Add a wall (change to wall)
        tilemap.set(IVec2::new(2, 2), TileKind::Wall);
        grid.sync_walls(&tilemap);
        
        // Cell should now be impassable but preserve its moles
        assert!(!grid.is_passable(IVec2::new(2, 2)));
        assert_eq!(grid.pressure_at(IVec2::new(2, 2)), Some(100.0));
        
        // Total moles should still include the sealed cell
        assert_eq!(grid.total_moles(), 100.0);
    }

    #[test]
    fn test_total_moles_conservation() {
        let mut grid = GasGrid::new(3, 3);
        
        grid.set_moles(IVec2::new(0, 0), 10.0);
        grid.set_moles(IVec2::new(1, 1), 20.0);
        grid.set_moles(IVec2::new(2, 2), 30.0);
        
        assert_eq!(grid.total_moles(), 60.0);
    }
}
