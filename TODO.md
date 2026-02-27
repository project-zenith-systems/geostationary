# TODO — Break a Wall, Watch Gas Rush Out

**Plan:** `plan/break-a-wall` · [docs/plans/break-a-wall.md](docs/plans/break-a-wall.md)

## Spike: Pressure-force semantics

Apply `ExternalForce` to a `RigidBody::Dynamic` entity in a minimal Avian
scene. Verify: (a) force integrates correctly over `FixedUpdate`, (b) force
can be updated every tick without accumulation issues, (c) `ExternalForce`
can be inserted at runtime on entities that didn't have it at spawn.

**Question to answer:** does `ExternalForce` persist or reset each frame?
Output: whether we need to clear/reset the force each tick and whether
runtime insertion works.

**Merged into:** Pressure-force system task (done as part of the same PR).

**Plan:** `plan/break-a-wall` · [docs/plans/break-a-wall.md](docs/plans/break-a-wall.md)

## Spike: Context-menu UI

Spawn a Bevy UI `Node` with a `WorldSpaceOverlay` targeting a specific
world position. Place two buttons inside it. Verify: (a) the node appears
at the correct projected position, (b) buttons receive `Interaction` events,
(c) moving the camera causes the menu to track the world position, (d) the
node doesn't interfere with 3D camera raycasting when dismissed.

**Question to answer:** does the world-to-viewport overlay pattern work for
interactive menus (not just passive text), and does the 2D UI camera
(order=1) correctly layer the menu above the 3D scene?

**Merged into:** Interactions module task (done as part of the same PR).

**Plan:** `plan/break-a-wall` · [docs/plans/break-a-wall.md](docs/plans/break-a-wall.md)

## Input module

Create `modules/input/` (new L0 module).

- `PointerAction { button: MouseButton, screen_pos: Vec2 }` event —
  normalised pointer event fired on mouse button press
- `WorldHit` enum with variants `Tile { position: IVec2, kind: TileKind }`
  and `Thing { entity: Entity, kind: u16 }` — shared hit-test result type
- `emit_pointer_actions` system in `PreUpdate`, gated on
  `in_state(AppState::InGame)` and `not(resource_exists::<Headless>)`
- `InputPlugin` struct
- `modules/input/Cargo.toml` — depends on `bevy`, `tiles` (for `TileKind`)
- Root `Cargo.toml` — add `input` to workspace members and `[dependencies]`
- `src/main.rs` — add `InputPlugin` to the app plugin chain

**Not included:** raycasting logic (that lives in `tiles` and `things`).

**Plan:** `plan/break-a-wall` · [docs/plans/break-a-wall.md](docs/plans/break-a-wall.md)

## Client-to-server streams

Implement the full client→server stream data path in the `network` module.
The `StreamDirection::ClientToServer` variant exists but has no runtime
plumbing.

- `modules/network/src/server.rs` — `accept_uni` from clients, route
  incoming frames to per-tag `StreamReader` buffers (server-side analogue
  of `route_stream_frame`)
- `modules/network/src/client.rs` — `open_uni` to send client→server
  streams, provide client-facing `StreamSender<T>` path
- `src/server.rs` — accept client→server stream connections
- `src/client.rs` — minimal changes for stream opening
- The public `StreamRegistry::register` API is unchanged; modules register
  with `StreamDirection::ClientToServer` as before

**Not included:** any module-specific stream registrations (those come in
the tiles task).

**Depends on:** nothing.

**Plan:** `plan/break-a-wall` · [docs/plans/break-a-wall.md](docs/plans/break-a-wall.md)

## WorldSpaceOverlay extraction

Extract the nameplate world-to-viewport positioning pattern from `player`
into a reusable system in `ui`, then refactor `player` to use it.

**`modules/ui/src/lib.rs`:**

- `WorldSpaceOverlay` — marker component for UI nodes tracking a world
  position
