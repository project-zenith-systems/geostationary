use std::collections::{BTreeMap, HashMap};

use base64::Engine as _;
use bevy::prelude::*;
use input::{PointerAction, WorldHit};
use serde::{Deserialize, Serialize};
use world::{MapLayer, MapLayerRegistryExt, from_layer_value, to_layer_value};
use network::{
    ClientId, Headless, ModuleReadySent, NetworkSet, PlayerEvent, Server, StreamDef,
    StreamDirection, StreamReader, StreamRegistry, StreamSender,
};
use physics::{Collider, RigidBody};
use wincode::{SchemaRead, SchemaWrite};

/// System set for the tiles module's server-side lifecycle systems.
/// Other modules can use this for explicit ordering relative to tiles systems.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub enum TilesSet {
    /// Sends the full tilemap snapshot and [`StreamReady`] sentinel to a joining client.
    ///
    /// Runs in `PreUpdate` so that ordering constraints against other modules'
    /// on-connect sends (e.g. [`things::ThingsSet::HandleClientJoined`]) can be
    /// expressed within the same schedule.  If the [`Tilemap`] resource is not yet
    /// available the client is queued in [`PendingTilesSyncs`] and retried each frame.
    SendOnConnect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Reflect, SchemaRead, SchemaWrite, Serialize, Deserialize)]
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

// ---------------------------------------------------------------------------
// Map layer data types (on-disk format for the "tiles" layer)
// ---------------------------------------------------------------------------

/// On-disk representation of the `"tiles"` map layer.
///
/// Uses key-dictionary deduplication + base64-encoded chunk blobs.
/// See `docs/map-format.md` for the full format description.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TilesLayerData {
    pub chunk_size: u32,
    /// Maps u16 key → tile definition.  Only configurations that actually
    /// appear in the map are stored.
    pub keys: BTreeMap<u16, TileDef>,
    /// Maps (chunk_x, chunk_y) → base64-encoded u16 key array.
    /// Each chunk stores `chunk_size * chunk_size` u16 values in
    /// row-major order (y * chunk_size + x), encoded as little-endian bytes.
    pub chunks: BTreeMap<(i32, i32), String>,
}

/// Per-tile configuration stored in the key dictionary.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TileDef {
    pub kind: TileKind,
    /// Atmosphere override for this tile.  `None` means the default
    /// (Pressurised for Floor, no atmos for Wall).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub atmosphere: Option<Atmo>,
}

/// Atmosphere state baked into a tile definition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Atmo {
    Pressurised,
    Vacuum,
}

// ---------------------------------------------------------------------------
// TilesLayer — MapLayer implementation
// ---------------------------------------------------------------------------

/// Chunk size used when saving a [`Tilemap`] to disk.
const SAVE_CHUNK_SIZE: u32 = 32;

/// `MapLayer` implementation for the `"tiles"` layer.
///
/// **load**: Decodes key-dictionary + base64 chunks → inserts [`Tilemap`] resource.
/// **save**: Reads [`Tilemap`] resource → builds key-dictionary + base64 chunks.
pub struct TilesLayer;

