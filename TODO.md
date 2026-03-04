## Spike: MapLayer trait and ron::Value dispatch

Time-box: 30 minutes. Answer the questions in the plan's spike section before
committing to the MapLayer design.

1. Can `ron::Value` be deserialized into a concrete type in a second pass?
   (Parse `MapFile` with `HashMap<String, ron::Value>`, then deserialize
   individual values into `TilesLayerData` or `Vec<SpawnPoint>`.)
2. Does `ron::Value` round-trip with fidelity for unknown layers?
3. What context does `MapLayer::load()` need — `&mut Commands`, `&mut World`,
   or something else? Does it need read access to resources like
   `ThingRegistry`?
4. Can the same `MapLayer::save()` path work for both the running server and
   the editor, or do they need separate serialization logic?
5. Is atmosphere best kept in `TileDef` (benefits from key deduplication) or
   as a separate overlay grid layer owned by the atmos module?

Keep the spike code — its test assertions become regression tests.
If any answer invalidates the plan, update `docs/plans/map-authoring.md`
before continuing.

**Plan:** `plan/map-authoring` · [docs/plans/map-authoring.md](docs/plans/map-authoring.md)

## Spike: Editor app state and camera

Time-box: 30 minutes. Answer the questions in the plan's spike section before
building the editor.

1. Can `AppState::Editor` coexist with `MainMenu` / `InGame` without
   breaking `DespawnOnExit` cleanup?
2. Does orthographic camera + XZ-plane raycasting work for grid cell
   selection?
3. Can tile entities from the game's rendering path be reused in the editor,
   or does the editor need its own spawn logic?

Depends on: Spike: MapLayer trait and ron::Value dispatch (plan may change).

**Plan:** `plan/map-authoring` · [docs/plans/map-authoring.md](docs/plans/map-authoring.md)

## Create the world module (L0)

New crate at `modules/world/`. This is the L0 module that owns the map file
container, the `MapLayer` trait, and world lifecycle messages.

Files created:
- `modules/world/Cargo.toml`
- `modules/world/src/lib.rs` — `WorldPlugin`, `MapLayer` trait, `MapLayerRegistry` resource, `app.register_map_layer()` extension method
- `modules/world/src/map_file.rs` — `MapFile` struct with RON serde
- `modules/world/src/lifecycle.rs` — `WorldLoading`, `WorldReady`, `WorldTeardown` messages
- `modules/world/src/loader.rs` — `load_map` system: reads `.station.ron`, dispatches layers to registered `MapLayer` implementations

Concrete changes:
- `MapFile { version: u32, layers: HashMap<String, ron::Value> }` serializes/deserializes RON
- `MapLayerRegistry` stores `Box<dyn MapLayer>` keyed by layer name
- `load_map` iterates layers, calls `MapLayer::load()` for known keys, preserves unknown layers as raw `ron::Value`
- `WorldPlugin` reads the map file path from config and triggers loading on startup
- Add `ron` as an explicit workspace dependency in root `Cargo.toml`
- Add `world` to workspace members

Does not include: tile or spawn layer implementations (separate tasks), editor systems, or deletion of `world_setup.rs`.

Depends on: Both spikes completed.

**Plan:** `plan/map-authoring` · [docs/plans/map-authoring.md](docs/plans/map-authoring.md)

## Implement MapLayer for tiles

`modules/tiles` implements `MapLayer("tiles")` using the key-dictionary +
base64 chunk format documented in [docs/map-format.md](docs/map-format.md).

Files touched:
- `modules/tiles/src/lib.rs` — add `TilesLayer` struct implementing `MapLayer`
- `modules/tiles/src/lib.rs` — register via `app.register_map_layer(TilesLayer)` in `TilesPlugin::build()`
- `modules/tiles/Cargo.toml` — add dependency on `world` module, add `base64` crate

Concrete changes:
- `TilesLayerData`, `TileDef`, `Atmo` structs with RON serde
- `TilesLayer::load()` decodes key dictionary + base64 chunks and builds the `Tilemap` + tile entities
- `TilesLayer::save()` scans tile entities, builds key dictionary, encodes base64 chunks
- `Tilemap::test_room()` is removed — all tile creation goes through `MapLayer::load()`

Does not include: editor painting systems, default map files, or atmosphere initialisation changes.

Depends on: Create the world module (L0).

**Plan:** `plan/map-authoring` · [docs/plans/map-authoring.md](docs/plans/map-authoring.md)

## Implement MapLayer for spawns

`modules/things` implements `MapLayer("spawns")` for spawn points with
template names, as documented in [docs/map-format.md](docs/map-format.md).