- `OverlayTarget(Entity)` — links the UI node to the 3D entity it tracks
- `OverlayOffset(Vec3)` — optional world-space offset above the target
- `update_world_space_overlays` system — projects target position to screen
  space, sets `Node::left`/`Node::top`, hides when behind camera. Runs in
  `Update` after `TransformPropagate`

**`modules/player/src/lib.rs`:**

- Refactor `spawn_nameplate` to spawn with `WorldSpaceOverlay` +
  `OverlayTarget` + `OverlayOffset(Vec3::Y * 2.0)` instead of custom
  positioning logic
- Remove `update_nameplate_positions` system (replaced by shared system)
- `Nameplate` component remains for player-specific queries;
  `NameplateTarget` removed (replaced by `OverlayTarget`)

**`modules/player/Cargo.toml`:** add `ui` dependency.

**Depends on:** nothing.

**Plan:** `plan/break-a-wall` · [docs/plans/break-a-wall.md](docs/plans/break-a-wall.md)

## Tile mutation replication

Add networked tile mutation to the `tiles` module: client→server requests,
server validation, and broadcast to all clients with incremental rendering.
Also adds tile hit detection for the interaction system.

**`modules/tiles/src/lib.rs`:**

- `TileToggle { position: IVec2, kind: TileKind }` — client→server message
- `TileMutated { position: IVec2, kind: TileKind }` — server→client message
- `TileToggleRequest { position: IVec2, kind: TileKind }` — Bevy event
  (fired by interactions module on menu selection)
- Stream 4 registration (`ClientToServer`)
- `raycast_tiles` system — listens for `PointerAction` (right-click),
  raycasts to ground plane (y=0), converts to grid coords, emits
  `WorldHit::Tile`. Gated on `InGame` + not headless
- `execute_tile_toggle` — reads `TileToggleRequest`, sends `TileToggle` on
  stream 4. Gated on `InGame` + not headless
- `handle_tile_toggle` — server reads `TileToggle` from stream 4, validates
  position in-bounds and tile differs, calls `Tilemap::set`, broadcasts
  `TileMutated` on stream 1. Gated on `InGame` + `Server`
- `handle_tile_mutation` — client receives `TileMutated`, calls
  `Tilemap::set`. Gated on `Client`
- `apply_tile_mutation` — new system handling `TileMutated` events, queries
  existing tile entity at affected grid position, swaps mesh/material/
  collider in place (incremental rendering)
- `spawn_tile_meshes` — gated on tile entities not yet existing (initial
  load only, no longer triggers on every `tilemap.is_changed()`)

**`modules/tiles/Cargo.toml`:** add `input` dependency.

**Depends on:** Input module, Client-to-server streams.

**Plan:** `plan/break-a-wall` · [docs/plans/break-a-wall.md](docs/plans/break-a-wall.md)

## Thing hit detection

Add `raycast_things` system to the `things` module.

- Listens for `PointerAction` (right-click), raycasts against entity
  colliders via `SpatialQuery`, emits `WorldHit::Thing { entity, kind }`
  for the nearest hit
- Gated on `in_state(AppState::InGame)` and
  `not(resource_exists::<Headless>)`
- No other changes to entity replication

**`modules/things/Cargo.toml`:** add `input` dependency.

**`modules/physics/src/lib.rs`:** re-export `SpatialQuery`.

**Depends on:** Input module.

**Plan:** `plan/break-a-wall` · [docs/plans/break-a-wall.md](docs/plans/break-a-wall.md)

## Gas grid replication

Add ongoing gas grid replication to the `atmospherics` module: server
broadcasts snapshots and deltas, clients apply them.

**`modules/atmospherics/src/lib.rs`:**

- `GasGridDelta { changes: Vec<(u16, f32)> }` — new variant in
  `AtmosStreamMessage`
- `GasGridData` extended with `passable: Vec<bool>` field
- `broadcast_gas_grid` system — sends full `GasGridData` snapshot every ~2s
  and `GasGridDelta` at ~10 Hz for cells changed beyond epsilon. Server
  tracks `last_broadcast_moles: Vec<f32>`. Gated on `Server` + `InGame`
