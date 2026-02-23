use bevy::prelude::*;
use network::{StreamDef, StreamDirection, StreamRegistry, StreamSender};
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

    /// Creates a 12x10 test room with perimeter walls and internal obstacles.
    pub fn test_room() -> Tilemap {
        let mut tilemap = Tilemap::new(12, 10, TileKind::Floor);

        // Perimeter walls
        for x in 0..12 {
            tilemap.set(IVec2::new(x, 0), TileKind::Wall);
            tilemap.set(IVec2::new(x, 9), TileKind::Wall);
        }
        for y in 0..10 {
            tilemap.set(IVec2::new(0, y), TileKind::Wall);
            tilemap.set(IVec2::new(11, y), TileKind::Wall);
        }

        // Internal walls for collision testing
        // Vertical wall segment
        for y in 2..6 {
            tilemap.set(IVec2::new(4, y), TileKind::Wall);
        }
        // Horizontal wall segment
        for x in 7..10 {
            tilemap.set(IVec2::new(x, 5), TileKind::Wall);
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
    TilemapData { width: u32, height: u32, tiles: Vec<TileKind> },
}

/// Stream tag for the server→client tiles stream (stream 1).
pub const TILES_STREAM_TAG: u8 = 1;

/// Decode a [`TilesStreamMessage`] from raw stream-frame bytes.
pub fn decode_tiles_message(bytes: &[u8]) -> Result<TilesStreamMessage, String> {
    wincode::deserialize(bytes).map_err(|e| e.to_string())
}

impl Tilemap {
    /// Serialize the tilemap to bytes using the `TilesStreamMessage::TilemapData` wire format.
    pub fn to_bytes(&self) -> Vec<u8> {
        wincode::serialize(&self.to_stream_message()).unwrap_or_else(|e| {
            panic!(
                "Failed to serialize Tilemap ({}×{}): {e}",
                self.width, self.height
            )
        })
    }

    /// Build the stream 1 wire message for this tilemap.
    pub fn to_stream_message(&self) -> TilesStreamMessage {
        TilesStreamMessage::TilemapData {
            width: self.width,
            height: self.height,
            tiles: self.tiles.clone(),
        }
    }

    /// Construct a [`Tilemap`] from a decoded [`TilesStreamMessage`].
    pub fn from_stream_message(msg: TilesStreamMessage) -> Self {
        match msg {
            TilesStreamMessage::TilemapData { width, height, tiles } => {
                Tilemap { width, height, tiles }
            }
        }
    }

    /// Deserialize a tilemap from bytes produced by [`Tilemap::to_bytes`].
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, String> {
        decode_tiles_message(bytes).map(Tilemap::from_stream_message)
    }
}

pub struct TilesPlugin;

impl Plugin for TilesPlugin {
    fn build(&self, app: &mut App) {
        app.register_type::<TileKind>();
        app.register_type::<Tilemap>();
        app.register_type::<Tile>();
        app.init_resource::<TileMeshes>();
        app.add_systems(Update, spawn_tile_meshes);

        // Register stream 1 (server→client tiles stream) if NetworkPlugin is present.
        if let Some(mut registry) = app.world_mut().get_resource_mut::<StreamRegistry>() {
            let sender: StreamSender<TilesStreamMessage> = registry.register(StreamDef {
                tag: TILES_STREAM_TAG,
                name: "tiles",
                direction: StreamDirection::ServerToClient,
            });
            app.insert_resource(sender);
        }
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

    // Only spawn if tilemap was just added or changed
    if !tilemap.is_changed() {
        return;
    }

    // Despawn existing tile entities
    for entity in &existing_tiles {
        commands.entity(entity).despawn();
    }

    // Spawn tile entities
    for (pos, kind) in tilemap.iter() {
        let world_x = pos.x as f32;
        let world_z = pos.y as f32;

        match kind {
            TileKind::Floor => {
                // Collider is 0.1 tall (full dim), centered on transform.
                // Offset y by -0.05 so the top surface sits at y=0.0.
                commands.spawn((
                    Mesh3d(tile_meshes.floor_mesh.clone()),
                    MeshMaterial3d(tile_meshes.floor_material.clone()),
                    Transform::from_xyz(world_x, -0.05, world_z),
                    Tile { position: pos },
                    RigidBody::Static,
                    Collider::cuboid(1.0, 0.1, 1.0),
                ));
            }
            TileKind::Wall => {
                commands.spawn((
                    Mesh3d(tile_meshes.wall_mesh.clone()),
                    MeshMaterial3d(tile_meshes.wall_material.clone()),
                    Transform::from_xyz(world_x, 0.5, world_z),
                    Tile { position: pos },
                    RigidBody::Static,
                    // avian3d Collider::cuboid takes full dimensions, not half-extents
                    Collider::cuboid(1.0, 1.0, 1.0),
                ));
            }
        }
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
        assert_eq!(room.width(), 12);
        assert_eq!(room.height(), 10);

        // Perimeter should be walls
        for x in 0..12 {
            assert_eq!(room.get(IVec2::new(x, 0)), Some(TileKind::Wall));
            assert_eq!(room.get(IVec2::new(x, 9)), Some(TileKind::Wall));
        }
        for y in 0..10 {
            assert_eq!(room.get(IVec2::new(0, y)), Some(TileKind::Wall));
            assert_eq!(room.get(IVec2::new(11, y)), Some(TileKind::Wall));
        }

        // Interior should be floor (spot check)
        assert_eq!(room.get(IVec2::new(5, 5)), Some(TileKind::Floor));
        assert_eq!(room.get(IVec2::new(6, 3)), Some(TileKind::Floor));

        // Internal walls
        assert_eq!(room.get(IVec2::new(4, 3)), Some(TileKind::Wall));
        assert_eq!(room.get(IVec2::new(8, 5)), Some(TileKind::Wall));
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
        let bytes = original.to_bytes();
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

        let bytes = tilemap.to_bytes();
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
}
