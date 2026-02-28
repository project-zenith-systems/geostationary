use bevy::prelude::*;
use input::{PointerAction, WorldHit};
use network::{
    ClientId, Headless, ModuleReadySent, NetworkSet, PlayerEvent, Server, StreamDef,
    StreamDirection, StreamReader, StreamRegistry, StreamSender,
};
use physics::{Collider, RigidBody};
use wincode::{SchemaRead, SchemaWrite};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Reflect, SchemaRead, SchemaWrite)]
#[reflect(Debug, PartialEq)]
pub enum TileKind {
    Floor,
    Wall,
}

#[derive(Component, Debug, Clone, Copy, PartialEq, Eq, Reflect)]
#[reflect(Component, Debug, PartialEq)]
pub struct Tile {
    pub position: IVec2,
}

impl TileKind {
    pub fn is_walkable(&self) -> bool {
        match self {
            TileKind::Floor => true,
            TileKind::Wall => false,
        }
    }
}

#[derive(Debug, Clone, Resource, Reflect)]
#[reflect(Debug, Resource)]
pub struct Tilemap {
    width: u32,
    height: u32,
    tiles: Vec<TileKind>,
}

impl Tilemap {
    pub fn new(width: u32, height: u32, fill: TileKind) -> Self {
        let size = (width * height) as usize;
        Self {
            width,
            height,
            tiles: vec![fill; size],
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    fn coord_to_index(&self, pos: IVec2) -> Option<usize> {
        if pos.x >= 0 && pos.x < self.width as i32 && pos.y >= 0 && pos.y < self.height as i32 {
            Some((pos.y * self.width as i32 + pos.x) as usize)
        } else {
            None
        }
    }

    pub fn get(&self, pos: IVec2) -> Option<TileKind> {
        self.coord_to_index(pos).map(|idx| self.tiles[idx])
    }

    pub fn set(&mut self, pos: IVec2, kind: TileKind) -> bool {
        if let Some(idx) = self.coord_to_index(pos) {
            self.tiles[idx] = kind;
            true
        } else {
            false
        }
    }

    pub fn is_walkable(&self, pos: IVec2) -> bool {
        self.get(pos).is_some_and(|kind| kind.is_walkable())
    }

    /// Creates a 16x10 test room for decompression scenarios, with perimeter walls
    /// and a central double wall that separates a left (pressurized) chamber from
    /// a right (vacuum) chamber.
    pub fn test_room() -> Tilemap {
        let mut tilemap = Tilemap::new(16, 10, TileKind::Floor);

        // Perimeter walls
        for x in 0..16 {
            tilemap.set(IVec2::new(x, 0), TileKind::Wall);
            tilemap.set(IVec2::new(x, 9), TileKind::Wall);
        }
        for y in 0..10 {
            tilemap.set(IVec2::new(0, y), TileKind::Wall);
            tilemap.set(IVec2::new(15, y), TileKind::Wall);
        }

        // Separating wall between left (pressurized, cols 1–8) and right (vacuum, cols 11–14) chambers
        for y in 0..10 {
            tilemap.set(IVec2::new(9, y), TileKind::Wall);
            tilemap.set(IVec2::new(10, y), TileKind::Wall);
        }

        tilemap
    }

    /// Returns an iterator over all tiles with their positions and kinds
    pub fn iter(&self) -> impl Iterator<Item = (IVec2, TileKind)> + '_ {
        (0..self.height).flat_map(move |y| {
            (0..self.width).map(move |x| {
                let pos = IVec2::new(x as i32, y as i32);
                let kind = self
                    .get(pos)
                    .expect("iterator should only visit valid positions");
                (pos, kind)
            })
        })
    }
}

/// Wire format for stream 1 (server→client tiles stream).
#[derive(Debug, Clone, SchemaRead, SchemaWrite)]
pub enum TilesStreamMessage {
    /// Full tilemap snapshot sent once on connect.
    TilemapData {
        width: u32,
        height: u32,
        tiles: Vec<TileKind>,
    },
    /// Incremental mutation broadcast to all clients after the server applies a toggle.
    TileMutated {
        position: [i32; 2],
        kind: TileKind,
    },
}

/// Client→server request to toggle a tile at the given position (stream 4).
///
/// **Temporary:** This dedicated stream will be superseded by a general-purpose
/// interactions stream in a later plan iteration. Do not rely on stream 4 being
/// tile-specific long-term.
#[derive(Debug, Clone, SchemaRead, SchemaWrite)]
pub struct TileToggle {
    pub position: [i32; 2],
    pub kind: TileKind,
}

/// Bevy event fired by the interactions module when the player requests a tile mutation.
/// Consumed by [`execute_tile_toggle`] to send [`TileToggle`] on stream 4.
#[derive(Message, Debug, Clone, Copy)]
pub struct TileToggleRequest {
    pub position: IVec2,
    pub kind: TileKind,
}

/// Bevy event fired when a tile mutation arrives from the server (or is applied locally
/// on a listen-server). Consumed by [`apply_tile_mutation`] to update the visual
/// representation incrementally.
#[derive(Message, Debug, Clone, Copy)]
pub struct TileMutated {
    pub position: IVec2,
    pub kind: TileKind,
}

/// Stream tag for the server→client tiles stream (stream 1).
pub const TILES_STREAM_TAG: u8 = 1;

/// Stream tag for the client→server tile-toggle stream (stream 4).
pub const TILE_TOGGLE_STREAM_TAG: u8 = 4;

/// Decode a [`TilesStreamMessage`] from raw stream-frame bytes.
pub fn decode_tiles_message(bytes: &[u8]) -> Result<TilesStreamMessage, String> {
    wincode::deserialize(bytes).map_err(|e| e.to_string())
}

impl From<&Tilemap> for TilesStreamMessage {
    fn from(tilemap: &Tilemap) -> Self {
        TilesStreamMessage::TilemapData {
            width: tilemap.width,
            height: tilemap.height,
            tiles: tilemap.tiles.clone(),
        }
    }
}

impl TryFrom<TilesStreamMessage> for Tilemap {
    type Error = String;

