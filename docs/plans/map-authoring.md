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
each layer. Unknown layers round-trip through the raw `RawValue` store.

### Dependencies

The `world` module requires `ron` as a direct dependency for `RawValue`.
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

**Status: Complete.** Answers are embedded in `bins/client/src/editor/mod.rs`
(module doc comments) and `bins/client/src/editor/grid.rs` (unit tests).

1. **Can `AppState::Editor` coexist with `MainMenu` / `InGame`?**
   **Yes.** `DespawnOnExit` is per-entity/per-state with no cross-state
   interference. Main-menu entities carry `DespawnOnExit(AppState::MainMenu)`
   and clean up on `MainMenu` exit regardless of destination. The
   `on_net_id_added` observer (hardcoded to `InGame`) never fires in the
   editor since there is no active network connection. Tile entities (spawned
   by the state-agnostic `spawn_tile_meshes`) are cleaned up explicitly via
   `teardown_editor_world` on `OnExit(AppState::Editor)`.

2. **Does orthographic camera + XZ-plane raycasting work?**
   **Yes.** A `Camera3d` with `Projection::Orthographic` looking straight
   down `-Y` produces parallel rays that always intersect the y=0 plane.
   `ray_to_grid_cell` uses the same `round()` convention and `1e-4`
   threshold as `raycast_tiles` in the tiles module — six unit tests
   validate boundaries, negative coords, and degenerate rays. See
   `bins/client/src/editor/grid.rs`.

3. **Can tile entities from the game's rendering path be reused?**
   **Yes — unchanged.** Inserting `Tilemap` on `OnEnter(AppState::Editor)`
   is sufficient. `TilesPlugin::spawn_tile_meshes` picks it up automatically
   and spawns full mesh + collider entities. No editor-specific spawn logic
   is required.

**Plan impact:** None. All spike answers confirm the planned design. The
editor uses `AppState::Editor` with `DespawnOnExit`, an orthographic
top-down camera, and the game's existing tile rendering path.

## Post-mortem

### Outcome

The plan delivered all 15 observable outcomes. Station layouts are now data files
(`assets/maps/default.station.ron`, `test_room.station.ron`), the `world` module
at L0 provides `MapFile`, `MapLayer` trait, and `MapLayerRegistry`, and the
server loads maps via `config.toml`. The editor is fully functional with tile
painting, spawn placement, and save/load. `world_setup.rs` and
`Tilemap::test_room()` are gone. The work was completed across 27 commits on
`plan/map-authoring`, touching 63 files with +6,166/-1,707 lines changed.

Beyond the core plan, the implementation solved the "tile data extensibility"
problem that was explicitly deferred, made atmosphere a separate `MapLayer`
(contradicting Spike 1 Q5), and added a perspective orbit camera instead of the
planned orthographic top-down view.

### What shipped beyond the plan

| Addition | Why |
|----------|-----|
| `TileGrid<T>` generic system | The plan deferred "tile data extensibility" but the refactor was needed to cleanly separate tile kind from atmosphere data. `TileGrid<TileKind>` replaces monolithic `Tilemap`; new domains add `TileGrid<NewType>` without coupling. |
| `AtmosLayer` as separate `MapLayer` | Spike 1 Q5 recommended keeping atmosphere in `TileDef`. During implementation, region-based atmosphere (vacuum rectangles instead of per-tile flags) proved cleaner for authoring and more compact in the file format. |
| `load_default()` method on `MapLayer` | Layers not present in the map file need initialization. `AtmosLayer` uses this to set all tiles pressurised when the "atmosphere" key is absent. |
| `contents` property on spawn points | Enables pre-loading container contents (e.g., toolbox spawns with items inside). Required new `register_contents_property` in items module. |
| Perspective orbit camera | Plan said "orthographic top-down" and "3D perspective in the editor" was in "Not in this plan". Implemented perspective with orbit controls (pan, zoom, Q/E rotation) for better spatial understanding while authoring. |
| `main_menu` promoted to module crate | Decoupled from `bins/shared` to break a dependency cycle. `MainMenuPlugin<S>` is now generic over state. |
| Network orchestration moved to `network` module | `orchestrate.rs` consolidates client/server sync barriers, previously scattered across `bins/client/client.rs` and `bins/shared/server.rs`. |
| Loading state infrastructure | `MapLoaded` marker, loading state management in `WorldPlugin`, `AtmosphericsPlugin`, `TilesPlugin`. Required for headless server and network sync. |
| Headless vs visual tile spawning | `TilesPlugin` now conditionally spawns meshes (visual) or data-only (headless) based on plugin configuration. |

### Deviations from plan

- **Perspective camera instead of orthographic.** Plan stated "orthographic
  top-down camera" and explicitly excluded "3D perspective in the editor". The
  implementation uses a `Camera3d` with perspective projection and orbit
  controls (`EditorOrbit` resource with focus point, distance, pitch, yaw).
  Spike 2 validated orthographic raycasting but the team opted for perspective
  during editor implementation for better depth perception.

- **Atmosphere is a separate layer, not in `TileDef`.** Spike 1 Q5 concluded
  "keep in `TileDef`" with key-dictionary deduplication reasoning. The
  implementation instead created `AtmosLayer` with region-based vacuum
  specification (`Rect(min, max)` areas). This contradicts the spike finding
  but produces cleaner map files and simpler authoring — vacuum regions are
  explicit rectangles, not implicit from tile definitions.

