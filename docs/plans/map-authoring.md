# Plan: Map Authoring & Loading

> **Stage goal:** Station layouts are data files, not code. An editor mode
> in the client lets a user paint tiles and place spawn points on a grid,
> then save the result as a `.station.ron` file. The server loads a map file
> on startup, and each module deserializes its own layer of data via a
> `MapLayer` trait. A new L0 `world` module provides the map file container,
> modular layer dispatch, and world lifecycle events. The hardcoded test room
> is replaced by a default map file. This is plan 1 of the Tangible Station
> arc.

## What "done" looks like

1. A `.station.ron` map file exists at `assets/maps/default.station.ron`
2. The server reads the map file path from `config.toml` and the `world`
   module loads it, dispatching each layer to its registered `MapLayer`
3. `Tilemap::test_room()`, `world_setup.rs`, and hardcoded item spawns are
   gone — all world state originates from the map file
4. The `world` module (L0) exposes `MapLayer`, `MapLayerRegistry`, and
   lifecycle messages; `tiles` and `things` each register a layer
5. Unknown layers in the map file are preserved on round-trip
6. The editor is launchable from the main menu or `--editor` CLI flag
7. The editor runs as a live Bevy world with simulation systems disabled
   (no physics, atmos, interactions, networking) — tile entities exist as
   real ECS entities with meshes, not a separate in-memory representation
8. Tile palette: click/drag to paint Floor or Wall on the grid
9. Entity palette: click to place spawn markers (ball, can, toolbox)
10. Spawn markers are visible overlays, not simulated entities
11. Single chunk (32x32) for this plan; format supports multiple chunks
12. Save/Load writes and reads `.station.ron` files
13. Default map is 32x32 with multiple rooms, corridors, and a vacuum section
14. `test_room.station.ron` reproduces the old 16x10 layout for regression
15. Existing multiplayer flows are unchanged

## Strategy

1. **Create the `world` module at L0** — map file container (`MapFile`),
   `MapLayer` trait, `MapLayerRegistry`, lifecycle messages, and file I/O.
   `world_setup.rs` is deleted; `WorldPlugin` owns the load-on-startup path.
2. **Implement `MapLayer` in `tiles` and `things`** — tiles owns the
   key-dictionary + chunk format; things owns spawn points with template
   names.
3. **Build the editor mode** in the client as `AppState::Editor`. The editor
   is a live Bevy world — tile entities are real ECS entities with meshes
   rendered by an orthographic top-down camera. Simulation plugins (physics,
   atmos, interactions, networking) are disabled via run conditions. The
   editor reads/writes the `MapLayer` data through the same trait the server
   uses.

The editor is deliberately minimal — no undo, no multi-select, no 3D
perspective. Future arc plans add structures, lighting, and art.

**Lessons from previous post-mortems:**
- Spike runtime behaviour assumptions before committing
- Plan system ordering in the layer participation table
- No new network streams needed — the server loader feeds existing systems

The map file format, tile layer encoding, and spawns layer encoding are
documented in [docs/map-format.md](../map-format.md).

### Layer participation

| Layer | Module | Plan scope |
|-------|--------|------------|
| L0 | **`world` (new)** | `MapFile`, `MapLayer` trait, `MapLayerRegistry`, lifecycle messages (`WorldLoading`/`WorldReady`/`WorldTeardown`), file I/O, load-on-startup system |
| L1 | `tiles` | Implements `MapLayer("tiles")`: key-dict + base64 chunk encode/decode, builds `Tilemap` on load. Removes `test_room()` |
| L1 | `main_menu` | Adds "Editor" button and routes to `AppState::Editor`; wires `--editor` CLI flag |
| L1 | `things` | Implements `MapLayer("spawns")`: spawn points with template names, drives `ThingRegistry` on load |
| L2 | `items` | No changes — item components added by existing template system |
| L6 | `camera` | Editor camera: orthographic top-down, pan/zoom |
| — | `bins/client/src/editor` | `AppState::Editor`, tile painting, palette UI, spawn placement, save/load |

### Not in this plan

- **Undo/redo** — repaint or reload
- **Copy/paste or selection tools** — paint one tile at a time (with drag)
- **Structure placement** — future plan; structures module registers its own
  `MapLayer` with zero changes to `world` or existing layers
- **Custom gas mixtures** — tiles are Pressurised or Vacuum only
- **Multi-floor / Z-levels**
- **Map validation** — basic sanity checks only
- **3D perspective in the editor**
- **Runtime map switching** — one map per server startup
- **Lighting placement** — deferred to "Tile art & lighting" plan
- **Trait-based editor UI discovery** — editor hardcodes tiles + spawns layers
- **Tile data extensibility** — `TileDef` currently includes atmosphere
  directly; a strategy for extending per-tile data without coupling tiles to
  higher modules is deferred (see [map-format.md](../map-format.md),
  "Extending tile data")

### Module placement