    fn try_from(msg: TilesStreamMessage) -> Result<Self, Self::Error> {
        match msg {
            TilesStreamMessage::TilemapData {
                width,
                height,
                tiles,
            } => {
                let expected = width
                    .checked_mul(height)
                    .and_then(|n| usize::try_from(n).ok())
                    .ok_or_else(|| format!("Tilemap dimensions {width}×{height} overflow"))?;
                if tiles.len() != expected {
                    return Err(format!(
                        "tile data length mismatch: expected {expected}, got {}",
                        tiles.len()
                    ));
                }
                Ok(Tilemap {
                    width,
                    height,
                    tiles,
                })
            }
            TilesStreamMessage::TileMutated { .. } => {
                Err("TileMutated is not a full tilemap snapshot".to_string())
            }
        }
    }
}

impl Tilemap {
    /// Serialize the tilemap to bytes using the `TilesStreamMessage::TilemapData` wire format.
    pub fn to_bytes(&self) -> Result<Vec<u8>, String> {
        wincode::serialize(&TilesStreamMessage::from(self)).map_err(|e| {
            format!(
                "Failed to serialize Tilemap ({}×{}): {e}",
                self.width, self.height
            )
        })
    }

    /// Deserialize a tilemap from bytes produced by [`Tilemap::to_bytes`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        decode_tiles_message(bytes).and_then(Tilemap::try_from)
    }
}

pub struct TilesPlugin;

