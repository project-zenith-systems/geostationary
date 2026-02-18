use bevy::prelude::*;
use tiles::{TileKind, Tilemap};

/// Constant for deriving pressure from moles.
/// pressure = moles * PRESSURE_CONSTANT
/// This simplifies the ideal gas law by assuming fixed temperature and unit cell volume.
const PRESSURE_CONSTANT: f32 = 1.0;
/// Fraction of the pressure difference that can be equalized between two neighboring cells
/// over a full simulation step.
///
/// A value of `0.25` means that, at most, 25% of the pressure difference is resolved per
/// step. Higher values make gas spread faster but can cause oscillations or numerical
/// instability if too large.
const DIFFUSION_RATE: f32 = 0.25;
/// Maximum fraction of the per-step diffusion (`DIFFUSION_RATE`) that is allowed to be
/// applied in a single diffusion substep.
///
/// This is used by the sub-stepping logic to break a potentially large diffusion update
/// into several smaller, stable substeps. It must remain **strictly less** than
/// `DIFFUSION_RATE` so that the loop performing substeps can make progress and terminate
/// correctly. If you change `DIFFUSION_RATE`, adjust this value accordingly while
/// preserving the invariant `MAX_DIFFUSION_FACTOR_PER_STEP < DIFFUSION_RATE`.
const MAX_DIFFUSION_FACTOR_PER_STEP: f32 = 0.24;

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

/// Represents a proposed gas flow between two cells during a diffusion substep.
#[derive(Debug, Clone, Copy)]
struct ProposedFlow {
    from: usize,
    to: usize,
    amount: f32,
}

