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
/// into several smaller, numerically stable substeps. It must remain **strictly less**
/// than `DIFFUSION_RATE` so that the per-substep diffusion factor
/// (`DIFFUSION_RATE * substep_dt`) stays bounded by `MAX_DIFFUSION_FACTOR_PER_STEP`,
/// which is a stability constraint for explicit diffusion updates. If you change
/// `DIFFUSION_RATE`, adjust this value accordingly while preserving the invariant
/// `0.0 < MAX_DIFFUSION_FACTOR_PER_STEP < DIFFUSION_RATE`.
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
    /// Tracks the moles values from the last broadcast snapshot or delta.
    /// Used by the server to compute incremental deltas for replication.
    #[reflect(ignore)]
    pub last_broadcast_moles: Vec<f32>,
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
            last_broadcast_moles: vec![0.0; size],
            scratch_flows: Vec::new(),
            scratch_outgoing: vec![0.0; size],
            scratch_source_scale: vec![1.0; size],
            scratch_delta: vec![0.0; size],
        }
    }

    /// Returns the width of the gas grid.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Returns the height of the gas grid.
    pub fn height(&self) -> u32 {
        self.height
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
    /// When a cell transitions to impassable (wall), its moles are zeroed —
    /// walls always hold 0.0 moles.
    pub fn sync_walls(&mut self, tilemap: &Tilemap) {
        for y in 0..self.height {
            for x in 0..self.width {
                let pos = IVec2::new(x as i32, y as i32);
                if let Some(idx) = self.coord_to_index(pos) {
                    let new_passable = if let Some(tile_kind) = tilemap.get(pos) {
                        matches!(tile_kind, TileKind::Floor)
                    } else {
                        // Out of tilemap bounds -> treat as impassable
                        false
                    };
                    if !new_passable && self.passable[idx] {
                        // Transitioning floor → wall: zero moles
                        self.cells[idx].moles = 0.0;
                    }
                    self.passable[idx] = new_passable;
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

    /// Computes the 2D pressure gradient at the given position using central differences.
    ///
    /// For each axis the gradient is `(pressure_positive − pressure_negative) / 2.0`.
    /// If a neighbouring cell is out of bounds or impassable, the center cell's pressure
    /// is used in its place, contributing zero gradient for that direction.
    ///
    /// Returns the gradient as a `Vec2` in (x, z) tile-grid space.  Callers that need a
    /// world-space force vector should map x→X and y→Z (the horizontal plane).
    pub fn pressure_gradient_at(&self, pos: IVec2) -> Vec2 {
        let center = self.pressure_at(pos).unwrap_or(0.0);

        let p_x_pos = self
            .passable_pressure_at(IVec2::new(pos.x + 1, pos.y))
            .unwrap_or(center);
        let p_x_neg = self
            .passable_pressure_at(IVec2::new(pos.x - 1, pos.y))
            .unwrap_or(center);
        let p_y_pos = self
            .passable_pressure_at(IVec2::new(pos.x, pos.y + 1))
            .unwrap_or(center);
        let p_y_neg = self
            .passable_pressure_at(IVec2::new(pos.x, pos.y - 1))
            .unwrap_or(center);

        Vec2::new(
            (p_x_pos - p_x_neg) / 2.0,
            (p_y_pos - p_y_neg) / 2.0,
        )
    }

    /// Returns the pressure at `pos` only if the cell is in-bounds and passable; otherwise `None`.
    fn passable_pressure_at(&self, pos: IVec2) -> Option<f32> {
        let idx = self.coord_to_index(pos)?;
        if self.passable[idx] {
            Some(self.cells[idx].moles * PRESSURE_CONSTANT)
        } else {
            None
        }
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

    /// Returns the moles of every cell as a flat `Vec<f32>` in row-major order.
    /// Used to serialize the gas grid for network transmission.
    pub fn moles_vec(&self) -> Vec<f32> {
        self.cells.iter().map(|cell| cell.moles).collect()
    }

    /// Returns the passability mask as a flat `Vec<bool>` in row-major order.
    /// Used to serialize the gas grid for network transmission.
    pub fn passable_vec(&self) -> Vec<bool> {
        self.passable.clone()
    }

    /// Computes incremental changes since the last broadcast.
    /// Returns `(cell_index, new_moles)` pairs for every cell whose moles differ
    /// from [`last_broadcast_moles`] by more than `epsilon`.
    ///
    /// Cell indices are encoded as `u16`; cells whose flat index exceeds
    /// [`u16::MAX`] are silently skipped.
    pub fn compute_delta_changes(&self, epsilon: f32) -> Vec<(u16, f32)> {
        self.cells
            .iter()
            .zip(self.last_broadcast_moles.iter())
            .enumerate()
            .filter(|(idx, (cell, last))| {
                *idx <= u16::MAX as usize && (cell.moles - *last).abs() > epsilon
            })
            .map(|(idx, (cell, _))| (idx as u16, cell.moles))
            .collect()
    }

    /// Updates [`last_broadcast_moles`] to match the current cell moles.
    /// Call after sending a full snapshot or delta to reset the change baseline.
    pub fn update_last_broadcast_moles(&mut self) {
        for (last, cell) in self.last_broadcast_moles.iter_mut().zip(self.cells.iter()) {
            *last = cell.moles;
        }
    }

    /// Applies delta changes received from the server.
    /// Each entry is `(cell_index, new_moles_value)`.
    /// Out-of-bounds indices are silently ignored.
    pub fn apply_delta_changes(&mut self, changes: &[(u16, f32)]) {
        for &(idx, moles) in changes {
            if let Some(cell) = self.cells.get_mut(idx as usize) {
                cell.moles = moles.max(0.0);
            }
        }
    }

    /// Reconstructs a [`GasGrid`] from dimensions, a flat moles slice produced by
    /// [`GasGrid::moles_vec`], and a passability mask produced by [`GasGrid::passable_vec`].
    ///
    /// Returns an error if the length of `gas_moles` or `passable` does not match
    /// `width * height`.
    pub fn from_moles_vec(
        width: u32,
        height: u32,
        gas_moles: Vec<f32>,
        passable: Vec<bool>,
    ) -> Result<Self, String> {
        let expected = width
            .checked_mul(height)
            .and_then(|n| usize::try_from(n).ok())
            .ok_or_else(|| format!("GasGrid dimensions {width}×{height} overflow"))?;
        if gas_moles.len() != expected {
            return Err(format!(
                "gas_moles length mismatch: expected {expected}, got {}",
                gas_moles.len()
            ));
        }
        if passable.len() != expected {
            return Err(format!(
                "passable length mismatch: expected {expected}, got {}",
                passable.len()
            ));
        }
        let size = expected;
        let cells = gas_moles
            .iter()
            .map(|&moles| GasCell {
                moles: moles.max(0.0),
            })
            .collect::<Vec<_>>();
        let last_broadcast_moles = cells.iter().map(|c| c.moles).collect();
        Ok(Self {
            width,
            height,
            cells,
            passable,
            last_broadcast_moles,
            scratch_flows: Vec::new(),
            scratch_outgoing: vec![0.0; size],
            scratch_source_scale: vec![1.0; size],
            scratch_delta: vec![0.0; size],
        })
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
        let substeps_f = (dt / max_substep_dt).ceil().max(1.0);
        // Clamp to avoid overflow when converting to u32 for extremely large dt values.
        let substeps = if substeps_f > u32::MAX as f32 {
            u32::MAX
        } else {
            substeps_f as u32
        };
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

        // Change tiles to walls (floor -> wall transition: moles zeroed)
        tilemap.set(IVec2::new(1, 1), TileKind::Wall);
        grid.sync_walls(&tilemap);

        // Check passability updated
        assert!(!grid.passable[4]); // (1, 1) now impassable
        assert!(grid.passable[0]); // (0, 0) still passable

        // Moles at (1,1) should be zeroed (walls always hold 0.0 moles)
        assert_eq!(grid.pressure_at(IVec2::new(1, 1)), Some(0.0));
        assert_eq!(grid.pressure_at(IVec2::new(0, 0)), Some(3.0));

        // (1,1) is now 0.0 moles
        assert_eq!(grid.total_moles(), 3.0);

        // Change wall back to floor (wall -> floor transition)
        tilemap.set(IVec2::new(1, 1), TileKind::Floor);
        grid.sync_walls(&tilemap);
        assert!(grid.passable[4]); // (1, 1) passable again
        assert_eq!(grid.pressure_at(IVec2::new(1, 1)), Some(0.0)); // was zeroed, stays 0

        // Test wall removal: add a wall, then re-sync
        tilemap.set(IVec2::new(2, 2), TileKind::Wall);
        grid.sync_walls(&tilemap);
        assert!(!grid.passable[8]); // (2, 2) is wall, impassable
        assert_eq!(grid.pressure_at(IVec2::new(2, 2)), Some(0.0)); // zeroed

        // Remove wall
        tilemap.set(IVec2::new(2, 2), TileKind::Floor);
        grid.sync_walls(&tilemap);

        assert!(grid.passable[8]); // (2, 2) now passable
        assert_eq!(grid.pressure_at(IVec2::new(2, 2)), Some(0.0)); // vacuum (was already 0)
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

    #[test]
    fn test_moles_vec_roundtrip() {
        let mut grid = GasGrid::new(3, 2);
        grid.set_moles(IVec2::new(0, 0), 1.0);
        grid.set_moles(IVec2::new(1, 0), 2.5);
        grid.set_moles(IVec2::new(2, 1), 7.0);

        let moles = grid.moles_vec();
        assert_eq!(moles.len(), 6);
        assert_eq!(moles[0], 1.0);
        assert_eq!(moles[1], 2.5);
        assert_eq!(moles[5], 7.0);

        let passable = grid.passable_vec();
        let restored =
            GasGrid::from_moles_vec(3, 2, moles, passable).expect("roundtrip should succeed");
        assert_eq!(restored.total_moles(), grid.total_moles());
        assert_eq!(
            restored.pressure_at(IVec2::new(0, 0)),
            grid.pressure_at(IVec2::new(0, 0))
        );
        assert_eq!(
            restored.pressure_at(IVec2::new(2, 1)),
            grid.pressure_at(IVec2::new(2, 1))
        );
    }

    #[test]
    fn test_from_moles_vec_length_mismatch() {
        let result = GasGrid::from_moles_vec(3, 2, vec![1.0; 5], vec![true; 6]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("mismatch"));
    }

    #[test]
    fn test_from_moles_vec_passable_length_mismatch() {
        let result = GasGrid::from_moles_vec(3, 2, vec![1.0; 6], vec![true; 5]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("mismatch"));
    }

    #[test]
    fn test_from_moles_vec_negative_clamped() {
        let grid = GasGrid::from_moles_vec(2, 1, vec![-5.0, 3.0], vec![true; 2])
            .expect("construction should succeed");
        assert_eq!(grid.pressure_at(IVec2::new(0, 0)), Some(0.0));
        assert_eq!(grid.pressure_at(IVec2::new(1, 0)), Some(3.0));
    }

    #[test]
    fn test_from_moles_vec_overflow() {
        let result = GasGrid::from_moles_vec(u32::MAX, 2, vec![], vec![]);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("overflow"));
    }

    #[test]
    fn test_pressure_gradient_at_uniform() {
        // Uniform pressure → zero gradient everywhere.
        let mut grid = GasGrid::new(3, 3);
        let tilemap = Tilemap::new(3, 3, TileKind::Floor);
        grid.sync_walls(&tilemap);
        for y in 0..3 {
            for x in 0..3 {
                grid.set_moles(IVec2::new(x, y), 5.0);
            }
        }
        let g = grid.pressure_gradient_at(IVec2::new(1, 1));
        assert!((g.x).abs() < 1e-6, "expected zero x-gradient, got {}", g.x);
        assert!((g.y).abs() < 1e-6, "expected zero y-gradient, got {}", g.y);
    }

    #[test]
    fn test_pressure_gradient_at_x_direction() {
        // High pressure on right (x+1), vacuum on left (x-1): gradient should point +x.
        let mut grid = GasGrid::new(3, 1);
        let tilemap = Tilemap::new(3, 1, TileKind::Floor);
        grid.sync_walls(&tilemap);
        grid.set_moles(IVec2::new(0, 0), 0.0);
        grid.set_moles(IVec2::new(1, 0), 5.0);
        grid.set_moles(IVec2::new(2, 0), 10.0);

        let g = grid.pressure_gradient_at(IVec2::new(1, 0));
        // (p_x_pos - p_x_neg) / 2.0 = (10.0 - 0.0) / 2.0 = 5.0
        assert!(
            (g.x - 5.0).abs() < 1e-6,
            "expected x-gradient = 5.0, got {}",
            g.x
        );
    }

    #[test]
    fn test_pressure_gradient_at_wall_neighbour() {
        // Wall on one side: the centre pressure is used in place of the impassable neighbour,
        // contributing zero gradient for that direction.
        let mut grid = GasGrid::new(3, 1);
        let mut tilemap = Tilemap::new(3, 1, TileKind::Floor);
        tilemap.set(IVec2::new(0, 0), TileKind::Wall);
        grid.sync_walls(&tilemap);
        // centre pressure
        grid.set_moles(IVec2::new(1, 0), 4.0);
        // right neighbour
        grid.set_moles(IVec2::new(2, 0), 8.0);

        let g = grid.pressure_gradient_at(IVec2::new(1, 0));
        // Left is wall → use centre (4.0). Right = 8.0.
        // gradient.x = (8.0 - 4.0) / 2.0 = 2.0
        assert!(
            (g.x - 2.0).abs() < 1e-6,
            "expected x-gradient = 2.0 with wall on left, got {}",
            g.x
        );
    }

    #[test]
    fn test_compute_and_apply_delta_changes() {
        let mut grid = GasGrid::new(3, 1);
        grid.set_moles(IVec2::new(0, 0), 5.0);
        grid.set_moles(IVec2::new(1, 0), 5.0);
        grid.set_moles(IVec2::new(2, 0), 5.0);
        grid.update_last_broadcast_moles();

        // No changes yet
        assert!(grid.compute_delta_changes(0.01).is_empty());

        // Modify one cell
        grid.set_moles(IVec2::new(1, 0), 7.5);

        let delta = grid.compute_delta_changes(0.01);
        assert_eq!(delta.len(), 1);
        assert_eq!(delta[0], (1u16, 7.5));

        // Update baseline and verify no delta after
        grid.update_last_broadcast_moles();
        assert!(grid.compute_delta_changes(0.01).is_empty());
    }

    #[test]
    fn test_apply_delta_changes() {
        let mut grid = GasGrid::new(3, 1);
        grid.set_moles(IVec2::new(0, 0), 1.0);
        grid.set_moles(IVec2::new(1, 0), 2.0);
        grid.set_moles(IVec2::new(2, 0), 3.0);

        grid.apply_delta_changes(&[(1u16, 9.0), (2u16, 0.5)]);

        assert_eq!(grid.pressure_at(IVec2::new(0, 0)), Some(1.0));
        assert_eq!(grid.pressure_at(IVec2::new(1, 0)), Some(9.0));
        assert_eq!(grid.pressure_at(IVec2::new(2, 0)), Some(0.5));

        // Out-of-bounds index silently ignored
        grid.apply_delta_changes(&[(100u16, 999.0)]);
        assert_eq!(grid.total_moles(), 10.5);
    }

    #[test]
    fn test_sync_walls_zeros_moles_on_wall() {
        let mut grid = GasGrid::new(3, 1);
        let mut tilemap = Tilemap::new(3, 1, TileKind::Floor);
        grid.set_moles(IVec2::new(0, 0), 10.0);
        grid.set_moles(IVec2::new(1, 0), 10.0);
        grid.set_moles(IVec2::new(2, 0), 10.0);
        grid.sync_walls(&tilemap);

        // Seal the middle cell
        tilemap.set(IVec2::new(1, 0), TileKind::Wall);
        grid.sync_walls(&tilemap);

        // Moles at (1,0) must be zeroed; neighbours untouched
        assert_eq!(grid.pressure_at(IVec2::new(1, 0)), Some(0.0));
        assert_eq!(grid.pressure_at(IVec2::new(0, 0)), Some(10.0));
        assert_eq!(grid.pressure_at(IVec2::new(2, 0)), Some(10.0));
    }
}