impl Plugin for TilesPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<TileKind>();
        app.register_type::<Tilemap>();
        app.register_type::<Tile>();

        app.add_message::<TileToggleRequest>();
        app.add_message::<TileMutated>();

        // Register messages that raycast_tiles / execute_tile_toggle read/write
        // so the resources exist even when InputPlugin is not added (e.g. headless tests).
        app.add_message::<PointerAction>();
        app.add_message::<WorldHit>();

        let headless = app.world().contains_resource::<Headless>();
        if !headless {
            // Tile mesh spawning and visual mutation are visual-only; skip in headless server mode.
            app.init_resource::<TileMeshes>();
            app.add_systems(Update, spawn_tile_meshes);
            // On a listen-server, TileMutated events are written by handle_tile_toggle
            // (which runs in the same Update schedule).  Ordering after it ensures those
            // events are visible in the same frame.
            app.add_systems(
                Update,
                apply_tile_mutation
                    .run_if(resource_exists::<Server>)
                    .after(handle_tile_toggle),
            );
            // On a dedicated client, TileMutated events come from handle_tiles_stream
            // (PreUpdate), so no intra-Update ordering is needed.
            app.add_systems(
                Update,
                apply_tile_mutation.run_if(not(resource_exists::<Server>)),
            );
            app.add_systems(
                Update,
                (raycast_tiles, execute_tile_toggle),
            );
        }

        app.add_systems(
            PreUpdate,
            handle_tiles_stream
                .run_if(not(resource_exists::<Server>))
                .after(NetworkSet::Receive),
        );
        // Runs in Update (not PreUpdate) so that StateTransition has already
        // processed OnEnter(InGame) → setup_world → Tilemap inserted.
        // ClientJoined messages written in PreUpdate are still readable here
        // thanks to double-buffered message semantics.
        app.init_resource::<PendingTilesSyncs>();
        app.add_systems(
            Update,
            send_tilemap_on_connect.run_if(resource_exists::<Server>),
        );
        app.add_systems(
            Update,
            handle_tile_toggle
                .run_if(resource_exists::<Server>)
                .after(send_tilemap_on_connect),
        );

        // Register streams. Requires NetworkPlugin to be added first.
        let mut registry = app.world_mut().get_resource_mut::<StreamRegistry>().expect(
            "TilesPlugin requires NetworkPlugin to be added before it (StreamRegistry not found)",
        );
        let (sender, reader): (
            StreamSender<TilesStreamMessage>,
            StreamReader<TilesStreamMessage>,
        ) = registry.register(StreamDef {
            tag: TILES_STREAM_TAG,
            name: "tiles",
            direction: StreamDirection::ServerToClient,
        });
        let (toggle_sender, toggle_reader): (
            StreamSender<TileToggle>,
            StreamReader<TileToggle>,
        ) = registry.register(StreamDef {
            tag: TILE_TOGGLE_STREAM_TAG,
            name: "tile_toggle",
            direction: StreamDirection::ClientToServer,
        });
        app.insert_resource(sender);
        app.insert_resource(reader);
        app.insert_resource(toggle_sender);
        app.insert_resource(toggle_reader);
    }
}

#[derive(Resource)]
struct TileMeshes {
    floor_mesh: Handle<Mesh>,
    wall_mesh: Handle<Mesh>,
    floor_material: Handle<StandardMaterial>,
    wall_material: Handle<StandardMaterial>,
}

impl FromWorld for TileMeshes {
    fn from_world(world: &mut World) -> Self {
        let mut meshes = world.resource_mut::<Assets<Mesh>>();
        let floor_mesh = meshes.add(Plane3d::new(Vec3::Y, Vec2::splat(0.5)));
        let wall_mesh = meshes.add(Cuboid::new(1.0, 1.0, 1.0));

        let mut materials = world.resource_mut::<Assets<StandardMaterial>>();
        // Dark grey for floors, lighter grey for walls
        let floor_material = materials.add(StandardMaterial {
            base_color: Color::srgb(0.3, 0.3, 0.3),
            ..default()
        });
        let wall_material = materials.add(StandardMaterial {
            base_color: Color::srgb(0.6, 0.6, 0.6),
            ..default()
        });

        Self {
            floor_mesh,
            wall_mesh,
            floor_material,
            wall_material,
        }
    }
}