```
assets/maps/
  default.station.ron       # 32x32 station
  test_room.station.ron     # 16x10 regression map

modules/world/              # NEW — L0
  src/
    lib.rs                  # WorldPlugin, MapLayer, MapLayerRegistry
    map_file.rs             # MapFile container, RON serde
    lifecycle.rs            # WorldLoading/WorldReady/WorldTeardown
    loader.rs               # load_map system: read file, dispatch layers

modules/tiles/src/lib.rs    # impl MapLayer for TilesLayer
modules/things/src/lib.rs   # impl MapLayer for SpawnsLayer

bins/client/src/editor/
  mod.rs                    # EditorPlugin, AppState::Editor
  camera.rs                 # orthographic top-down, pan/zoom
  painting.rs               # tile painting systems
  palette.rs                # tile & entity palette UI
  spawns.rs                 # spawn point placement & markers
  io.rs                     # save/load .station.ron files
```

`WorldPlugin` handles map loading directly, triggered by the server's
startup sequence. `bins/shared/src/world_setup.rs` is deleted entirely.

### Editor design

The editor enters `AppState::Editor` from the main menu or `--editor` CLI.
It is a **live Bevy world** with an orthographic top-down camera — tile
entities are real ECS entities with meshes, reusing the same rendering the
game uses. Simulation systems are disabled via run conditions gated on
`AppState::Editor`:

- Physics: disabled (no gravity, no collision response)
- Atmospherics: disabled (no gas simulation, no pressure forces)
- Interactions: disabled (no context menus, no pickup/drop)
- Networking: disabled (no server, no client connection)

**Tile painting:** Left-click/drag paints the selected tile kind. The editor
mutates tile entities directly in the ECS and tracks changes for
serialization at save time.

**Spawn placement:** With a template selected, clicking a floor tile places
a spawn marker entity. Right-click deletes markers.

**Save/Load:** Save queries the live world via registered `MapLayer::save()`
implementations, collects all layers into a `MapFile`, and writes RON. Load
reads a `MapFile`, clears the editor world, and calls `MapLayer::load()` for
each layer. Unknown layers round-trip through the raw `ron::Value` store.

### Dependencies

The `world` module requires `ron` as a direct dependency for `ron::Value`.
Currently `ron` enters the workspace via the `config` crate — it should be
added as an explicit workspace dependency.

## Spike 1: MapLayer trait and ron::Value dispatch (30 min)

**Status: Complete.** Answers are embedded in `modules/world/src/map_file.rs`
as test assertions (regression tests).

1. **Can `ron::Value` be deserialized into a concrete type in a second pass?**
   **Partially — and not the right tool.** `ron::Value` is a lossy
   representation: unit enum variant identifiers (e.g., `floor`, `Wall`) are
   parsed to `Value::Unit` with the variant name discarded. Attempting
   `into_rust::<TileKind>()` on a `Value::Unit` fails with
   `InvalidValueForType`. **Fix:** use `ron::value::RawValue` (raw RON bytes)
   for layer storage. `RawValue::into_rust::<T>()` correctly reconstructs any
   type including those with unit enum variants. `MapFile.layers` is now
   `BTreeMap<String, Box<ron::value::RawValue>>`. See
   `spike_q1_raw_value_to_concrete_type_second_pass` and
   `spike_q1_ron_value_loses_unit_enum_variant_name`.

2. **Does `ron::Value` round-trip with fidelity for unknown layers?**
   `Box<ron::value::RawValue>` round-trips with **exact** syntactic fidelity
   — the raw RON bytes are stored and re-emitted unchanged. Unknown layers
   survive save/load cycles unmodified. See `spike_q2_unknown_layer_round_trips`.

3. **What context does `MapLayer::load()` need?**
   `&mut World` is sufficient. It allows reading any existing resource (e.g.,
   `ThingRegistry`) and inserting new ones (e.g., `Tilemap`) without
   intermediate buffering. `Commands` would require an extra `world.flush()`
   and adds no benefit at load time because simulation hasn't started.
   See `spike_q3_load_receives_mut_world_with_resource_access`.

4. **Can the same `MapLayer::save()` path work for both server and editor?**
   **Yes.** `save(&self, world: &World)` reads whatever live state the world
   holds. The server and editor both use the same Bevy `World` — the layer
   implementation is identical in both contexts, only the entity contents
   differ. See `spike_q4_same_save_path_for_server_and_editor`.

5. **Is atmosphere best kept in `TileDef` or as a separate overlay grid?**
   **Keep in `TileDef`.** Key-dictionary deduplication stores only unique
   tile configurations — a 32×32 map with Wall / Pressurised floor / Vacuum
   floor encodes exactly three dictionary entries. A separate overlay grid
   would add an entry per tile, growing the file and duplicating chunk
   infrastructure. Atmosphere is also always co-read with tile kind at load
   time, so there is no benefit to separating it at this scale. See
   `spike_q5_atmosphere_in_tile_def_serde_default`.

**Plan impact:** `MapFile.layers` type changes from
`BTreeMap<String, ron::Value>` to `BTreeMap<String, Box<ron::value::RawValue>>`.
`MapLayer::save()` returns `Box<RawValue>`; `MapLayer::load()` receives
`&RawValue`. `docs/map-format.md` updated accordingly. All other plan
decisions remain valid.

## Spike 2: Editor app state and camera (30 min)

Depends on Spike 1 (plan may change based on MapLayer findings).

1. Can `AppState::Editor` coexist with `MainMenu` / `InGame` without
   breaking `DespawnOnExit` cleanup?
2. Does orthographic camera + XZ-plane raycasting work for grid cell
   selection?
3. Can tile entities from the game's rendering path be reused in the editor,
   or does the editor need its own spawn logic?

## Post-mortem

*To be filled in after the plan ships.*
