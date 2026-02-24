use bevy::prelude::*;
use bevy::window::PrimaryWindow;
use network::{
    NetworkSet, PlayerEvent, Server, StreamDef, StreamDirection, StreamReader, StreamRegistry,
    StreamSender,
};
use tiles::{TileKind, Tilemap};
use wincode::{SchemaRead, SchemaWrite};

mod gas_grid;
pub use gas_grid::{GasCell, GasGrid};

mod debug_overlay;
pub use debug_overlay::{AtmosDebugOverlay, OverlayQuad};

/// Resource that controls whether the atmospherics simulation is paused.
/// When true, `diffusion_step_system` skips advancing the gas grid.
/// Toggle with F5.
#[derive(Resource, Default)]
pub struct AtmosSimPaused(pub bool);

/// Stream tag for the server→client atmospherics stream (stream 2).
pub const ATMOS_STREAM_TAG: u8 = 2;

/// Wire format for stream 2 (server→client atmospherics stream).
#[derive(Debug, Clone, SchemaRead, SchemaWrite)]
pub enum AtmosStreamMessage {
    /// Full gas grid snapshot sent once on connect.
    GasGridData {
        width: u32,
        height: u32,
        gas_moles: Vec<f32>,
    },
}

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

/// System that toggles the atmospherics simulation pause state on F5 keypress.
/// When paused, `diffusion_step_system` does not advance the gas grid.
fn pause_toggle_input(keyboard: Res<ButtonInput<KeyCode>>, mut paused: ResMut<AtmosSimPaused>) {
    if keyboard.just_pressed(KeyCode::F5) {
        paused.0 = !paused.0;
        info!(
            "Atmospherics simulation {}",
            if paused.0 { "paused" } else { "resumed" }
        );
    }
}

/// System that advances the atmospherics simulation by one fixed-timestep tick.
/// Runs in `FixedUpdate` so gas diffusion happens at a consistent simulation rate.
/// Skips if the simulation is paused via `AtmosSimPaused`.
fn diffusion_step_system(
    time: Res<Time<Fixed>>,
    paused: Res<AtmosSimPaused>,
    gas_grid: Option<ResMut<GasGrid>>,
) {
    if paused.0 {
        return;
    }

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
        app.init_resource::<AtmosSimPaused>();
        app.add_systems(
            FixedUpdate,
            (wall_sync_system, diffusion_step_system).chain(),
        );
        app.add_systems(
            Update,
            (
                wall_toggle_input,
                manual_step_input,
                pause_toggle_input,
                debug_overlay::toggle_overlay,
                debug_overlay::spawn_overlay_quads,
                debug_overlay::despawn_overlay_quads,
                debug_overlay::update_overlay_colors,
            )
                .chain(),
        );
        app.add_systems(
            PreUpdate,
            handle_atmos_stream
                .run_if(not(resource_exists::<Server>))
                .after(NetworkSet::Receive),
        );
        app.add_systems(
            Update,
            send_gas_grid_on_connect.run_if(resource_exists::<Server>),
        );

        // Register stream 2 (server→client atmospherics stream). Requires NetworkPlugin to be added first.
        let mut registry = app
            .world_mut()
            .get_resource_mut::<StreamRegistry>()
            .expect(
                "AtmosphericsPlugin requires NetworkPlugin to be added before it (StreamRegistry not found)",
            );
        let (sender, reader): (
            StreamSender<AtmosStreamMessage>,
            StreamReader<AtmosStreamMessage>,
        ) = registry.register(StreamDef {
            tag: ATMOS_STREAM_TAG,
            name: "atmospherics",
            direction: StreamDirection::ServerToClient,
        });
        app.insert_resource(sender);
        app.insert_resource(reader);
    }
}

/// Client-side system: handles incoming gas grid snapshots from the server on stream 2.
/// Drains [`StreamReader<AtmosStreamMessage>`], reconstructs a [`GasGrid`] via
/// [`GasGrid::from_moles_vec`], and inserts it as a resource.
fn handle_atmos_stream(
    mut commands: Commands,
    mut reader: ResMut<StreamReader<AtmosStreamMessage>>,
) {
    for msg in reader.drain() {
        match msg {
            AtmosStreamMessage::GasGridData {
                width,
                height,
                gas_moles,
            } => match GasGrid::from_moles_vec(width, height, gas_moles) {
                Ok(gas_grid) => {
                    info!(
                        "Received gas grid {}×{} from server",
                        width, height
                    );
                    commands.insert_resource(gas_grid);
                }
                Err(e) => error!("Invalid gas grid data on stream {ATMOS_STREAM_TAG}: {e}"),
            },
        }
    }
}

/// Server-side system: sends a full gas grid snapshot + [`StreamReady`] to each joining client.
/// Listens to the [`PlayerEvent::Joined`] lifecycle event so `AtmosphericsPlugin` is decoupled from
/// internal network events.
fn send_gas_grid_on_connect(
    mut events: MessageReader<PlayerEvent>,
    atmos_sender: Option<Res<StreamSender<AtmosStreamMessage>>>,
    gas_grid: Option<Res<GasGrid>>,
) {
    for event in events.read() {
        let PlayerEvent::Joined { id: from, .. } = event else {
            continue;
        };
        let sender = match atmos_sender.as_deref() {
            Some(s) => s,
            None => {
                error!(
                    "No AtmosStreamMessage sender available for ClientId({})",
                    from.0
                );
                continue;
            }
        };

        let grid = match gas_grid.as_deref() {
            Some(g) => g,
            None => {
                error!("No GasGrid resource available for ClientId({})", from.0);
                continue;
            }
        };

        let msg = AtmosStreamMessage::GasGridData {
            width: grid.width(),
            height: grid.height(),
            gas_moles: grid.moles_vec(),
        };

        if let Err(e) = sender.send_to(*from, &msg) {
            error!(
                "Failed to send GasGridData to ClientId({}): {}",
                from.0, e
            );
            continue;
        }

        if let Err(e) = sender.send_stream_ready_to(*from) {
            error!(
                "Failed to send StreamReady to ClientId({}): {}",
                from.0, e
            );
            continue;
        }

        info!(
            "Sent gas grid snapshot {}×{} + StreamReady to ClientId({})",
            grid.width(),
            grid.height(),
            from.0
        );
    }
}