/// Spawns a single tile entity for the given grid position and kind.
/// Used by both [`spawn_tile_meshes`] (initial load) and [`apply_tile_mutation`]
/// (incremental updates) to guarantee identical visual and physics setup.
fn spawn_tile_entity(commands: &mut Commands, position: IVec2, kind: TileKind, tile_meshes: &TileMeshes) {
    let world_x = position.x as f32;
    let world_z = position.y as f32;
    match kind {
        TileKind::Floor => {
            // Collider is 0.1 tall (full dim), centered on transform.
            // Offset y by -0.05 so the top surface sits at y=0.0.
            commands.spawn((
                Mesh3d(tile_meshes.floor_mesh.clone()),
                MeshMaterial3d(tile_meshes.floor_material.clone()),
                Transform::from_xyz(world_x, -0.05, world_z),
                Tile { position },
                RigidBody::Static,
                Collider::cuboid(1.0, 0.1, 1.0),
            ));
        }
        TileKind::Wall => {
            commands.spawn((
                Mesh3d(tile_meshes.wall_mesh.clone()),
                MeshMaterial3d(tile_meshes.wall_material.clone()),
                Transform::from_xyz(world_x, 0.5, world_z),
                Tile { position },
                RigidBody::Static,
                // avian3d Collider::cuboid takes full dimensions, not half-extents
                Collider::cuboid(1.0, 1.0, 1.0),
            ));
        }
    }
}

fn spawn_tile_meshes(
    mut commands: Commands,
    tilemap: Option<Res<Tilemap>>,
    existing_tiles: Query<Entity, With<Tile>>,
    tile_meshes: Res<TileMeshes>,
) {
    let Some(tilemap) = tilemap else {
        // If the Tilemap resource is missing, ensure any previously spawned
        // Tile entities are cleaned up so they don't persist indefinitely.
        for entity in &existing_tiles {
            commands.entity(entity).despawn();
        }
        return;
    };

    // Only spawn for the initial load; incremental mutations are handled by
    // apply_tile_mutation.  Once any tile entities exist, this system is a no-op.
    if !existing_tiles.is_empty() {
        return;
    }

    // Spawn tile entities for the full initial tilemap.
    for (pos, kind) in tilemap.iter() {
        spawn_tile_entity(&mut commands, pos, kind, &tile_meshes);
    }
}

/// Bevy system that handles incoming tilemap messages from the server on stream 1.
/// Drains [`StreamReader<TilesStreamMessage>`], explicitly matches on each variant:
/// - [`TilesStreamMessage::TilemapData`]: validates dimensions via [`TryFrom`] and
///   inserts the [`Tilemap`] resource (initial full snapshot).
/// - [`TilesStreamMessage::TileMutated`]: applies [`Tilemap::set`] for the affected
///   cell and fires a [`TileMutated`] Bevy event so [`apply_tile_mutation`] can update
///   the visual representation incrementally.
fn handle_tiles_stream(
    mut commands: Commands,
    mut reader: ResMut<StreamReader<TilesStreamMessage>>,
    mut tilemap: Option<ResMut<Tilemap>>,
    mut mutation_events: MessageWriter<TileMutated>,
) {
    for msg in reader.drain() {
        match msg {
            variant @ TilesStreamMessage::TilemapData { .. } => match Tilemap::try_from(variant) {
                Ok(tm) => {
                    info!(
                        "Received tilemap {}×{} from server",
                        tm.width, tm.height
                    );
                    commands.insert_resource(tm);
                }
                Err(e) => error!("Invalid tilemap data on stream {TILES_STREAM_TAG}: {e}"),
            },
            TilesStreamMessage::TileMutated { position, kind } => {
                let pos = IVec2::new(position[0], position[1]);
                if let Some(ref mut tm) = tilemap {
                    tm.set(pos, kind);
                    // Only emit the mutation event once the Tilemap resource exists.
                    // This prevents spawning partial tile entities before the initial
                    // TilemapData snapshot arrives. If entities were spawned early,
                    // spawn_tile_meshes would see a non-empty tile query and skip
                    // the full initial map spawn, leaving most tiles missing.
                    mutation_events.write(TileMutated { position: pos, kind });
                }
            }
        }
    }
}

/// Clients that joined before the [`Tilemap`] resource was available (e.g. on a
/// listen-server where `PlayerEvent::Joined` fires before `OnEnter(InGame)`).
/// Drained once the resource exists.
#[derive(Resource, Default)]
struct PendingTilesSyncs(Vec<ClientId>);

