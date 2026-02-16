use bevy::prelude::*;

mod gas_grid;
pub use gas_grid::{GasCell, GasGrid};

mod debug_overlay;
pub use debug_overlay::{AtmosDebugOverlay, OverlayQuad};

/// Plugin that manages atmospheric simulation in the game.
/// Registers the GasGrid as a Bevy resource and provides the infrastructure
/// for gas diffusion across the tilemap.
pub struct AtmosphericsPlugin;

impl Plugin for AtmosphericsPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<GasGrid>();
        app.init_resource::<AtmosDebugOverlay>();
        app.add_systems(Update, (
            debug_overlay::toggle_overlay,
            debug_overlay::manage_overlay_quads,
            debug_overlay::update_overlay_colors,
        ).chain());
    }
}
