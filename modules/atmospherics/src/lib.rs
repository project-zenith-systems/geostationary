use bevy::prelude::*;
use network::{
    ClientId, Headless, ModuleReadySent, NetworkSet, PlayerEvent, Server, StreamDef,
    StreamDirection, StreamReader, StreamRegistry, StreamSender,
};
use physics::{ConstantForce, RigidBody};
use tiles::Tilemap;
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
    /// Full gas grid snapshot sent on connect and every ~2 seconds.
    GasGridData {
        width: u32,
        height: u32,
        gas_moles: Vec<f32>,
        passable: Vec<bool>,
    },
    /// Incremental update broadcast at ~10 Hz; contains only cells that changed
    /// beyond the delta epsilon since the last snapshot or delta.
    GasGridDelta {
        changes: Vec<(u16, f32)>,
    },
}

/// Creates and initializes a GasGrid from a Tilemap.
/// All floor cells are filled with the given standard atmospheric pressure,
/// except cells inside `vacuum_region` (inclusive bounding rect) which start at 0.0 moles.
pub fn initialize_gas_grid(
    tilemap: &Tilemap,
    standard_pressure: f32,
    vacuum_region: Option<(IVec2, IVec2)>,
) -> GasGrid {
    let mut gas_grid = GasGrid::new(tilemap.width(), tilemap.height());

    // Sync walls from tilemap to mark impassable cells
    gas_grid.sync_walls(tilemap);

    // Fill all floor cells with standard pressure, skipping those in the vacuum region
    for y in 0..tilemap.height() {
        for x in 0..tilemap.width() {
            let pos = IVec2::new(x as i32, y as i32);
            if tilemap.is_walkable(pos) {
                let in_vacuum = vacuum_region.map_or(false, |(min, max)| {
                    pos.x >= min.x && pos.x <= max.x && pos.y >= min.y && pos.y <= max.y
                });
                if !in_vacuum {
                    gas_grid.set_moles(pos, standard_pressure);
                }
            }
        }
    }

    gas_grid.update_last_broadcast_moles();
    gas_grid
}

/// Simulation time step (in seconds) applied when advancing the atmospherics simulation manually
/// (e.g., via the F4 key). A value of 2.0 seconds makes gas movement visibly noticeable per step,
/// while still keeping the number of manual steps reasonable during debugging.
const MANUAL_STEP_DT: f32 = 2.0;

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

/// Resource that holds the configurable scale factor applied to the pressure gradient
/// to produce a force in Newtons.  Inserted by the app from `config.toml`
/// (`atmospherics.pressure_force_scale`).
#[derive(Resource, Debug, Clone, Copy)]
pub struct PressureForceScale(pub f32);

/// Scale factor applied to the pressure gradient to produce a force in Newtons.
/// A value of `50.0` makes a 1-mole/cell gradient exert 50 N on a body.
/// Adjust during integration testing to produce convincing entity movement.
const PRESSURE_FORCE_SCALE: f32 = 50.0;