/// Server-side system: sends a full tilemap snapshot + [`StreamReady`] to each joining client.
/// Listens to [`PlayerEvent::Joined`] so `TilesPlugin` is decoupled from
/// internal network events ([`ServerEvent`]).
///
/// If the [`Tilemap`] resource does not exist yet (listen-server startup), the
/// client ID is queued in [`PendingTilesSyncs`] and retried each frame.
fn send_tilemap_on_connect(
    mut events: MessageReader<PlayerEvent>,
    tiles_sender: Option<Res<StreamSender<TilesStreamMessage>>>,
    tilemap: Option<Res<Tilemap>>,
    mut module_ready: MessageWriter<ModuleReadySent>,
    mut pending: ResMut<PendingTilesSyncs>,
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

    let Some(ts) = tiles_sender.as_deref() else {
        error!("No TilesStreamMessage sender available; {} client(s) waiting", pending.0.len());
        return;
    };

    let Some(map) = tilemap.as_deref() else {
        // Resource not yet inserted (listen-server: setup_world hasn't run).
        // Keep clients queued; we'll retry next frame.
        return;
    };

    let clients = std::mem::take(&mut pending.0);
    for from in clients {
        if let Err(e) = ts.send_to(from, &TilesStreamMessage::from(map)) {
            error!("Failed to send TilemapData to ClientId({}): {}", from.0, e);
            continue;
        }

        if let Err(e) = ts.send_stream_ready_to(from) {
            error!("Failed to send StreamReady to ClientId({}): {}", from.0, e);
            continue;
        }

        info!(
            "Sent tilemap snapshot {}×{} + StreamReady to ClientId({})",
            map.width(),
            map.height(),
            from.0
        );
        module_ready.write(ModuleReadySent { client: from });
    }
}

/// System that listens for left-click and right-click [`PointerAction`] events, raycasts from the
/// camera through the screen position to the ground plane (y = 0), and emits a
/// [`WorldHit`] event carrying the hit tile entity and world position if a valid
/// tile exists at the resulting grid coordinate.
///
/// Runs in `Update`, gated on absence of [`Headless`].
fn raycast_tiles(
    mut pointer_events: MessageReader<PointerAction>,
    camera_query: Query<(&Camera, &GlobalTransform), With<Camera3d>>,
    tile_query: Query<(Entity, &Tile)>,
    tilemap: Option<Res<Tilemap>>,
    mut hit_events: MessageWriter<WorldHit>,
) {
    let Some(tilemap) = tilemap else { return };
    let Ok((camera, cam_transform)) = camera_query.single() else { return };

    for action in pointer_events.read() {
        if !matches!(action.button, MouseButton::Left | MouseButton::Right) {
            continue;
        }

        let Ok(ray) = camera.viewport_to_world(cam_transform, action.screen_pos) else {
            continue;
        };

        // Convert Dir3 to Vec3 for arithmetic.
        let dir = Vec3::from(ray.direction);

        // Intersect with the y = 0 ground plane: origin.y + t * dir.y = 0.
        // Use a practical threshold rather than f32::EPSILON to avoid rejecting
        // near-horizontal rays while still preventing division by near-zero.
        if dir.y.abs() < 1e-4 {
            continue; // Ray is effectively parallel to the ground plane.
        }
        let t = -ray.origin.y / dir.y;
        if t < 0.0 {
            continue; // Intersection is behind the camera.
        }

        let world_pos = ray.origin + t * dir;
        // Grid coordinates: world X → column, world Z → row.
        let grid_pos = IVec2::new(world_pos.x.round() as i32, world_pos.z.round() as i32);

        if tilemap.get(grid_pos).is_some() {
            if let Some((entity, _)) = tile_query.iter().find(|(_, t)| t.position == grid_pos) {
                hit_events.write(WorldHit { button: action.button, entity, world_pos });
            }
        }
    }
}

/// System that reads [`TileToggleRequest`] events and sends a [`TileToggle`] message
/// to the server on stream 4 (client→server).
///
/// Runs in `Update`, gated on absence of [`Headless`].
fn execute_tile_toggle(
    mut requests: MessageReader<TileToggleRequest>,
    sender: Option<Res<StreamSender<TileToggle>>>,
) {
    let Some(ref s) = sender else {
        // Drain the event queue even when disconnected so they don't accumulate.
        for _ in requests.read() {}
        return;
    };
    for req in requests.read() {
        if let Err(e) = s.send(&TileToggle {
            position: [req.position.x, req.position.y],
            kind: req.kind,
        }) {
            error!("Failed to send TileToggle to server: {}", e);
        }
    }
}