- `handle_atmos_updates` — client applies snapshots and deltas to local
  `GasGrid`. Gated on `Client`
- `wall_toggle_input` system removed (raycast moves to tiles)

**`modules/atmospherics/src/gas_grid.rs`:**

- `from_moles_vec` updated to accept `passable: Vec<bool>` parameter
  (currently hardcodes all passable)
- `last_broadcast_moles: Vec<f32>` field for delta computation
- `sync_walls()` updated to zero moles on any cell whose passability
  changes — walls always hold 0.0 moles

**Depends on:** Tile mutation replication (for `TileMutated` triggering
`wall_sync_system` on clients, and for the `wall_toggle_input` removal to
be safe — raycasting now lives in tiles).

**Plan:** `plan/break-a-wall` · [docs/plans/break-a-wall.md](docs/plans/break-a-wall.md)

## Pressure-force system

Add pressure-gradient forces to the `atmospherics` module. Entities near
pressure differences are pushed by `ExternalForce`.

**`modules/atmospherics/src/gas_grid.rs`:**

- `pressure_gradient_at(pos)` helper — computes 2D gradient via central
  difference across cardinal neighbours

**`modules/atmospherics/src/lib.rs`:**

- `apply_pressure_forces` system in `FixedUpdate`, after
  `diffusion_step_system`, gated on `Server` + `InGame`
- Queries `RigidBody::Dynamic` entities with `Transform`, reads pressure
  gradient at entity's cell, scales by `PRESSURE_FORCE_SCALE`, applies via
  `ExternalForce` (inserted at runtime if absent)
- `PRESSURE_FORCE_SCALE` read from `config.toml`
  (`atmospherics.pressure_force_scale`)

**`modules/atmospherics/Cargo.toml`:** add `physics` dependency.

**`modules/physics/src/lib.rs`:** re-export `ExternalForce`.

**`config.toml`:** add `[atmospherics]` section with
`pressure_force_scale` value.

**Depends on:** Gas grid replication. Includes spike: Pressure-force
semantics.

**Plan:** `plan/break-a-wall` · [docs/plans/break-a-wall.md](docs/plans/break-a-wall.md)

## Test room redesign

Redesign the test room to demonstrate decompression: a pressurized chamber
adjacent to a sealed vacuum chamber.

**`modules/tiles/src/lib.rs`:**

- `Tilemap::test_room()` expanded to 16×10 with left chamber (cols 1–8,
  pressurized), right chamber (cols 11–14, vacuum), and separating wall
  (cols 9–10)

**`modules/atmospherics/src/lib.rs`:**

- `initialize_gas_grid` updated to accept an optional vacuum region
  (bounding rect) — floor cells inside the region start at 0.0 moles

**`src/world_setup.rs`:**

- Call updated `Tilemap::test_room()` and `initialize_gas_grid` with vacuum
  region covering the right chamber
- Ball and player spawn in the pressurised side

**Depends on:** Gas grid replication (for passability in `GasGridData`).

**Plan:** `plan/break-a-wall` · [docs/plans/break-a-wall.md](docs/plans/break-a-wall.md)

## Interactions module

Create `modules/interactions/` (new L6 module). Right-click context menu
system that reads `WorldHit` events and maps them to domain actions.

- `build_context_menu` — reads `WorldHit` events, looks up actions in
  hard-coded action table (`Wall` → "Remove Wall", `Floor` → "Build Wall",
  `Thing` → empty), spawns context menu anchored to target world position
  via `WorldSpaceOverlay`
- `handle_menu_selection` — on button press, fires domain event (e.g.
  `TileToggleRequest` defined in `tiles`)
- `dismiss_context_menu` — click-away, right-click elsewhere, or Escape.
  Despawns anchor entity (`OverlayTarget` target) to prevent entity leaks.
  Only one menu open at a time
