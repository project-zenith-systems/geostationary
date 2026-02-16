use bevy::prelude::*;
use tiles::Tilemap;

mod gas_grid;
pub use gas_grid::{GasCell, GasGrid};

mod debug_overlay;
pub use debug_overlay::{AtmosDebugOverlay, OverlayQuad};

/// Creates and initializes a GasGrid from a Tilemap.
/// All floor cells are filled with the given standard atmospheric pressure.
pub fn initialize_gas_grid(tilemap: &Tilemap, standard_pressure: f32) -> GasGrid {
    let mut gas_grid = GasGrid::new(tilemap.width(), tilemap.height());
    
    // Sync walls from tilemap to mark impassable cells
    gas_grid.sync_walls(tilemap);
    
    // Fill all floor cells with standard pressure
    for y in 0..tilemap.height() {
        for x in 0..tilemap.width() {
            let pos = IVec2::new(x as i32, y as i32);
            if tilemap.is_walkable(pos) {
                gas_grid.set_moles(pos, standard_pressure);
            }
        }
    }
    
    gas_grid
}

/// Plugin that manages atmospheric simulation in the game.
/// Registers the GasGrid as a Bevy resource and provides the infrastructure
/// for gas diffusion across the tilemap.
pub struct AtmosphericsPlugin;

impl Plugin for AtmosphericsPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<GasGrid>();
        app.init_resource::<AtmosDebugOverlay>();
        app.add_systems(
            Update,
            (
                debug_overlay::toggle_overlay,
                debug_overlay::spawn_overlay_quads,
                debug_overlay::despawn_overlay_quads,
                debug_overlay::update_overlay_colors,
            )
                .chain(),
        );
    }
}