/// Server-side system that reads [`TileToggle`] messages from stream 4, validates each
/// request (in-bounds, tile currently differs from the requested kind), applies the
/// mutation via [`Tilemap::set`], then broadcasts [`TilesStreamMessage::TileMutated`]
/// to all clients on stream 1.  Also fires a local [`TileMutated`] Bevy event so the
/// listen-server's own [`apply_tile_mutation`] system can update its visuals.
///
/// Runs in `Update`, gated on [`Server`] resource.
fn handle_tile_toggle(
    mut reader: ResMut<StreamReader<TileToggle>>,
    mut tilemap: Option<ResMut<Tilemap>>,
    sender: Option<Res<StreamSender<TilesStreamMessage>>>,
    mut mutation_events: MessageWriter<TileMutated>,
) {
    for (from, toggle) in reader.drain_from_client() {
        let position = IVec2::new(toggle.position[0], toggle.position[1]);

        let Some(ref mut tm) = tilemap else {
            warn!("handle_tile_toggle: Tilemap resource not available");
            continue;
        };

        // Validate: position must be within the tilemap bounds.
        let Some(current) = tm.get(position) else {
            warn!(
                "TileToggle from {:?}: position {:?} is out of bounds",
                from, position
            );
            continue;
        };

        // Validate: requested kind must differ from the current tile.
        if current == toggle.kind {
            warn!(
                "TileToggle from {:?}: tile at {:?} is already {:?}",
                from, position, toggle.kind
            );
            continue;
        }

        tm.set(position, toggle.kind);

        // Fire local Bevy event so the listen-server updates its own visuals.
        mutation_events.write(TileMutated { position, kind: toggle.kind });

        // Broadcast the mutation to all connected clients on stream 1.
        let Some(ref ts) = sender else {
            error!("handle_tile_toggle: tiles stream sender not available");
            continue;
        };
        if let Err(e) = ts.broadcast(&TilesStreamMessage::TileMutated {
            position: toggle.position,
            kind: toggle.kind,
        }) {
            error!("Failed to broadcast TileMutated: {}", e);
        }
    }
}

