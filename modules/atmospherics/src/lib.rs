use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use tiles::{TileKind, Tilemap};

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

/// Epsilon threshold for detecting parallel rays in raycasting.
/// Used to avoid division by near-zero values when the ray is nearly parallel to the ground plane.
const RAY_PARALLEL_EPSILON: f32 = 0.001;
/// Simulation time step (in seconds) applied when advancing the atmospherics simulation manually
/// (e.g., via the F4 key). A value of 2.0 seconds makes gas movement visibly noticeable per step,
/// while still keeping the number of manual steps reasonable during debugging.
const MANUAL_STEP_DT: f32 = 2.0;

/// Performs raycasting from the camera through the cursor to find the tile position.
/// Returns the tile grid position (IVec2) if a tile is found within the raycast.
fn raycast_tile_from_cursor(
    camera_query: &Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    window_query: &Query<&Window, With<PrimaryWindow>>,
) -> Option<IVec2> {
    let (camera, camera_transform) = camera_query.single().ok()?;
    let window = window_query.single().ok()?;
    let cursor_position = window.cursor_position()?;

    // Convert cursor position to a ray in world space
    let ray = camera
        .viewport_to_world(camera_transform, cursor_position)
        .ok()?;

    // Find intersection with the ground plane (y = 0)
    // Ray equation: point = origin + t * direction
    // For y = 0: origin.y + t * direction.y = 0
    // Solve for t: t = -origin.y / direction.y
    if ray.direction.y.abs() < RAY_PARALLEL_EPSILON {
        // Ray is nearly parallel to ground plane
        return None;
    }

    let t = -ray.origin.y / ray.direction.y;
    if t < 0.0 {
        // Intersection is behind the camera
        return None;
    }

    let intersection = ray.origin + ray.direction * t;

    // Convert world position to tile coordinates
    // Tiles are centered at integer coordinates (e.g., tile at (0,0) is centered at world (0,0))
    let tile_x = intersection.x.round() as i32;
    let tile_z = intersection.z.round() as i32;

    Some(IVec2::new(tile_x, tile_z))
}

/// System that toggles walls when the middle mouse button is clicked.
/// Uses raycasting to determine which tile is under the cursor.
/// When a wall is removed (Wall -> Floor), the gas cell is set to 0.0 moles (vacuum).
/// When a wall is added (Floor -> Wall), the cell becomes impassable but preserves its moles.
fn wall_toggle_input(
    mouse_input: Res<ButtonInput<MouseButton>>,
    camera_query: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    window_query: Query<&Window, With<PrimaryWindow>>,
    tilemap: Option<ResMut<Tilemap>>,
    gas_grid: Option<ResMut<GasGrid>>,
) {
    if !mouse_input.just_pressed(MouseButton::Middle) {
        return;
    }

    let (Some(mut tilemap), Some(mut gas_grid)) = (tilemap, gas_grid) else {
        return;
    };

    let Some(tile_pos) = raycast_tile_from_cursor(&camera_query, &window_query) else {
        return;
    };

    // Check if the tile is within bounds
    let Some(current_tile) = tilemap.get(tile_pos) else {
        return;
    };

    // Toggle the tile
    let new_tile = match current_tile {
        TileKind::Wall => {
            // When removing a wall, set the cell to vacuum (0.0 moles)
            gas_grid.set_moles(tile_pos, 0.0);
            TileKind::Floor
        }
        TileKind::Floor => {
            // When adding a wall, preserve the current moles (cell becomes impassable)
            TileKind::Wall
        }
    };

    tilemap.set(tile_pos, new_tile);
    info!("Toggled tile at {:?} to {:?}", tile_pos, new_tile);
}

/// System that synchronizes the GasGrid passability mask with the Tilemap.
/// Runs when the Tilemap has been modified (via change detection).
/// Updates which cells allow gas flow based on whether they are Floor or Wall tiles.
fn wall_sync_system(tilemap: Option<Res<Tilemap>>, gas_grid: Option<ResMut<GasGrid>>) {
    let (Some(tilemap), Some(mut gas_grid)) = (tilemap, gas_grid) else {
        return;
    };

    if !tilemap.is_changed() {
        return;
    }

    gas_grid.sync_walls(&tilemap);
    info!("Synchronized GasGrid walls with Tilemap");
}

/// System that advances the atmospherics simulation by one manual tick.
/// Press F4 to advance diffusion by a fixed dt (MANUAL_STEP_DT), which may be internally sub-stepped, for debugging/inspection.
fn manual_step_input(keyboard: Res<ButtonInput<KeyCode>>, gas_grid: Option<ResMut<GasGrid>>) {
    if !keyboard.just_pressed(KeyCode::F4) {
        return;
    }

    let Some(mut gas_grid) = gas_grid else {
        return;
    };

    gas_grid.step(MANUAL_STEP_DT);
    info!("Atmospherics manual step (F4): dt={}", MANUAL_STEP_DT);
}

/// System that advances the atmospherics simulation by one fixed-timestep tick.
/// Runs in `FixedUpdate` so gas diffusion happens at a consistent simulation rate.
fn diffusion_tick(time: Res<Time<Fixed>>, gas_grid: Option<ResMut<GasGrid>>) {
    let Some(mut gas_grid) = gas_grid else {
        return;
    };

    gas_grid.step(time.delta_secs());
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
            FixedUpdate,
            (wall_sync_system, diffusion_tick).chain(),
        );
        app.add_systems(
            Update,
            (
                wall_toggle_input,
                wall_sync_system,
                manual_step_input,
                debug_overlay::toggle_overlay,
                debug_overlay::spawn_overlay_quads,
                debug_overlay::despawn_overlay_quads,
                debug_overlay::update_overlay_colors,
            )
                .chain(),
        );
    }
}