impl MapLayer for TilesLayer {
    fn key(&self) -> &'static str {
        "tiles"
    }

    fn load(
        &self,
        data: &ron::value::RawValue,
        world: &mut World,
    ) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        let layer: TilesLayerData = from_layer_value(data)?;

        let chunk_size = layer.chunk_size;
        if chunk_size == 0 {
            return Err("tiles layer: chunk_size must be greater than 0".into());
        }

        // Compute expected bytes per chunk with overflow protection.
        let tiles_per_chunk = (chunk_size as usize)
            .checked_mul(chunk_size as usize)
            .ok_or("tiles layer: chunk_size is too large (chunk_size² overflows usize)")?;
        let expected_bytes = tiles_per_chunk
            .checked_mul(2)
            .ok_or("tiles layer: chunk_size is too large (chunk_size² × 2 overflows usize)")?;

        // Handle the empty-map case before doing any chunk work.
        if layer.chunks.is_empty() {
            world.insert_resource(Tilemap::new(0, 0, TileKind::Floor));
            return Ok(());
        }

        // Derive map dimensions from chunk indices without decoding any tile data.
        // Reject negative chunk coordinates here so the subsequent u32 casts are safe.
        let mut max_chunk_x: i32 = i32::MIN;
        let mut max_chunk_y: i32 = i32::MIN;
        for &(cx, cy) in layer.chunks.keys() {
            if cx < 0 || cy < 0 {
                return Err(format!(
                    "tiles layer: negative chunk coordinate ({cx},{cy}) is not supported"
                )
                .into());
            }
            max_chunk_x = max_chunk_x.max(cx);
            max_chunk_y = max_chunk_y.max(cy);
        }

        let width = (max_chunk_x as u32)
            .checked_add(1)
            .and_then(|v| v.checked_mul(chunk_size))
            .ok_or("tiles layer: map dimensions overflow u32")?;
        let height = (max_chunk_y as u32)
            .checked_add(1)
            .and_then(|v| v.checked_mul(chunk_size))
            .ok_or("tiles layer: map dimensions overflow u32")?;

        let mut tilemap = Tilemap::new(width, height, TileKind::Floor);

        // Decode each chunk and write directly into the tilemap.
        for (&(chunk_x, chunk_y), b64) in &layer.chunks {
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|e| {
                    format!("tiles layer: chunk ({chunk_x},{chunk_y}) base64 decode failed: {e}")
                })?;

            if bytes.len() != expected_bytes {
                return Err(format!(
                    "tiles layer: chunk ({chunk_x},{chunk_y}) has {} bytes, expected {expected_bytes} \
                     (chunk_size={chunk_size})",
                    bytes.len()
                )
                .into());
            }

            for local_y in 0..chunk_size {
                for local_x in 0..chunk_size {
                    // Compute the byte offset in usize arithmetic to avoid u32 overflow
                    // for large (but still valid) chunk sizes.
                    let offset =
                        (local_y as usize * chunk_size as usize + local_x as usize) * 2;
                    let key = u16::from_le_bytes([bytes[offset], bytes[offset + 1]]);

                    let tile_def = layer.keys.get(&key).ok_or_else(|| {
                        format!(
                            "tiles layer: chunk ({chunk_x},{chunk_y}) references unknown key {key}"
                        )
                    })?;

                    let global_x = chunk_x * chunk_size as i32 + local_x as i32;
                    let global_y = chunk_y * chunk_size as i32 + local_y as i32;
                    tilemap.set(IVec2::new(global_x, global_y), tile_def.kind);
                }
            }
        }

        world.insert_resource(tilemap);
        Ok(())
    }

    fn save(
        &self,
        world: &World,
    ) -> Result<Box<ron::value::RawValue>, Box<dyn std::error::Error + Send + Sync>> {
        let tilemap = world
            .get_resource::<Tilemap>()
            .ok_or("tiles layer: Tilemap resource not found")?;

        let chunk_size = SAVE_CHUNK_SIZE;
        let num_chunks_x = tilemap.width().div_ceil(chunk_size);
        let num_chunks_y = tilemap.height().div_ceil(chunk_size);
        let tiles_per_chunk = chunk_size as usize * chunk_size as usize;

        let mut key_dict: BTreeMap<u16, TileDef> = BTreeMap::new();
        let mut def_to_key: HashMap<TileDef, u16> = HashMap::new();
        let mut next_key: u16 = 0;
        let mut chunks: BTreeMap<(i32, i32), String> = BTreeMap::new();

        // Single pass: assign keys on first occurrence and encode each chunk in one loop.
        for chunk_y in 0..num_chunks_y {
            for chunk_x in 0..num_chunks_x {
                let mut buf: Vec<u8> = Vec::with_capacity(tiles_per_chunk * 2);
                for local_y in 0..chunk_size {
                    for local_x in 0..chunk_size {
                        let pos = IVec2::new(
                            chunk_x as i32 * chunk_size as i32 + local_x as i32,
                            chunk_y as i32 * chunk_size as i32 + local_y as i32,
                        );
                        // Pad out-of-bounds positions with Floor (the Tilemap default fill).
                        let kind = tilemap.get(pos).unwrap_or(TileKind::Floor);
                        let def = TileDef { kind, atmosphere: None };
                        let key = match def_to_key.entry(def.clone()) {
                            std::collections::hash_map::Entry::Occupied(e) => *e.get(),
                            std::collections::hash_map::Entry::Vacant(e) => {
                                let k = next_key;
                                next_key = next_key.checked_add(1).ok_or(
                                    "tiles layer: too many unique tile configurations (>65535)",
                                )?;
                                key_dict.insert(k, def);
                                *e.insert(k)
                            }
                        };
                        buf.extend_from_slice(&key.to_le_bytes());
                    }
                }
                chunks.insert(
                    (chunk_x as i32, chunk_y as i32),
                    base64::engine::general_purpose::STANDARD.encode(&buf),
                );
            }
        }

        Ok(to_layer_value(&TilesLayerData {
            chunk_size,
            keys: key_dict,
            chunks,
        })?)
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
        app.register_map_layer(TilesLayer);
        app.register_type::<Tilemap>();
        app.register_type::<Tile>();

        app.add_message::<TileMutated>();

        // Register messages that raycast_tiles read/write
        // so the resources exist even when InputPlugin is not added (e.g. headless tests).
        app.add_message::<PointerAction>();
        app.add_message::<WorldHit>();

        let headless = app.world().contains_resource::<Headless>();
        if headless {
            // Headless server: spawn tile colliders (no meshes) so physics
            // entities don't fall through the floor.
            app.add_systems(Update, spawn_tile_colliders);
        } else {
            // Visual client / listen-server: spawn full tile entities with meshes + colliders.
            app.init_resource::<TileMeshes>();
            app.add_systems(Update, spawn_tile_meshes);
            // On a listen-server, TileMutated events are written by dispatch_interaction
            // (interactions module, Update schedule).  Running apply_tile_mutation in PostUpdate
            // guarantees it executes after dispatch_interaction has written the events.
            app.add_systems(
                PostUpdate,
                apply_tile_mutation.run_if(resource_exists::<Server>),
            );
            // On a dedicated client, TileMutated events come from handle_tiles_stream
            // (PreUpdate), so no intra-Update ordering is needed.
            app.add_systems(
                Update,
                apply_tile_mutation.run_if(not(resource_exists::<Server>)),
            );
            app.add_systems(
                Update,
                raycast_tiles,
            );
        }

        app.add_systems(
            PreUpdate,
            handle_tiles_stream
                .run_if(not(resource_exists::<Server>))
                .after(NetworkSet::Receive),
        );
        // Runs in PreUpdate after NetworkSet::Receive so PlayerEvent::Joined is
        // readable.  If the Tilemap resource is not yet available (e.g.
        // listen-server: setup_world hasn't run yet) the client is queued in
        // PendingTilesSyncs and retried each frame.
        app.init_resource::<PendingTilesSyncs>();
        app.configure_sets(
            PreUpdate,
            TilesSet::SendOnConnect
                .after(NetworkSet::Receive)
                .before(NetworkSet::Send),
        );
        app.add_systems(
            PreUpdate,
            send_tilemap_on_connect
                .run_if(resource_exists::<Server>)
                .in_set(TilesSet::SendOnConnect),
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
        app.insert_resource(sender);
        app.insert_resource(reader);
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

/// Headless-server variant of [`spawn_tile_meshes`]: spawns tile colliders
/// (no meshes or materials) so physics entities have a floor to stand on.
fn spawn_tile_colliders(
    mut commands: Commands,
    tilemap: Option<Res<Tilemap>>,
    existing_tiles: Query<Entity, With<Tile>>,
) {
    let Some(tilemap) = tilemap else {
        for entity in &existing_tiles {
            commands.entity(entity).despawn();
        }
        return;
    };

    if !existing_tiles.is_empty() {
        return;
    }

    for (pos, kind) in tilemap.iter() {
        let world_x = pos.x as f32;
        let world_z = pos.y as f32;
        match kind {
            TileKind::Floor => {
                commands.spawn((
                    Transform::from_xyz(world_x, -0.05, world_z),
                    Tile { position: pos },
                    RigidBody::Static,
                    Collider::cuboid(1.0, 0.1, 1.0),
                ));
            }
            TileKind::Wall => {
                commands.spawn((
                    Transform::from_xyz(world_x, 0.5, world_z),
                    Tile { position: pos },
                    RigidBody::Static,
                    Collider::cuboid(1.0, 1.0, 1.0),
                ));
            }
        }
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

/// Client-side system that handles [`TileMutated`] events (fired by both
/// [`handle_tiles_stream`] and, on listen-servers, by `dispatch_interaction` in the
/// interactions module).
///
/// Despawns the existing tile entity at the affected grid position and spawns a new
/// one with the updated mesh, material, and collider via [`spawn_tile_entity`].
/// This provides incremental rendering — only the changed tile is rebuilt.
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
        let mut original = Tilemap::new(16, 10, TileKind::Floor);
        for x in 0..16 {
            original.set(IVec2::new(x, 0), TileKind::Wall);
            original.set(IVec2::new(x, 9), TileKind::Wall);
        }
        for y in 0..10 {
            original.set(IVec2::new(0, y), TileKind::Wall);
            original.set(IVec2::new(15, y), TileKind::Wall);
        }
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

    // ---------------------------------------------------------------------------
    // TilesLayer (MapLayer) tests
    // ---------------------------------------------------------------------------

    /// Helper: build a minimal World with the resources needed by TilesLayer::save.
    fn world_with_tilemap(tilemap: Tilemap) -> World {
        let mut world = World::new();
        world.insert_resource(tilemap);
        world
    }

    /// A 2×2 tilemap survives a save→load round-trip through TilesLayer.
    #[test]
    fn tiles_layer_roundtrip_small() {
        let mut original = Tilemap::new(2, 2, TileKind::Floor);
        original.set(IVec2::new(0, 0), TileKind::Wall);
        original.set(IVec2::new(1, 1), TileKind::Wall);

        let world = world_with_tilemap(original.clone());
        let raw = TilesLayer.save(&world).expect("save must succeed");

        let mut load_world = World::new();
        TilesLayer
            .load(&raw, &mut load_world)
            .expect("load must succeed");

        let loaded = load_world
            .get_resource::<Tilemap>()
            .expect("Tilemap resource must be present after load");

        // The loaded map must cover at least the original 2×2 extent (it may
        // be padded to the nearest chunk boundary).
        for (pos, kind) in original.iter() {
            assert_eq!(
                loaded.get(pos),
                Some(kind),
                "tile at {pos:?} must match after round-trip"
            );
        }
    }

    /// A tilemap with only Floor tiles serializes to a single key and
    /// round-trips correctly.
    #[test]
    fn tiles_layer_all_floor() {
        let original = Tilemap::new(4, 4, TileKind::Floor);
        let world = world_with_tilemap(original.clone());

        let raw = TilesLayer.save(&world).expect("save must succeed");
        let data: TilesLayerData =
            world::from_layer_value(&raw).expect("TilesLayerData must deserialize");
        assert_eq!(data.keys.len(), 1, "only one unique tile def (Floor)");
        assert_eq!(data.chunk_size, SAVE_CHUNK_SIZE);

        let mut load_world = World::new();
        TilesLayer.load(&raw, &mut load_world).expect("load must succeed");
        let loaded = load_world.resource::<Tilemap>();
        for (pos, kind) in original.iter() {
            assert_eq!(loaded.get(pos), Some(kind));
        }
    }

    /// A tilemap with Floor and Wall tiles produces exactly two keys.
    #[test]
    fn tiles_layer_two_kinds_produce_two_keys() {
        let mut original = Tilemap::new(3, 3, TileKind::Floor);
        original.set(IVec2::new(0, 0), TileKind::Wall);

        let world = world_with_tilemap(original);
        let raw = TilesLayer.save(&world).expect("save must succeed");
        let data: TilesLayerData =
            world::from_layer_value(&raw).expect("deserialize");
        assert_eq!(data.keys.len(), 2, "Floor and Wall → two keys");
    }

    /// Saving when no Tilemap resource is present returns an error.
    #[test]
    fn tiles_layer_save_without_tilemap_errors() {
        let world = World::new();
        let result = TilesLayer.save(&world);
        assert!(result.is_err(), "save without Tilemap must fail");
    }

    /// Loading corrupt base64 returns an error.
    #[test]
    fn tiles_layer_load_invalid_base64_errors() {
        use std::collections::BTreeMap;

        let data = TilesLayerData {
            chunk_size: 4,
            keys: {
                let mut m = BTreeMap::new();
                m.insert(0, TileDef { kind: TileKind::Floor, atmosphere: None });
                m
            },
            chunks: {
                let mut m = BTreeMap::new();
                m.insert((0, 0), "!!!not-valid-base64!!!".to_owned());
                m
            },
        };
        let raw = world::to_layer_value(&data).expect("serialize");
        let mut w = World::new();
        let result = TilesLayer.load(&raw, &mut w);
        assert!(result.is_err(), "corrupt base64 must cause a load error");
    }

    /// Loading a chunk with the wrong byte count returns an error.
    #[test]
    fn tiles_layer_load_wrong_chunk_length_errors() {
        use base64::Engine as _;
        use std::collections::BTreeMap;

        let data = TilesLayerData {
            chunk_size: 4,
            keys: {
                let mut m = BTreeMap::new();
                m.insert(0, TileDef { kind: TileKind::Floor, atmosphere: None });
                m
            },
            chunks: {
                let mut m = BTreeMap::new();
                // 4×4 chunk needs 32 bytes; give it 2
                let short = base64::engine::general_purpose::STANDARD.encode([0u8, 0u8]);
                m.insert((0, 0), short);
                m
            },
        };
        let raw = world::to_layer_value(&data).expect("serialize");
        let mut w = World::new();
        let result = TilesLayer.load(&raw, &mut w);
        assert!(result.is_err(), "wrong byte count must cause a load error");
    }

    /// Loading a chunk that references an unknown key returns an error.
    #[test]
    fn tiles_layer_load_unknown_key_errors() {
        use base64::Engine as _;
        use std::collections::BTreeMap;

        let chunk_size: u32 = 2;
        let tiles = chunk_size * chunk_size;
        // Encode all tiles with key=5, which is not in the dictionary.
        let bytes: Vec<u8> = (0..tiles).flat_map(|_| 5u16.to_le_bytes()).collect();
        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);

        let data = TilesLayerData {
            chunk_size,
            keys: {
                let mut m = BTreeMap::new();
                m.insert(0, TileDef { kind: TileKind::Floor, atmosphere: None });
                m
            },
            chunks: {
                let mut m = BTreeMap::new();
                m.insert((0, 0), encoded);
                m
            },
        };
        let raw = world::to_layer_value(&data).expect("serialize");
        let mut w = World::new();
        let result = TilesLayer.load(&raw, &mut w);
        assert!(result.is_err(), "unknown key must cause a load error");
    }

    /// An empty chunks map loads as an empty Tilemap (0×0).
    #[test]
    fn tiles_layer_load_empty_chunks_gives_empty_tilemap() {
        use std::collections::BTreeMap;

        let data = TilesLayerData {
            chunk_size: 32,
            keys: BTreeMap::new(),
            chunks: BTreeMap::new(),
        };
        let raw = world::to_layer_value(&data).expect("serialize");
        let mut w = World::new();
        TilesLayer.load(&raw, &mut w).expect("empty map must load without error");
        let tm = w.resource::<Tilemap>();
        assert_eq!(tm.width(), 0);
        assert_eq!(tm.height(), 0);
    }

    /// chunk_size: 0 is rejected with an error.
    #[test]
    fn tiles_layer_load_zero_chunk_size_errors() {
        use std::collections::BTreeMap;

        let data = TilesLayerData {
            chunk_size: 0,
            keys: BTreeMap::new(),
            chunks: BTreeMap::new(),
        };
        let raw = world::to_layer_value(&data).expect("serialize");
        let mut w = World::new();
        let result = TilesLayer.load(&raw, &mut w);
        assert!(result.is_err(), "chunk_size 0 must be rejected");
    }

    /// A chunk with a negative x coordinate is rejected with an error.
    #[test]
    fn tiles_layer_load_negative_chunk_coord_errors() {
        use base64::Engine as _;
        use std::collections::BTreeMap;

        let chunk_size: u32 = 2;
        let tile_count = (chunk_size * chunk_size) as usize;
        let bytes: Vec<u8> = (0..tile_count).flat_map(|_| 0u16.to_le_bytes()).collect();
        let encoded = base64::engine::general_purpose::STANDARD.encode(&bytes);

        let data = TilesLayerData {
            chunk_size,
            keys: {
                let mut m = BTreeMap::new();
                m.insert(0, TileDef { kind: TileKind::Floor, atmosphere: None });
                m
            },
            chunks: {
                let mut m = BTreeMap::new();
                // chunk_x = -1 is negative
                m.insert((-1, 0), encoded);
                m
            },
        };
        let raw = world::to_layer_value(&data).expect("serialize");
        let mut w = World::new();
        let result = TilesLayer.load(&raw, &mut w);
        assert!(
            result.is_err(),
            "negative chunk coordinate must cause a load error"
        );
        let msg = result.unwrap_err().to_string();
        assert!(
            msg.contains("negative chunk coordinate"),
            "error message should mention 'negative chunk coordinate', got: {msg}"
        );
    }
}