/// A grid-based gas simulation that tracks moles per cell and derives pressure.
/// Uses Bevy types for integration but keeps the simulation logic independent
/// of ECS systems for easier testing.
#[derive(Debug, Clone, Resource, Reflect)]
#[reflect(Debug, Resource)]
pub struct GasGrid {
    width: u32,
    height: u32,
    cells: Vec<GasCell>,
    passable: Vec<bool>,
    // Scratch buffers reused across substeps to avoid per-frame heap allocations
    #[reflect(ignore)]
    scratch_flows: Vec<ProposedFlow>,
    #[reflect(ignore)]
    scratch_outgoing: Vec<f32>,
    #[reflect(ignore)]
    scratch_source_scale: Vec<f32>,
    #[reflect(ignore)]
    scratch_delta: Vec<f32>,
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
            scratch_flows: Vec::new(),
            scratch_outgoing: vec![0.0; size],
            scratch_source_scale: vec![1.0; size],
            scratch_delta: vec![0.0; size],
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
    /// Clamps negative moles to 0.0 since negative gas quantities are physically invalid.
    pub fn set_moles(&mut self, pos: IVec2, moles: f32) -> bool {
        if let Some(idx) = self.coord_to_index(pos) {
            self.cells[idx].moles = moles.max(0.0);
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

    /// Performs one diffusion step using a Jacobi-style update.
    ///
    /// For each pair of passable cardinal neighbors, this computes a proposed flow
    /// proportional to their pressure difference and applies all flows simultaneously.
    /// Outgoing flow from each cell is clamped so no cell can go negative.
    pub fn step(&mut self, dt: f32) {
        if dt <= 0.0 || !dt.is_finite() {
            return;
        }

        let cell_count = self.cells.len();
        if cell_count == 0 {
            return;
        }

        // Explicit diffusion is only stable when DIFFUSION_RATE * dt stays small.
        // Split large dt into smaller sub-steps to avoid odd/even checkerboard oscillation.
        let max_substep_dt = MAX_DIFFUSION_FACTOR_PER_STEP / DIFFUSION_RATE;
        let substeps = (dt / max_substep_dt).ceil().max(1.0) as u32;
        let substep_dt = dt / substeps as f32;

        for _ in 0..substeps {
            self.step_substep(substep_dt);
        }
    }

    fn step_substep(&mut self, dt: f32) {
        let width = self.width as usize;
        let height = self.height as usize;

        // Reuse scratch buffers to avoid per-substep heap allocations
        self.scratch_flows.clear();
        for val in self.scratch_outgoing.iter_mut() {
            *val = 0.0;
        }

        for y in 0..height {
            for x in 0..width {
                let idx = y * width + x;
                if !self.passable[idx] {
                    continue;
                }

                let moles_here = self.cells[idx].moles;

                if x + 1 < width {
                    let n_idx = y * width + (x + 1);
                    if self.passable[n_idx] {
                        let diff = moles_here - self.cells[n_idx].moles;
                        if diff > 0.0 {
                            let amount = diff * DIFFUSION_RATE * dt;
                            self.scratch_flows.push(ProposedFlow {
                                from: idx,
                                to: n_idx,
                                amount,
                            });
                            self.scratch_outgoing[idx] += amount;
                        } else if diff < 0.0 {
                            let amount = (-diff) * DIFFUSION_RATE * dt;
                            self.scratch_flows.push(ProposedFlow {
                                from: n_idx,
                                to: idx,
                                amount,
                            });
                            self.scratch_outgoing[n_idx] += amount;
                        }
                    }
                }

                if y + 1 < height {
                    let n_idx = (y + 1) * width + x;
                    if self.passable[n_idx] {
                        let diff = moles_here - self.cells[n_idx].moles;
                        if diff > 0.0 {
                            let amount = diff * DIFFUSION_RATE * dt;
                            self.scratch_flows.push(ProposedFlow {
                                from: idx,
                                to: n_idx,
                                amount,
                            });
                            self.scratch_outgoing[idx] += amount;
                        } else if diff < 0.0 {
                            let amount = (-diff) * DIFFUSION_RATE * dt;
                            self.scratch_flows.push(ProposedFlow {
                                from: n_idx,
                                to: idx,
                                amount,
                            });
                            self.scratch_outgoing[n_idx] += amount;
                        }
                    }
                }
            }
        }

        for (idx, &outgoing) in self.scratch_outgoing.iter().enumerate() {
            if outgoing > 0.0 {
                let available = self.cells[idx].moles.max(0.0);
                self.scratch_source_scale[idx] = (available / outgoing).clamp(0.0, 1.0);
            } else {
                self.scratch_source_scale[idx] = 1.0;
            }
        }

        for val in self.scratch_delta.iter_mut() {
            *val = 0.0;
        }

        for flow in &self.scratch_flows {
            let actual = flow.amount * self.scratch_source_scale[flow.from];
            if actual <= 0.0 || !actual.is_finite() {
                continue;
            }

            self.scratch_delta[flow.from] -= actual;
            self.scratch_delta[flow.to] += actual;
        }

        for (idx, cell) in self.cells.iter_mut().enumerate() {
            let next = cell.moles + self.scratch_delta[idx];
            cell.moles = if next.is_finite() { next.max(0.0) } else { 0.0 };
        }
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
    fn test_negative_moles_clamped() {
        let mut grid = GasGrid::new(3, 3);

        // Negative moles should be clamped to 0.0
        assert!(grid.set_moles(IVec2::new(1, 1), -5.0));
        assert_eq!(grid.pressure_at(IVec2::new(1, 1)), Some(0.0));

        // Set positive, then negative
        grid.set_moles(IVec2::new(0, 0), 10.0);
        assert_eq!(grid.pressure_at(IVec2::new(0, 0)), Some(10.0));
        grid.set_moles(IVec2::new(0, 0), -3.0);
        assert_eq!(grid.pressure_at(IVec2::new(0, 0)), Some(0.0));
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

        // Change tiles to walls (floor -> wall transition: moles preserved)
        tilemap.set(IVec2::new(1, 1), TileKind::Wall);
        grid.sync_walls(&tilemap);

        // Check passability updated
        assert!(!grid.passable[4]); // (1, 1) now impassable
        assert!(grid.passable[0]); // (0, 0) still passable

        // Moles should be preserved even when impassable
        assert_eq!(grid.pressure_at(IVec2::new(1, 1)), Some(5.0));
        assert_eq!(grid.pressure_at(IVec2::new(0, 0)), Some(3.0));

        // Total moles should still count the sealed cell
        assert_eq!(grid.total_moles(), 8.0);

        // Change wall back to floor (wall -> floor transition)
        tilemap.set(IVec2::new(1, 1), TileKind::Floor);
        grid.sync_walls(&tilemap);
        assert!(grid.passable[4]); // (1, 1) passable again
        assert_eq!(grid.pressure_at(IVec2::new(1, 1)), Some(5.0)); // moles still preserved

        // Test wall removal with explicit vacuum creation
        // Set a cell to wall, then remove it and set to 0.0 moles (simulating wall_toggle_input)
        tilemap.set(IVec2::new(2, 2), TileKind::Wall);
        grid.sync_walls(&tilemap);
        assert!(!grid.passable[8]); // (2, 2) is wall, impassable

        // Remove wall and set to vacuum (what wall_toggle_input does)
        tilemap.set(IVec2::new(2, 2), TileKind::Floor);
        grid.set_moles(IVec2::new(2, 2), 0.0);
        grid.sync_walls(&tilemap);

        assert!(grid.passable[8]); // (2, 2) now passable
        assert_eq!(grid.pressure_at(IVec2::new(2, 2)), Some(0.0)); // vacuum (0.0 moles)
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

        // Run several step iterations
        for _ in 0..10 {
            grid.step(0.1);
        }

        // Total moles should remain constant (within epsilon for floating point)
        let final_total = grid.total_moles();
        assert!((final_total - initial_total).abs() < 1e-4);
    }

    #[test]
    fn test_step_reduces_pressure_discontinuity() {
        let mut grid = GasGrid::new(2, 1);
        let tilemap = Tilemap::new(2, 1, TileKind::Floor);
        grid.sync_walls(&tilemap);

        grid.set_moles(IVec2::new(0, 0), 10.0);
        grid.set_moles(IVec2::new(1, 0), 0.0);

        let before_diff = (grid.pressure_at(IVec2::new(0, 0)).unwrap()
            - grid.pressure_at(IVec2::new(1, 0)).unwrap())
        .abs();

        grid.step(1.0);

        let after_diff = (grid.pressure_at(IVec2::new(0, 0)).unwrap()
            - grid.pressure_at(IVec2::new(1, 0)).unwrap())
        .abs();

        assert!(after_diff < before_diff);
    }

    #[test]
    fn test_diffusion_spike_criteria_convergence_and_conservation() {
        let mut grid = GasGrid::new(12, 10);
        let mut tilemap = Tilemap::new(12, 10, TileKind::Floor);

        // Build two chambers separated by a vertical wall at x = 5.
        for y in 0..10 {
            tilemap.set(IVec2::new(5, y), TileKind::Wall);
        }

        grid.sync_walls(&tilemap);

        // Left chamber pressurized to 1.0 atm, right chamber vacuum.
        for y in 0..10 {
            for x in 0..12 {
                let pos = IVec2::new(x, y);
                if !tilemap.is_walkable(pos) {
                    continue;
                }

                if x < 5 {
                    grid.set_moles(pos, 1.0);
                } else {
                    grid.set_moles(pos, 0.0);
                }
            }
        }

        let initial_total = grid.total_moles();

        // Closed chambers should remain internally stable/non-negative/finite.
        for _ in 0..200 {
            grid.step(2.0);
        }

        for cell in &grid.cells {
            assert!(cell.moles.is_finite());
            assert!(cell.moles >= 0.0);
        }

        // Remove a contiguous wall segment to connect chambers.
        for y in 0..10 {
            tilemap.set(IVec2::new(5, y), TileKind::Floor);
        }
        grid.sync_walls(&tilemap);

        for _ in 0..200 {
            grid.step(2.0);
        }

        // Re-convergence: passable cells should be near-uniform.
        let mut min_p = f32::MAX;
        let mut max_p = f32::MIN;

        for y in 0..10 {
            for x in 0..12 {
                let pos = IVec2::new(x, y);
                if !tilemap.is_walkable(pos) {
                    continue;
                }

                let p = grid.pressure_at(pos).unwrap();
                min_p = min_p.min(p);
                max_p = max_p.max(p);
            }
        }

        assert!(
            (max_p - min_p) < 0.01,
            "Expected convergence with max-min < 0.01, got {}",
            max_p - min_p
        );

        let final_total = grid.total_moles();
        assert!(
            (final_total - initial_total).abs() < 1e-4,
            "Expected mass conservation, initial={}, final={}",
            initial_total,
            final_total
        );

        for cell in &grid.cells {
            assert!(cell.moles.is_finite());
            assert!(cell.moles >= 0.0);
        }
    }
}