/// Server-side system: applies pressure-gradient forces to nearby `RigidBody::Dynamic` entities.
///
/// For each dynamic body with a `Transform`, reads the gas pressure at the entity's grid cell and
/// its four cardinal neighbours, computes the net force vector from the central-difference pressure
/// gradient, scales it by `PRESSURE_FORCE_SCALE`, and writes it to the entity's `ConstantForce`
/// component (inserting the component if the entity doesn't yet have one).
///
/// Runs in `FixedUpdate` after `diffusion_step_system`.  Forces are overwritten every tick, so
/// there is no accumulation even if `ConstantForce` persists across frames.
fn apply_pressure_forces(
    mut commands: Commands,
    gas_grid: Option<Res<GasGrid>>,
    force_scale: Option<Res<PressureForceScale>>,
    mut query: Query<(Entity, &RigidBody, &Transform, Option<&mut ConstantForce>)>,
) {
    let Some(grid) = gas_grid else {
        return;
    };

    let scale = force_scale.map(|r| r.0).unwrap_or(PRESSURE_FORCE_SCALE);

    for (entity, rigid_body, transform, maybe_force) in &mut query {
        if *rigid_body != RigidBody::Dynamic {
            continue;
        }

        // Map world-space translation to tile-grid coordinates.
        let tile_pos = IVec2::new(
            transform.translation.x.round() as i32,
            transform.translation.z.round() as i32,
        );

        // gradient is in (x, z) tile-grid space.
        let gradient = grid.pressure_gradient_at(tile_pos);
        let force_vec = Vec3::new(gradient.x, 0.0, gradient.y) * scale;

        if let Some(mut cf) = maybe_force {
            cf.0 = force_vec;
        } else {
            commands.entity(entity).insert(ConstantForce(force_vec));
        }
    }
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
            (wall_sync_system, diffusion_step_system, apply_pressure_forces)
                .chain()
                .run_if(resource_exists::<Server>),
        );
        app.add_systems(
            FixedUpdate,
            (wall_sync_system, diffusion_step_system)
                .chain()
                .run_if(not(resource_exists::<Server>)),
        );
        app.add_systems(
            Update,
            (
                manual_step_input,
                pause_toggle_input,
                debug_overlay::toggle_overlay,
                debug_overlay::spawn_overlay_quads,
                debug_overlay::despawn_overlay_quads,
                debug_overlay::update_overlay_colors,
            )
                .chain()
                .run_if(not(resource_exists::<Headless>)),
        );
        app.add_systems(
            PreUpdate,
            handle_atmos_updates
                .run_if(not(resource_exists::<Server>))
                .after(NetworkSet::Receive),
        );
        app.init_resource::<PendingAtmosSyncs>();
        app.init_resource::<AtmosBroadcastTimers>();
        app.add_systems(
            Update,
            (
                send_gas_grid_on_connect,
                broadcast_gas_grid,
            )
                .run_if(resource_exists::<Server>),
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

/// Client-side system: handles incoming gas grid messages from the server on stream 2.
///
/// - [`AtmosStreamMessage::GasGridData`]: reconstructs a full [`GasGrid`] via
///   [`GasGrid::from_moles_vec`] and inserts/replaces the resource.
/// - [`AtmosStreamMessage::GasGridDelta`]: applies incremental cell updates to the
///   existing [`GasGrid`] resource; silently ignored when no grid is present yet.
fn handle_atmos_updates(
    mut commands: Commands,
    mut reader: ResMut<StreamReader<AtmosStreamMessage>>,
    gas_grid: Option<ResMut<GasGrid>>,
) {
    // `pending` holds a newly-received full snapshot that hasn't been committed yet.
    // Deltas that arrive in the same batch are applied to it directly so that no
    // updates are dropped within a single drain cycle.
    let mut pending: Option<GasGrid> = None;
    let mut gas_grid = gas_grid;
    for msg in reader.drain() {
        match msg {
            AtmosStreamMessage::GasGridData {
                width,
                height,
                gas_moles,
                passable,
            } => match GasGrid::from_moles_vec(width, height, gas_moles, passable) {
                Ok(new_grid) => {
                    info!("Received gas grid {}×{} from server", width, height);
                    pending = Some(new_grid);
                }
                Err(e) => error!("Invalid gas grid data on stream {ATMOS_STREAM_TAG}: {e}"),
            },
            AtmosStreamMessage::GasGridDelta { changes } => {
                if let Some(ref mut grid) = pending {
                    grid.apply_delta_changes(&changes);
                } else if let Some(ref mut grid) = gas_grid {
                    grid.apply_delta_changes(&changes);
                }
            }
        }
    }
    if let Some(new_grid) = pending {
        commands.insert_resource(new_grid);
    }
}

/// Clients that joined before the [`GasGrid`] resource was available (e.g. on a
/// listen-server where `PlayerEvent::Joined` fires before `OnEnter(InGame)`).
/// Drained once the resource exists.
#[derive(Resource, Default)]
struct PendingAtmosSyncs(Vec<ClientId>);

/// Server-side system: sends a full gas grid snapshot + [`StreamReady`] to each joining client.
/// Listens to the [`PlayerEvent::Joined`] lifecycle event so `AtmosphericsPlugin` is decoupled from
/// internal network events.
///
/// If the [`GasGrid`] resource does not exist yet (listen-server startup), the
/// client ID is queued in [`PendingAtmosSyncs`] and retried each frame.
fn send_gas_grid_on_connect(
    mut events: MessageReader<PlayerEvent>,
    atmos_sender: Option<Res<StreamSender<AtmosStreamMessage>>>,
    gas_grid: Option<Res<GasGrid>>,
    mut module_ready: MessageWriter<ModuleReadySent>,
    mut pending: ResMut<PendingAtmosSyncs>,
) {
    // Collect newly joined clients.
    for event in events.read() {
        let PlayerEvent::Joined { id: from, .. } = event else {
            continue;
        };
        pending.0.push(*from);
    }

    // Nothing to do if no clients are waiting.
    if pending.0.is_empty() {
        return;
    }

    let Some(sender) = atmos_sender.as_deref() else {
        error!("No AtmosStreamMessage sender available; {} client(s) waiting", pending.0.len());
        return;
    };

    let Some(grid) = gas_grid.as_deref() else {
        // Resource not yet inserted (listen-server: setup_world hasn't run).
        // Keep clients queued; we'll retry next frame.
        return;
    };

    let clients = std::mem::take(&mut pending.0);
    for from in clients {
        let msg = AtmosStreamMessage::GasGridData {
            width: grid.width(),
            height: grid.height(),
            gas_moles: grid.moles_vec(),
            passable: grid.passable_vec().to_vec(),
        };

        if let Err(e) = sender.send_to(from, &msg) {
            error!("Failed to send GasGridData to ClientId({}): {}", from.0, e);
            continue;
        }

        if let Err(e) = sender.send_stream_ready_to(from) {
            error!("Failed to send StreamReady to ClientId({}): {}", from.0, e);
            continue;
        }

        info!(
            "Sent gas grid snapshot {}×{} + StreamReady to ClientId({})",
            grid.width(),
            grid.height(),
            from.0
        );
        module_ready.write(ModuleReadySent { client: from });
    }
}

/// Moles-change threshold for including a cell in a [`GasGridDelta`].
/// Cells whose moles have changed by less than this amount since the last broadcast
/// are omitted to reduce network traffic.
const DELTA_EPSILON: f32 = 0.01;

/// Interval between full [`GasGridData`] snapshot broadcasts (seconds).
const FULL_SNAPSHOT_INTERVAL: f32 = 2.0;

/// Interval between incremental [`GasGridDelta`] broadcasts (seconds).
/// 0.1 s → ~10 Hz update rate.
const DELTA_INTERVAL: f32 = 0.1;

/// Timers that drive the periodic gas grid replication broadcasts.
#[derive(Resource)]
pub struct AtmosBroadcastTimers {
    /// Fires every [`FULL_SNAPSHOT_INTERVAL`] seconds to trigger a full snapshot broadcast.
    pub full_snapshot: Timer,
    /// Fires every [`DELTA_INTERVAL`] seconds to trigger an incremental delta broadcast.
    pub delta: Timer,
}

impl Default for AtmosBroadcastTimers {
    fn default() -> Self {
        Self {
            full_snapshot: Timer::from_seconds(FULL_SNAPSHOT_INTERVAL, TimerMode::Repeating),
            delta: Timer::from_seconds(DELTA_INTERVAL, TimerMode::Repeating),
        }
    }
}

/// Server-side system: broadcasts gas grid replication messages to all connected clients.
///
/// - Every [`DELTA_INTERVAL`] seconds (~10 Hz): computes a [`GasGridDelta`] of cells
///   that have changed beyond [`DELTA_EPSILON`] since the last broadcast and sends it
///   to all clients.  [`GasGrid::last_broadcast_moles`] is updated after each delta.
/// - Every [`FULL_SNAPSHOT_INTERVAL`] seconds (~0.5 Hz): broadcasts a full
///   [`GasGridData`] snapshot to resync clients.  [`GasGrid::last_broadcast_moles`] is
///   updated after the snapshot so the following deltas are relative to it.
fn broadcast_gas_grid(
    time: Res<Time>,
    mut timers: ResMut<AtmosBroadcastTimers>,
    atmos_sender: Option<Res<StreamSender<AtmosStreamMessage>>>,
    gas_grid: Option<ResMut<GasGrid>>,
) {
    timers.full_snapshot.tick(time.delta());
    timers.delta.tick(time.delta());

    let Some(sender) = atmos_sender.as_deref() else {
        return;
    };
    let Some(mut grid) = gas_grid else {
        return;
    };

    // Full snapshot broadcast takes priority; also resets the delta baseline.
    if timers.full_snapshot.just_finished() {
        let msg = AtmosStreamMessage::GasGridData {
            width: grid.width(),
            height: grid.height(),
            gas_moles: grid.moles_vec(),
            passable: grid.passable_vec().to_vec(),
        };
        match sender.broadcast(&msg) {
            Ok(()) => grid.update_last_broadcast_moles(),
            Err(e) => error!("Failed to broadcast GasGridData: {e}"),
        }
        return;
    }

    // Incremental delta broadcast.
    if timers.delta.just_finished() {
        let changes = grid.compute_delta_changes(DELTA_EPSILON);
        if !changes.is_empty() {
            let msg = AtmosStreamMessage::GasGridDelta { changes };
            match sender.broadcast(&msg) {
                Ok(()) => grid.update_last_broadcast_moles(),
                Err(e) => error!("Failed to broadcast GasGridDelta: {e}"),
            }
        }
    }
}