- `InteractionsPlugin` struct
- `modules/interactions/Cargo.toml` — depends on `bevy`, `input`, `ui`,
  `tiles`
- Root `Cargo.toml` — add `interactions` to workspace members and
  `[dependencies]`
- `src/main.rs` — add `InteractionsPlugin` to app plugin chain, register
  context-menu button event types via `UiPlugin::with_event::<T>()`
- All systems gated on `in_state(AppState::InGame)` and
  `not(resource_exists::<Headless>)`

**Depends on:** Input module, WorldSpaceOverlay extraction, Tile mutation
replication. Includes spike: Context-menu UI.

**Plan:** `plan/break-a-wall` · [docs/plans/break-a-wall.md](docs/plans/break-a-wall.md)

## Simulation tuning

Tune atmospheric simulation constants against the vacuum-chamber test room
so the decompression scenario is visually representative.

- `DIFFUSION_RATE` — gas should flow noticeably through the breach
- `PRESSURE_CONSTANT` — pressure differences should be visible in the
  debug overlay
- `PRESSURE_FORCE_SCALE` — entities should be pushed convincingly toward
  the breach
- Substep count — enough iterations for stable, visible gas redistribution

Values are determined by running the full scenario (remove wall between
pressurized and vacuum chambers) and adjusting until gas flows visibly and
entities are pushed through the breach. No code changes beyond constant
values.

**Depends on:** Pressure-force system, Test room redesign, Interactions
module.

**Plan:** `plan/break-a-wall` · [docs/plans/break-a-wall.md](docs/plans/break-a-wall.md)

## Creature pressure-force slip

Creatures currently hard-set their velocity every frame via
`apply_input_velocity`, which completely overrides pressure forces. When
the pressure gradient force on a creature exceeds a configurable
threshold, the controller should lose authority and let the force carry
the creature (slipping toward the breach). Below the threshold the
controller wins as it does now.

- Add a slip threshold constant to `config.toml`
  (`atmospherics.slip_force_threshold` or similar)
- In `apply_input_velocity` (or a new system after it), compare the
  pressure force magnitude against the threshold; if exceeded, blend or
  skip the velocity override so `ConstantForce` takes effect
- Consider a gradual blend rather than a hard cutover to avoid jarring
  transitions

**Note:** After implementing this, the simulation constants
(`pressure_force_scale`, `diffusion_rate`, `pressure_constant`) will
need to be re-tuned so that the slip feels right — the threshold
interacts with all three values.

**Depends on:** Simulation tuning (#151).

## Hot-reload config.toml at runtime

Add a system that detects changes to `config.toml` and re-applies values
to their corresponding resources without restarting the game. Could be
file-mtime polling, filesystem notify, or a debug keypress (e.g. F6).

Any config value backed by a Bevy resource or mutable field is a
candidate: simulation tuning constants, network settings, debug flags,
UI preferences, etc. Init-only values (like `standard_pressure`, which
seeds the gas grid once) would need a separate "reset" action rather
than live update.

## Hot-reload assets at runtime

Enable Bevy's asset hot-reloading so that changes to asset files are
picked up at runtime without restarting the game. Bevy supports this via
`AssetPlugin { watch_for_changes: true, .. }` or the equivalent 0.18
configuration.

Use case: editing tile materials or creature meshes in an external tool
and seeing the result in the running game immediately, without a
restart–reconnect cycle.

## [Deferred debug-overlay-quad-lifecycle]

The atmos debug overlay (`spawn_overlay_quads` in `debug_overlay.rs`) spawns
quads for walkable tiles but never removes them when a tile becomes a wall
while the overlay is active. With "Build Wall" now possible, toggling a floor
to a wall while F3 is on leaves a stale overlay quad. Fix: despawn/respawn
affected quads on `TileMutated`, or re-run `spawn_overlay_quads` when the
tilemap changes while the overlay is visible.

**Files:** `modules/atmospherics/src/debug_overlay.rs`