- **`TileGrid<T>` shipped in this plan.** "Tile data extensibility" was in "Not
  in this plan" and explicitly deferred to the map-format doc. The refactor
  happened anyway because atmosphere separation required it. This is scope
  creep that paid off — the generic grid is cleaner than the deferred approach.

- **Editor camera in `bins/client/src/editor/camera.rs`, not `modules/camera`.**
  Plan said "L6 | `camera` | Editor camera: orthographic top-down, pan/zoom".
  The game camera module (`modules/camera`) was untouched beyond minor refactors;
  editor camera lives entirely in the editor submodule. This is reasonable
  separation — editor camera is editor-specific, not reusable.

- **Entity palette reads from `ThingRegistry::named_templates()`.** Plan said
  "Entity palette: click to place spawn markers (ball, can, toolbox)" implying
  hardcoded templates. Implementation queries the registry dynamically, which
  is better but required adding `named_templates()` iterator and
  `register_named()` method.

- **`main_menu` promoted to standalone crate.** Layer participation table showed
  `L1 | main_menu | Adds "Editor" button...` within `bins/client`. Actual
  implementation created `modules/main_menu/` as a workspace crate with
  `MainMenuPlugin<S>` generic over state type. Triggered by refactoring
  `client.rs` and `server.rs` into `orchestrate.rs`.

### Hurdles

1. **`ron::Value` loses unit enum variant names.** Spike 1 caught this before
   implementation. `Value::Unit` stores `()` without the variant identifier,
   breaking deserialization of enums like `TileKind::Wall`. Switched to
   `Box<ron::value::RawValue>` which preserves exact RON bytes. **Lesson:**
   Spikes work. The 30-minute investment prevented a structural redesign
   mid-implementation.

2. **`main_menu` hardcoded `AppState` instead of generic parameter.** After
   promoting to a module crate, `MainMenuPlugin<S>` was generic but
   `menu_message_reader` still referenced `AppState::Editor` directly. Required
   adding `MainMenuEditorState` resource and `editor_state` field. **Lesson:**
   When making a plugin generic, audit all system functions for concrete type
   usage.

3. **Atmosphere-in-TileDef produced awkward authoring.** Per-tile atmosphere
   flags meant every floor tile needed `(kind: Floor, atmo: Pressurised)` or
   default inference. Region-based vacuum rectangles in a separate layer proved
   more intuitive. **Lesson:** Spike conclusions about data layout do not always
   predict authoring ergonomics. The spike tested serialization fidelity, not
   usability.

4. **Loading state race conditions.** Early implementation had tile spawning
   systems running before map load completed. Added `MapLoaded` marker entity,
   `in_state(MapLoadState::Ready)` run conditions, and proper cleanup in
   `WorldPlugin`. **Lesson:** Map loading is async-ish (file I/O, layer
   dispatch) even without actual async. Systems must wait for completion.

5. **Editor UI blocking pointer events.** Clicks on palette UI were passing
   through to painting/spawning systems. Added `FocusPolicy::Block` and
   UI-hover guards checking `Interaction` component on palette root. **Lesson:**
   Bevy UI focus policy defaults need explicit blocking for overlay panels.

6. **Spawn marker Y position mismatch.** Initial implementation placed markers
   at `y = 0.5` (visual offset) but saved `y = 0.0` to disk. On reload, markers
   appeared at floor level, not above it. Fixed by consistent `y = 0.0` and
   letting render offset handle visibility. **Lesson:** Serialize the canonical
   position, not the visual position.

### What went well

- **Spikes prevented major rework.** Both spikes completed on time and their
  findings directly shaped implementation. The `RawValue` discovery alone saved
  a likely redesign of `MapFile.layers`.

- **MapLayer trait extensibility proved out.** Three layers registered
  (`tiles`, `spawns`, `atmosphere`) with zero changes to `world` module after
  initial implementation. `AtmosLayer` was added late in the plan and slotted
  in cleanly.

- **Unknown layer round-trip works.** Map files with layers the code does not
  recognize survive save/load unchanged. Tested by manually adding a `"future"`
  layer to map files.

- **Editor reuses game rendering.** Tile entities use the same `spawn_tile_meshes`
  system as the game. No duplicate mesh generation or material handling code.

- **Clean deletion of legacy code.** `world_setup.rs` (327 lines),
  `Tilemap::test_room()`, hardcoded item spawns — all removed with no fallout.
  Module boundaries made the cuts surgical.

- **PR review caught real issues.** The editor PR (#222) had four review
  rounds addressing: tiles_data cloning, palette resource persistence,
  spawn marker positioning, UI hover guards, and hardcoded template lists.

### What to do differently next time

- **Re-evaluate spike conclusions when authoring ergonomics emerge.** Spike 1
  Q5 was technically correct (atmosphere-in-TileDef is more compact) but
  region-based vacuum is more intuitive to author. Add "how will this feel to
  edit?" to spike criteria.

- **Decide camera projection definitively before coding.** The plan said
  orthographic, the implementation shipped perspective. Either update the plan
  when the decision changes or spike the actual authoring experience to inform
  the choice.

- **Scope "not in this plan" items more carefully.** `TileGrid<T>` was
  explicitly deferred but shipped anyway because atmosphere separation
  required it. The exclusion list should distinguish "cannot do yet" from
  "choose not to do yet."

- **Test loading state in isolation.** The loading race conditions emerged
  from manual testing, not automated tests. A test that loads a map and
  asserts `TileGrid` exists would have caught this earlier.

- **Document module promotion decisions.** `main_menu` became a crate for
  dependency reasons, not feature reasons. The rationale is not in any commit
  message — it should be.