Files touched:
- `modules/things/src/lib.rs` — add `SpawnsLayer` struct implementing `MapLayer`
- `modules/things/src/lib.rs` — register via `app.register_map_layer(SpawnsLayer)` in `ThingsPlugin::build()`
- `modules/things/Cargo.toml` — add dependency on `world` module

Concrete changes:
- `SpawnPoint { position: [f32; 3], template: String, contents: Vec<String> }` with RON serde
- `SpawnsLayer::load()` iterates spawn points and calls `ThingRegistry` to spawn each template at its position
- `SpawnsLayer::save()` queries spawn-marker entities and serializes their positions + template names

Does not include: editor spawn placement UI or default map files.

Depends on: Create the world module (L0).

**Plan:** `plan/map-authoring` · [docs/plans/map-authoring.md](docs/plans/map-authoring.md)

## Wire server startup to WorldPlugin and delete world_setup.rs

Replace the hardcoded test room with map-file-driven world loading. The
server reads a map file path from `config.toml` and `WorldPlugin` loads it.

Files touched:
- `bins/shared/src/world_setup.rs` — **deleted entirely**
- `bins/server/src/main.rs` (or equivalent) — remove `world_setup` system, add `WorldPlugin`
- `bins/client/src/main.rs` — add `WorldPlugin` (client needs lifecycle messages)
- Config files — add `map_file` path setting

Concrete changes:
- `world_setup.rs` deleted — all world state now originates from the map file
- Server startup calls `WorldPlugin` which reads the configured `.station.ron` and dispatches layers
- Hardcoded item spawns removed — items come from the spawns layer
- Existing multiplayer flows unchanged (clients still receive state via network replication)

Depends on: Implement MapLayer for tiles, Implement MapLayer for spawns.

**Plan:** `plan/map-authoring` · [docs/plans/map-authoring.md](docs/plans/map-authoring.md)

## Create default map files

Author the two map files that replace the hardcoded test room.

Files created:
- `assets/maps/default.station.ron` — 32×32 station with multiple rooms, corridors, and a vacuum section; spawn points for ball, can, toolbox
- `assets/maps/test_room.station.ron` — 16×10 layout reproducing the old `Tilemap::test_room()` for regression

Concrete changes:
- `default.station.ron` is the map the server loads by default
- `test_room.station.ron` matches the old 16×10 hardcoded room (same tile layout, same item positions) so existing behaviour can be verified
- `config.toml` default points to `assets/maps/default.station.ron`

Does not include: editor (maps can be hand-authored in RON or created by the editor task below).

Depends on: Wire server startup to WorldPlugin and delete world_setup.rs.

**Plan:** `plan/map-authoring` · [docs/plans/map-authoring.md](docs/plans/map-authoring.md)

## Build the map editor

Editor mode in the client, launchable from the main menu or `--editor` CLI
flag. The editor is a live Bevy world with simulation disabled.

Files created:
- `bins/client/src/editor/mod.rs` — `EditorPlugin`, `AppState::Editor` state
- `bins/client/src/editor/camera.rs` — orthographic top-down camera, pan/zoom
- `bins/client/src/editor/painting.rs` — tile painting (click/drag to paint Floor or Wall)
- `bins/client/src/editor/palette.rs` — tile palette and entity palette UI panels
- `bins/client/src/editor/spawns.rs` — spawn point placement and marker rendering
- `bins/client/src/editor/io.rs` — save/load `.station.ron` files via `MapLayer` trait

Files touched:
- `bins/client/src/main.rs` — register `EditorPlugin`, add `--editor` CLI flag
- Main menu UI — add "Editor" button that transitions to `AppState::Editor`

Concrete changes:
- `AppState::Editor` with run conditions that disable physics, atmospherics, interactions, and networking
- Orthographic top-down camera with pan (middle-click drag or WASD) and zoom (scroll)
- Tile palette: select Floor or Wall, left-click/drag to paint
- Entity palette: select a template (ball, can, toolbox), click a floor tile to place a spawn marker
- Spawn markers are visible overlays (e.g., colored icons), not simulated entities
- Right-click on a spawn marker deletes it
- Save: queries live world via `MapLayer::save()`, writes `.station.ron`
- Load: reads `.station.ron`, clears editor world, calls `MapLayer::load()` per layer
- Unknown layers preserved on round-trip (stored in `MapFile`, re-serialized on save)
- Single chunk (32×32) for this plan

Does not include: undo/redo, copy/paste, multi-select, 3D perspective, structure placement.

Depends on: Create default map files (so there's a file to load and test with).

**Plan:** `plan/map-authoring` · [docs/plans/map-authoring.md](docs/plans/map-authoring.md)