/// Client-side system that handles [`TileMutated`] events (fired by both
/// [`handle_tiles_stream`] and, on listen-servers, by [`handle_tile_toggle`]).
///
/// Despawns the existing tile entity at the affected grid position and spawns a new
/// one with the updated mesh, material, and collider via [`spawn_tile_entity`].
/// This provides incremental rendering — only the changed tile is rebuilt.
///
/// On a listen-server, runs after [`handle_tile_toggle`] (same frame visibility).
/// On a dedicated client, runs unconditionally (events arrive from PreUpdate).
fn apply_tile_mutation(
    mut commands: Commands,
    mut events: MessageReader<TileMutated>,
    tile_query: Query<(Entity, &Tile)>,
    tile_meshes: Res<TileMeshes>,
) {
    for event in events.read() {
        let TileMutated { position, kind } = *event;

        // Despawn the existing tile entity at this grid position (if any).
        for (entity, tile) in &tile_query {
            if tile.position == position {
                commands.entity(entity).despawn();
                break;
            }
        }

        // Spawn a replacement tile entity with the new kind.
        spawn_tile_entity(&mut commands, position, kind, &tile_meshes);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tile_kind_walkability() {
        assert!(TileKind::Floor.is_walkable());
        assert!(!TileKind::Wall.is_walkable());
    }

    #[test]
    fn test_tilemap_creation() {
        let tilemap = Tilemap::new(10, 10, TileKind::Floor);
        assert_eq!(tilemap.width(), 10);
        assert_eq!(tilemap.height(), 10);
    }

    #[test]
    fn test_tilemap_get_set() {
        let mut tilemap = Tilemap::new(5, 5, TileKind::Floor);

        assert_eq!(tilemap.get(IVec2::new(0, 0)), Some(TileKind::Floor));
        assert_eq!(tilemap.get(IVec2::new(4, 4)), Some(TileKind::Floor));

        assert!(tilemap.set(IVec2::new(2, 2), TileKind::Wall));
        assert_eq!(tilemap.get(IVec2::new(2, 2)), Some(TileKind::Wall));

        assert_eq!(tilemap.get(IVec2::new(-1, 0)), None);
        assert_eq!(tilemap.get(IVec2::new(0, -1)), None);
        assert_eq!(tilemap.get(IVec2::new(5, 0)), None);
        assert_eq!(tilemap.get(IVec2::new(0, 5)), None);
        assert!(!tilemap.set(IVec2::new(-1, 0), TileKind::Wall));
        assert!(!tilemap.set(IVec2::new(10, 10), TileKind::Wall));
    }

    #[test]
    fn test_tilemap_is_walkable() {
        let mut tilemap = Tilemap::new(5, 5, TileKind::Floor);

        assert!(tilemap.is_walkable(IVec2::new(0, 0)));
        assert!(tilemap.is_walkable(IVec2::new(4, 4)));

        tilemap.set(IVec2::new(2, 2), TileKind::Wall);
        assert!(!tilemap.is_walkable(IVec2::new(2, 2)));

        assert!(!tilemap.is_walkable(IVec2::new(-1, 0)));
        assert!(!tilemap.is_walkable(IVec2::new(5, 0)));
        assert!(!tilemap.is_walkable(IVec2::new(0, 5)));
    }

    #[test]
    fn test_tilemap_coordinates() {
        let mut tilemap = Tilemap::new(3, 3, TileKind::Floor);

        tilemap.set(IVec2::new(0, 0), TileKind::Wall);
        tilemap.set(IVec2::new(1, 1), TileKind::Wall);
        tilemap.set(IVec2::new(2, 2), TileKind::Wall);

        assert_eq!(tilemap.get(IVec2::new(0, 0)), Some(TileKind::Wall));
        assert_eq!(tilemap.get(IVec2::new(1, 0)), Some(TileKind::Floor));
        assert_eq!(tilemap.get(IVec2::new(2, 0)), Some(TileKind::Floor));
        assert_eq!(tilemap.get(IVec2::new(0, 1)), Some(TileKind::Floor));
        assert_eq!(tilemap.get(IVec2::new(1, 1)), Some(TileKind::Wall));
        assert_eq!(tilemap.get(IVec2::new(2, 1)), Some(TileKind::Floor));
        assert_eq!(tilemap.get(IVec2::new(0, 2)), Some(TileKind::Floor));
        assert_eq!(tilemap.get(IVec2::new(1, 2)), Some(TileKind::Floor));
        assert_eq!(tilemap.get(IVec2::new(2, 2)), Some(TileKind::Wall));
    }

    #[test]
    fn test_tilemap_test_room() {
        let room = Tilemap::test_room();
        assert_eq!(room.width(), 16);
        assert_eq!(room.height(), 10);

        // Perimeter should be walls
        for x in 0..16 {
            assert_eq!(room.get(IVec2::new(x, 0)), Some(TileKind::Wall));
            assert_eq!(room.get(IVec2::new(x, 9)), Some(TileKind::Wall));
        }
        for y in 0..10 {
            assert_eq!(room.get(IVec2::new(0, y)), Some(TileKind::Wall));
            assert_eq!(room.get(IVec2::new(15, y)), Some(TileKind::Wall));
        }

        // Left chamber (cols 1–8) should be floor
        assert_eq!(room.get(IVec2::new(4, 5)), Some(TileKind::Floor));
        assert_eq!(room.get(IVec2::new(8, 3)), Some(TileKind::Floor));

        // Separating wall (cols 9–10)
        assert_eq!(room.get(IVec2::new(9, 5)), Some(TileKind::Wall));
        assert_eq!(room.get(IVec2::new(10, 3)), Some(TileKind::Wall));

        // Right chamber (cols 11–14) should be floor
        assert_eq!(room.get(IVec2::new(11, 5)), Some(TileKind::Floor));
        assert_eq!(room.get(IVec2::new(14, 3)), Some(TileKind::Floor));
    }

    #[test]
    fn test_tilemap_iter() {
        let mut tilemap = Tilemap::new(2, 2, TileKind::Floor);
        tilemap.set(IVec2::new(1, 1), TileKind::Wall);

        let tiles: Vec<_> = tilemap.iter().collect();
        assert_eq!(tiles.len(), 4);

        assert_eq!(tiles[0], (IVec2::new(0, 0), TileKind::Floor));
        assert_eq!(tiles[1], (IVec2::new(1, 0), TileKind::Floor));
        assert_eq!(tiles[2], (IVec2::new(0, 1), TileKind::Floor));
        assert_eq!(tiles[3], (IVec2::new(1, 1), TileKind::Wall));
    }

    #[test]
    fn test_tilemap_to_from_bytes_roundtrip() {
        let original = Tilemap::test_room();
        let bytes = original.to_bytes().expect("to_bytes should succeed");
        let restored = Tilemap::from_bytes(&bytes).expect("from_bytes should succeed");

        assert_eq!(restored.width(), original.width());
        assert_eq!(restored.height(), original.height());
        for (pos, kind) in original.iter() {
            assert_eq!(restored.get(pos), Some(kind));
        }
    }

    #[test]
    fn test_tilemap_to_from_bytes_small() {
        let mut tilemap = Tilemap::new(3, 2, TileKind::Floor);
        tilemap.set(IVec2::new(0, 0), TileKind::Wall);
        tilemap.set(IVec2::new(2, 1), TileKind::Wall);

        let bytes = tilemap.to_bytes().expect("to_bytes should succeed");
        let restored = Tilemap::from_bytes(&bytes).expect("from_bytes should succeed");

        assert_eq!(restored.width(), 3);
        assert_eq!(restored.height(), 2);
        assert_eq!(restored.get(IVec2::new(0, 0)), Some(TileKind::Wall));
        assert_eq!(restored.get(IVec2::new(1, 0)), Some(TileKind::Floor));
        assert_eq!(restored.get(IVec2::new(2, 1)), Some(TileKind::Wall));
    }

    #[test]
    fn test_from_bytes_invalid() {
        let result = Tilemap::from_bytes(&[0xFF, 0x00]);
        assert!(result.is_err());
    }

    #[test]
    fn test_try_from_dimension_overflow() {
        let msg = TilesStreamMessage::TilemapData {
            width: u32::MAX,
            height: 2,
            tiles: vec![TileKind::Floor; 4],
        };
        let result = Tilemap::try_from(msg);
        assert!(result.is_err());
        assert!(
            result.unwrap_err().contains("overflow"),
            "error should mention overflow"
        );
    }

    #[test]
    fn test_try_from_tile_mutated_returns_error() {
        let msg = TilesStreamMessage::TileMutated {
            position: [2, 3],
            kind: TileKind::Floor,
        };
        let result = Tilemap::try_from(msg);
        assert!(result.is_err(), "TileMutated should not convert to a Tilemap");
    }

    #[test]
    fn test_tile_toggle_roundtrip() {
        // TileToggle must survive a wincode encode→decode cycle.
        let original = TileToggle {
            position: [3, 7],
            kind: TileKind::Wall,
        };
        let bytes = wincode::serialize(&original).expect("encode should succeed");
        let restored: TileToggle = wincode::deserialize(&bytes).expect("decode should succeed");
        assert_eq!(restored.position, original.position);
        assert_eq!(restored.kind, TileKind::Wall);
    }

    #[test]
    fn test_tile_mutated_message_roundtrip() {
        let msg = TilesStreamMessage::TileMutated {
            position: [5, 2],
            kind: TileKind::Floor,
        };
        let bytes = wincode::serialize(&msg).expect("encode should succeed");
        let restored: TilesStreamMessage =
            wincode::deserialize(&bytes).expect("decode should succeed");
        match restored {
            TilesStreamMessage::TileMutated { position, kind } => {
                assert_eq!(position, [5, 2]);
                assert_eq!(kind, TileKind::Floor);
            }
            other => panic!("unexpected variant: {:?}", other),
        }
    }
}
