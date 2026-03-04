# Map File Format

This document describes the `.station.ron` map file format used by
Geostationary. It is a reference for the file's on-disk structure, the
design decisions behind it, and the module-level trait that makes the format
extensible.

## Overview

A map file is a RON-serialized container with a version number and a
dictionary of **layers**. Each layer is a named blob of RON data owned by a
specific game module. The L0 `world` module handles the container; higher
modules handle their own layers via the `MapLayer` trait.

```ron
(
    version: 1,
    layers: {
        "tiles": { ... },
        "spawns": [ ... ],
    },
)
```

## Container type

```rust
#[derive(Serialize, Deserialize)]
pub struct MapFile {
    pub version: u32,
    pub layers: HashMap<String, ron::Value>,
}
```

The `layers` map stores raw `ron::Value` blobs keyed by layer name. On load,
the world module iterates the map, finds the registered `MapLayer` for each
key, and calls `load()` with the blob. **Unrecognized layers are kept as raw
values** — re-saving preserves them, so a map authored with a newer version
(that has `"structures"` or `"wiring"` layers) doesn't lose data when loaded
by an older build.

## MapLayer trait

Each module that contributes map data implements `MapLayer` and registers it
during plugin `build()`:

```rust
/// A named section of a map file. Each module registers one.
pub trait MapLayer: Send + Sync + 'static {
    /// Unique key for this layer in the file (e.g., "tiles", "spawns")
    fn key(&self) -> &'static str;

    /// Serialize this module's world state into a RON value for saving
    fn save(&self, world: &World) -> ron::Value;

    /// Deserialize a RON value and apply it to the world on load
    fn load(&self, data: &ron::Value, world: &mut World);
}
```

> **Note:** The exact trait signature — particularly what context `load()`
> and `save()` receive — is subject to the MapLayer spike. `load()` may
> need read access to resources (e.g., `ThingRegistry` for resolving
> template names). `save()` needs read access to the live world state.

Registration:

```rust
// In modules/tiles/src/lib.rs
app.register_map_layer(TilesLayer);

// In modules/things/src/lib.rs
app.register_map_layer(SpawnsLayer);
```

The `MapLayerRegistry` (a Bevy resource) holds all registered
implementations. The world module uses it to dispatch on load/save.

## Tiles layer (`"tiles"`)

Owned by `modules/tiles`. Uses **key-dictionary deduplication** inspired by
SS13's DMM/TGM format and **chunk-based storage** mirroring SS14.

### Design rationale

- The editor is the primary authoring tool, not a text editor. Human
  readability of the grid is not a goal.
- Key deduplication keeps the file small — a station with 65K tiles but only
  50 unique tile configurations stores 50 dictionary entries plus compact
  chunk blobs.
- Chunk-based storage means a tile edit only changes the chunk it belongs to,
  keeping diffs localised. Future plans can use chunks for spatial
  partitioning (lazy loading, per-chunk dirty tracking).
- The key dictionary is the only part reviewers need to read in PRs ("what
  tile configurations exist?").

### Chunk model

Chunks are square, fixed-size sections of the grid indexed by integer chunk
coordinates `(chunk_x, chunk_y)`. These are **chunk indices**, not tile
coordinates: each chunk covers local tile positions `(x, y)` where
`0 <= x < chunk_size` and `0 <= y < chunk_size`. Global tile coordinates
are derived as:

- `global_x = chunk_x * chunk_size + x`
- `global_y = chunk_y * chunk_size + y`

Each chunk stores `chunk_size²` tile keys as base64-encoded u16 values
(little-endian). Map dimensions in tiles are derived from the set of chunk
indices and `chunk_size` — there are no explicit width/height fields.

### On-disk format

```ron
"tiles": {
    chunk_size: 32,
    keys: {
        0: (kind: Wall),
        1: (kind: Floor),
        2: (kind: Floor, atmosphere: Vacuum),
    },
    chunks: {
        (0, 0): "AAAAAAAAAAAAAAAAAA...==",
    },
},
```

### Rust types

```rust
#[derive(Serialize, Deserialize)]
pub struct TilesLayerData {
    pub chunk_size: u32,
    pub keys: HashMap<u16, TileDef>,
    pub chunks: HashMap<(i32, i32), String>,  // base64-encoded
}

#[derive(Serialize, Deserialize)]
pub struct TileDef {
    pub kind: TileKind,
    #[serde(default)]
    pub atmosphere: Option<Atmo>,
}

#[derive(Serialize, Deserialize)]
pub enum Atmo { Pressurised, Vacuum }
```

**Chunk decode:** `base64::decode(chunk)` → `chunks(2)` →
`u16::from_le_bytes` → look up in `keys` HashMap. Tile at local position
`(x, y)` within a chunk is at byte offset `(y * chunk_size + x) * 2`.

**Validation:** A conforming loader must validate decoded chunk data:

- The base64 decode must succeed.
- The decoded byte length must be even (each tile key is a u16).
- The decoded byte length must equal `chunk_size * chunk_size * 2` exactly.
- Every decoded u16 key must exist in the `keys` dictionary.

On any validation failure, the loader must treat the tiles layer as corrupt
and return an error for the map load — not silently substitute defaults or
partially decode.

**Key assignment:** The editor assigns keys automatically at save time. It
scans all tiles, groups identical configurations, assigns sequential u16 IDs
starting at 0, and encodes each chunk. Key numbering is deterministic
(sorted by first occurrence) so re-saving an unchanged map produces an
identical file.

### Extending tile data

Atmosphere is currently baked into `TileDef` because it's per-tile data that
benefits from key deduplication (a "vacuum floor" and a "pressurised floor"
are distinct keys). This keeps it simple but couples atmosphere to the tiles
layer.

As tile data grows (structures, connectables, pipe overlays), we need a
strategy for extending `TileDef` without the tiles module knowing about every
system. Options include:

- **Flat extension:** `TileDef` gains `#[serde(default)]` fields as modules
  need them. Simple but couples tiles to higher layers.
- **Per-tile extras map:** `TileDef` includes a `HashMap<String, ron::Value>`
  for module-contributed data. Decoupled but loses type safety and
  complicates deduplication.
- **Overlay grids:** Each module stores its own per-tile grid as a separate
  layer (e.g., `"atmos"` layer with its own chunk data). Fully decoupled,
  each module owns its grid, but duplicates chunk infrastructure.

The right answer depends on how many modules need per-tile data and how
important deduplication across those fields is. **This is deferred** — the
current plan ships with `atmosphere` in `TileDef` and the spike should
explore whether overlay grids or extras maps are viable.

## Spawns layer (`"spawns"`)

Owned by `modules/things`. A simple list of spawn points:

```rust
#[derive(Serialize, Deserialize)]
pub struct SpawnPoint {
    pub position: [f32; 3],
    pub template: String,        // ThingRegistry template name
    #[serde(default)]
    pub contents: Vec<String>,   // for containers: pre-loaded item templates
}
```

```ron
"spawns": [
    (position: (5.0, 0.0, 3.0), template: "toolbox", contents: ["can"]),
    (position: (4.0, 0.0, 3.0), template: "can"),
],
```

Template names (strings) keep the file decoupled from registry ordering.

## Atmosphere initialisation

Each `TileDef` carries an optional `atmosphere` field — `Pressurised`
(standard air mix at 101.3 kPa) or `Vacuum`. Floor tiles default to
`Pressurised` if the field is omitted. The server's atmos module reads these
flags when building the `GasGrid` after the tiles layer finishes loading.
