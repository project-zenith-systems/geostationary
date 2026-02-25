# Plan: Break a Wall, Watch Gas Rush Out

> **Stage goal:** Tilemap mutations and gas grid changes replicate in real
> time. A player right-clicks a wall, selects "Remove Wall" from a context
> menu, and all clients see the tile disappear, gas rush toward the breach,
> and nearby entities get pushed by the pressure gradient. The server is the
> single source of truth for tiles, atmospherics, and pressure forces. This
> is plan 2 of the Networked World State arc.

## What "done" looks like

1. Right-clicking a wall opens a floating context menu anchored to the wall's
   world position (projected to screen space) with a "Remove Wall" option
2. Selecting "Remove Wall" sends a request to the server; the server validates
   the target cell and applies the mutation
3. All connected clients see the wall mesh disappear, replaced by a floor, and
   the wall collider is removed
4. The test room has a pressurised chamber adjacent to a vacuum chamber,
   separated by a wall; removing that wall causes dramatic decompression
5. The server's gas simulation detects the newly passable cell; gas flows
   toward vacuum through the breach
6. The atmos debug overlay (F3) on all clients reflects the changing gas state
   in real time — pressure colours shift as gas redistributes
7. Physics bodies (ball, creatures) near the breach are pushed toward it by
   pressure-gradient forces applied server-side; their movement replicates
   to all clients via the existing entity state stream
8. Right-clicking a floor tile shows a context menu with "Build Wall"; the
   server validates and applies the reverse mutation, blocking gas flow
9. The interaction module's context menu is a general-purpose event-based
   system — `input` (L0) fires pointer events, `tiles`/`things` (L1)
   raycast and emit `WorldHit` events, `interactions` (L6) maps hits to
   actions and orchestrates the menu UX

## Strategy

The previous plan's post-mortem identified three recurring hurdles:
system-ordering bugs, codec edge cases, and undocumented coordination
patterns. This plan addresses all three by spiking pressure-force semantics
and context-menu UI before implementation, designing scheduling constraints
in the layer participation table, and using the established stream
registration pattern without inventing new coordination mechanisms.

**Data flow — wall removal:**

```
Client                          Server                          Other clients
  │                               │                                │
  │ right-click                   │                                │
  │ → input: PointerAction        │                                │
  │ → tiles: raycast_tiles        │                                │
  │   → WorldHit::Tile            │                                │
  │ → interactions: context menu  │                                │
  │                               │                                │
  │ select "Remove Wall"          │                                │
  │ → interactions fires          │                                │
  │   TileToggleRequest           │                                │
  │ → tiles: execute_tile_toggle  │                                │
  │ ─── TileToggle{pos} ───────► │                                │
  │     (stream 4, c→s)          │                                │
  │                               │ validate pos                   │
  │                               │ Tilemap::set(pos, Floor)       │
  │                               │ GasGrid::sync_walls()          │
  │                               │ (gas sim runs in FixedUpdate)  │
  │                               │                                │
  │                               │ ─── TileMutated{pos, kind} ──►│
  │ ◄── TileMutated{pos, kind} ──│     (stream 1, s→c)            │
  │                               │                                │
  │ apply Tilemap::set locally    │                                │ apply Tilemap::set locally
  │ re-render affected tile       │                                │ re-render affected tile
  │                               │                                │
  │                               │ (ongoing, FixedUpdate)         │
  │                               │ gas sim: flow toward breach    │
  │                               │ pressure-force: ExternalForce  │
  │                               │   on nearby RigidBody entities │
  │                               │                                │
  │ ◄── GasGridSnapshot ─────────│──── GasGridSnapshot ─────────►│
  │     (stream 2, s→c, periodic) │     (stream 2, s→c, periodic)  │
  │ ◄── GasGridDelta ────────────│──── GasGridDelta ─────────────►│
  │     (stream 2, s→c, between   │     (stream 2, s→c, between    │
  │      snapshots)               │      snapshots)                │
  │                               │                                │
  │ update local GasGrid          │                                │ update local GasGrid
  │ overlay colours shift         │                                │ overlay colours shift
  │                               │                                │
  │ ◄── StateUpdate ─────────────│──── StateUpdate ──────────────►│
  │     (stream 3, s→c, 30 Hz)   │     (stream 3, s→c, 30 Hz)     │
  │                               │                                │
  │ entities slide toward breach  │                                │ entities slide toward breach
```

**Stream architecture (additions in bold):**

| Stream tag | Owner          | Direction           | Content                                              |
| ---------- | -------------- | ------------------- | ---------------------------------------------------- |
| 0          | `network`      | bidirectional       | Welcome, InitialStateDone / Hello, Input (unchanged) |
| 1          | `tiles`        | server → client     | TilemapData, **TileMutated**, StreamReady            |
| 2          | `atmospherics` | server → client     | GasGridData, **GasGridDelta**, StreamReady           |
| 3          | `things`       | server → client     | EntitySpawned, StateUpdate, StreamReady (unchanged)  |
| **4**      | **`tiles`**    | **client → server** | **TileToggle { position, kind }**                    |

Work proceeds bottom-up: protocol and stream changes first, then tile
mutation replication, then gas grid replication, then pressure-force
coupling, then the interactions module last (it sends messages through
already-working infrastructure).

### Layer participation

| Layer | Module               | Systems / changes                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   | Schedule / run condition                                                                                                                                                                                                                                 |
| ----- | -------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| L0    | `input`              | **New module.** `PointerAction { button: MouseButton, screen_pos: Vec2 }` event — normalised pointer event fired when a mouse button is pressed. `WorldHit` enum — shared hit-test result type with variants `Tile { position: IVec2, kind: TileKind }` and `Thing { entity: Entity, kind: u16 }`. Both types are domain-neutral and live at L0 so that L1 modules can emit them and L6 can read them. No raycasting logic — `input` only captures device state and defines the shared vocabulary.                                                                                                                                                                                                                                                                                                                  | `emit_pointer_actions`: `PreUpdate`, after Bevy input update.                                                                                                                                                                                            |
| L0    | `network`            | **Implement client→server stream data path.** The `ClientToServer` direction variant exists but has no runtime plumbing — the server task does not `accept_uni` from clients, the client task does not `open_uni` to the server, and there is no server-side analogue of `route_stream_frame` to populate `StreamReader` buffers. This plan adds all three, plus a client-facing `StreamSender<T>` path. Modules continue using `StreamRegistry::register` with `StreamDirection::ClientToServer`; the public API is unchanged.                                                                                                                                                                                                                                                                                     | Drain client→server frames in `PreUpdate`, same as existing client event drain.                                                                                                                                                                          |
| L0    | `ui`                 | **Extract world-to-viewport overlay pattern** from `player` module into a reusable `WorldSpaceOverlay` component and `update_world_space_overlays` system. This generalizes the nameplate positioning code so both nameplates and context menus (and future world-anchored UI) share the same projection logic. Existing `Nameplate`/`NameplateTarget` in `player` refactored to use the shared system.                                                                                                                                                                                                                                                                                                                                                                                                             | `update_world_space_overlays`: `Update`, after `TransformPropagate`.                                                                                                                                                                                     |
| L1    | `tiles`              | **Hit detection:** `raycast_tiles` system listens for `PointerAction` (right-click), raycasts against the tile grid (ground-plane intersection → grid coordinates), emits `WorldHit::Tile { position, kind }`. **Stream 4 (client→server):** registers stream 4, adds `TileToggle { position, kind }` message. `execute_tile_toggle` system listens for a `TileToggleRequest` event (fired by interactions) and sends `TileToggle` on stream 4. **Server-side:** `handle_tile_toggle` reads `TileToggle` from stream 4, validates, mutates `Tilemap`, broadcasts `TileMutated` on stream 1. **Client-side:** `handle_tile_mutation` receives `TileMutated`, applies `Tilemap::set`, triggers re-render. `spawn_tile_meshes` updated for incremental tile changes (see tile mutation design for approach).           | `raycast_tiles`: `Update`, after `PointerAction` events. `execute_tile_toggle`: `Update`. `handle_tile_toggle`: `Update`, after stream drain. `handle_tile_mutation`: `Update`, after stream drain.                                                      |
| L1    | `things`             | **Hit detection:** `raycast_things` system listens for `PointerAction` (right-click), raycasts against entity colliders, emits `WorldHit::Thing { entity, kind }` for the nearest hit. No other changes — entity replication remains as-is.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | `raycast_things`: `Update`, after `PointerAction` events.                                                                                                                                                                                                |
| L2    | `atmospherics`       | **Ongoing replication:** `broadcast_gas_grid` system sends full `GasGridData` snapshot periodically (every ~2s) and `GasGridDelta { changes: Vec<(u16, f32)> }` between snapshots at ~10 Hz for cells whose moles changed beyond an epsilon. Client: `handle_atmos_updates` applies snapshots and deltas to local `GasGrid`. **Passability:** `GasGridData` extended with `passable: Vec<bool>` so client gas grid has correct wall data for overlay rendering. **Pressure-force system:** `apply_pressure_forces` queries all `RigidBody::Dynamic` entities with `Transform`, reads `GasGrid` pressure at entity's cell and neighbouring cells, computes net force vector from pressure gradient, applies via `ExternalForce`. Runs server-side only. `wall_toggle_input` system removed (raycast moves to tiles). | `broadcast_gas_grid`: `Update`, gated on `Res<Server>`, throttled by timer. `handle_atmos_updates`: `Update`, gated on `Res<Client>`, after stream drain. `apply_pressure_forces`: `FixedUpdate`, after `diffusion_step_system`, gated on `Res<Server>`. |
| L6    | `interactions`       | **New module.** Reads `WorldHit` events from `input` (L0). For each hit, looks up available actions from a hard-coded action table (e.g. `WorldHit::Tile` with `kind == Wall` → "Remove Wall"; `kind == Floor` → "Build Wall"). Spawns context menu anchored to the target's world position using `WorldSpaceOverlay` from `ui` (L0). On menu selection, fires domain-specific action events (e.g. `TileToggleRequest` defined in `tiles`). The action table is hard-coded now but designed for future scripting — the lookup is a simple match that can be replaced with a script-driven registry. Depends on: `input` (L0) for `WorldHit`, `ui` (L0) for button rendering and `WorldSpaceOverlay`, `tiles` (L1) for `TileToggleRequest` event type.                                                               | `build_context_menu`: `Update`, after `WorldHit` events. `handle_menu_selection`: `Update` (via button message system). `dismiss_context_menu`: `Update` (click-away or Escape).                                                                         |
| —     | `src/server.rs`      | Accept client→server streams. No new orchestration — tile mutation flows through the tiles module's own stream handler.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                             | —                                                                                                                                                                                                                                                        |
| —     | `src/client.rs`      | Minimal — stream routing already handled by network module.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | —                                                                                                                                                                                                                                                        |
| —     | `src/world_setup.rs` | **Test room update.** `Tilemap::test_room()` expanded: a pressurised main room (left) separated by an internal wall from a vacuum chamber (right). The vacuum chamber has floor tiles but `initialize_gas_grid` does not fill them with standard pressure — they start at 0.0 moles. Removing the separating wall causes dramatic decompression. The ball and player spawn in the pressurised side.                                                                                                                                                                                                                                                                                                                                                                                                                 | —                                                                                                                                                                                                                                                        |

**Run conditions:** All client-side interaction systems
(`emit_pointer_actions`, `raycast_tiles`, `raycast_things`,
`build_context_menu`, `handle_menu_selection`, `dismiss_context_menu`,
`execute_tile_toggle`) are gated on `in_state(AppState::InGame)` and
`not(resource_exists::<Headless>)`. Server-side systems
(`handle_tile_toggle`, `apply_pressure_forces`, `broadcast_gas_grid`) are
gated on `in_state(AppState::InGame)` and `resource_exists::<Server>`.
Client-side receivers (`handle_tile_mutation`, `handle_atmos_updates`) are
gated on `resource_exists::<Client>`.

### Not in this plan

- **Client-side gas simulation.** Clients render server-sent gas state. No
  local diffusion. The arc explicitly excludes this.
- **Client-side prediction for tile mutations.** Client waits for server
  confirmation before rendering the change. Latency is acceptable for a
  12×10 test room.
- **Delta compression or bandwidth optimisation.** Full gas grid snapshots
  are ~480 bytes. The arc excludes bandwidth work.
- **Gas mixtures, temperature, or advanced atmos.** Single-gas moles only.
- **Item interactions.** Plan 3 adds pick up / drop / container actions to
  the interactions module. This plan only adds tile toggle.
- **Admin-level tile editing.** All clients can toggle walls. No permission
  system.
- **Tilemap resize or structural changes beyond wall toggle.** Only
  Wall↔Floor toggling is supported.
- **Entity interpolation or smoothing.** Clients snap to server truth, same
  as plan 1.
- **Sound effects.** No audio for wall breaking or gas rushing.

### Module placement

```
modules/
  input/                         # NEW MODULE (L0)
    Cargo.toml
    src/
      lib.rs                     # PointerAction event, WorldHit enum.
                                 #   emit_pointer_actions system.
  ui/
    src/
      lib.rs                     # MODIFIED — add WorldSpaceOverlay component,
                                 #   OverlayTarget component,
                                 #   update_world_space_overlays system
  interactions/                  # NEW MODULE (L6)
    Cargo.toml
    src/
      lib.rs                     # Reads WorldHit events → looks up actions
                                 #   → spawns context menu at target world
                                 #   pos → fires domain action events
                                 #   (e.g. TileToggleRequest) on selection
  network/
    src/
      server.rs                  # MODIFIED — accept client→server streams,
                                 #   route to per-tag StreamReader buffers
      client.rs                  # MODIFIED — open client→server streams
  tiles/
    src/
      lib.rs                     # MODIFIED — TileMutated, TileToggle messages.
                                 #   Stream 4 registration (c→s). Server-side
                                 #   handle_tile_toggle. Client-side
                                 #   handle_tile_mutation. Incremental tile
                                 #   re-rendering. raycast_tiles system
                                 #   (listens for PointerAction, emits
                                 #   WorldHit::Tile). TileToggleRequest
                                 #   event + execute_tile_toggle system.
  things/
    src/
      lib.rs                     # MODIFIED — raycast_things system (listens
                                 #   for PointerAction, emits
                                 #   WorldHit::Thing). No other changes.
  atmospherics/
    src/
      lib.rs                     # MODIFIED — ongoing replication systems,
                                 #   GasGridDelta message, pressure-force
                                 #   system. Remove wall_toggle_input
                                 #   (raycast moves to tiles).
      gas_grid.rs                # MODIFIED — delta tracking (snapshot of
                                 #   previous moles for change detection),
                                 #   passable vec in serialization,
                                 #   pressure_gradient_at(pos) helper
      debug_overlay.rs           # NO CHANGES — already reads GasGrid resource,
                                 #   will reflect replicated updates via
                                 #   change detection
  player/
    src/
      lib.rs                     # MODIFIED — refactor nameplate positioning to
                                 #   use shared WorldSpaceOverlay from ui module
  camera/
    src/
      lib.rs                     # NO CHANGES expected
src/
  server.rs                      # MODIFIED — client→server stream acceptance
  client.rs                      # MINIMAL CHANGES
  world_setup.rs                 # MODIFIED — test room redesigned with
                                 #   pressurised room + vacuum chamber
```

### Dependency and wiring changes

New and modified `Cargo.toml` entries:

- **Root `Cargo.toml`:** Add `input` and `interactions` to workspace members
  and root `[dependencies]`
- **`modules/input/Cargo.toml`:** New. Depends on `bevy`
- **`modules/interactions/Cargo.toml`:** New. Depends on `bevy`, `input`,
  `ui`, `tiles`
- **`modules/tiles/Cargo.toml`:** Add `input` (for `PointerAction`,
  `WorldHit`)
- **`modules/things/Cargo.toml`:** Add `input` (for `PointerAction`,
  `WorldHit`)
- **`modules/atmospherics/Cargo.toml`:** Add `physics` (for `ExternalForce`,
  `RigidBody` queries in pressure-force system)
- **`modules/player/Cargo.toml`:** Add `ui` (for `WorldSpaceOverlay`)
- **`modules/physics/src/lib.rs`:** Re-export `ExternalForce` and
  `SpatialQuery`

Plugin and event registration in `src/main.rs`:

- Add `InputPlugin` and `InteractionsPlugin` to the app plugin chain
- Register context-menu button event types via
  `UiPlugin::with_event::<T>()` — each button message type used by the
  interactions module must be registered at compile time for
  `process_button_messages::<T>` to fire

### Tile mutation design

Tile mutations flow through three layers on the client before reaching the
server:

1. **`input` (L0):** Player right-clicks. The `emit_pointer_actions` system
   fires `PointerAction { button: Right, screen_pos }`.

2. **`tiles` (L1):** `raycast_tiles` listens for `PointerAction` (right-click
   only), raycasts from `screen_pos` through the 3D camera to the ground
   plane (y = 0), converts the world intersection to grid coordinates, and
   emits `WorldHit::Tile { position, kind }` if a valid tile exists there.

3. **`interactions` (L6):** `build_context_menu` reads `WorldHit::Tile`,
   looks up available actions in its hard-coded action table (wall →
   "Remove Wall"; floor → "Build Wall"), spawns a context menu anchored
   to the tile's world position via `WorldSpaceOverlay`.

4. **`interactions` → `tiles`:** On menu selection, the button message fires
   `TileToggleRequest { position, kind }` (event defined in `tiles`).
   The `tiles` module's `execute_tile_toggle` system reads this and sends
   `TileToggle { position, kind }` on stream 4 (tiles client→server).

5. **Server `tiles`:** Receives `TileToggle` on stream 4, validates the
   position is in-bounds and the current tile differs from the requested
   kind, then calls `Tilemap::set(pos, kind)`.

6. Bevy change detection on the `Tilemap` resource triggers
   `wall_sync_system` in the atmospherics module (already exists, runs in
   `FixedUpdate` before `diffusion_step_system`). `sync_walls()` is updated
   to also zero the moles of any cell whose passability changes — walls
   always hold 0.0 moles. When a wall becomes floor, the cell is at vacuum;
   gas flows in from adjacent pressurised cells on the next diffusion step.
   When a floor becomes wall, its gas is removed.

7. The server broadcasts `TileMutated { position, kind }` on stream 1
   (tiles server→client) to all connected clients.

8. Each client's `tiles` module receives `TileMutated`, calls
   `Tilemap::set(pos, kind)`, and updates the visual representation.
   The current `spawn_tile_meshes` system despawns and respawns _all_ tile
   entities whenever `tilemap.is_changed()` triggers — a naïve
   `Tilemap::set` from `handle_tile_mutation` would cause a full rebuild.
   To avoid this, a separate `apply_tile_mutation` system handles
   `TileMutated` events: it queries the existing tile entity at the affected
   grid position and swaps its mesh, material, and collider in place.
   `spawn_tile_meshes` is restricted to initial tilemap loads (gated on tile
   entities not yet existing).

### Gas grid replication design

The server sends gas grid state to clients using a hybrid snapshot + delta
approach:

**Full snapshots** are sent periodically (every ~2 seconds) on stream 2.
These use the existing `GasGridData` message, extended with a `passable`
field. A snapshot resets the client's gas grid entirely, correcting any
accumulated drift from missed or misordered deltas.

**Delta updates** are sent between snapshots at ~10 Hz. A new
`GasGridDelta { changes: Vec<(u16, f32)> }` message carries only cells
whose moles changed by more than an epsilon since the last broadcast. The
cell index is a `u16` (sufficient for grids up to 256×256). The server
tracks `last_broadcast_moles: Vec<f32>` to compute deltas.

The client applies updates to the local `GasGrid` resource. The debug
overlay already reads `GasGrid` via change detection and will reflect
updates without modification.

**Constructor update:** The existing `GasGrid::from_moles_vec()` discards
passability data (hardcodes all cells as passable). It needs an additional
`passable: Vec<bool>` parameter so snapshots correctly reconstruct wall
state on the client.

**Passability on the client:** The extended `GasGridData` snapshot includes
passability data so the client `GasGrid` correctly reflects wall positions.
The client-side `wall_sync_system` (already running in `FixedUpdate`)
detects `Tilemap` changes from `handle_tile_mutation` and updates
`GasGrid.passable` automatically, keeping the overlay accurate between
snapshots.

### Pressure-force design

Pressure differences across cell boundaries create forces on physics bodies.
The system lives in the `atmospherics` module (L2) and depends on `physics`
(L0) — strict downward dependency.

**`apply_pressure_forces` system (server-side, FixedUpdate):**

1. For each `RigidBody::Dynamic` entity with a `Transform`, determine which
   gas grid cell it occupies (world position → grid index).
2. Read the pressure (moles × `PRESSURE_CONSTANT`) of the occupied cell and
   its four cardinal neighbours.
3. Compute the pressure gradient as a 2D vector: for each axis, the gradient
   is `(pressure_positive - pressure_negative) / 2.0` (central difference).
   Missing or impassable neighbours use the occupied cell's pressure
   (zero gradient contribution).
4. Scale the gradient by a tunable `PRESSURE_FORCE_SCALE` constant to
   produce a force vector.
5. Apply the force via Avian's `ExternalForce` component. If the entity
   doesn't have one, insert it.

**Design choices:**

- Forces are computed server-side only. Clients see the result via entity
  state replication (stream 3). This avoids client-side physics simulation
  divergence.
- The force is proportional to the local pressure gradient, not the absolute
  pressure. A uniformly pressurised room exerts no net force. Only
  pressure _differences_ (like a breach to vacuum) create forces.
- `PRESSURE_FORCE_SCALE` is a config value (`atmospherics.pressure_force_scale`)
  for tuning without recompilation.

**Simulation tuning:** `DIFFUSION_RATE`, `PRESSURE_CONSTANT`, substep
count, and `PRESSURE_FORCE_SCALE` are tuned during this plan to make the
decompression scenario visually representative — gas should flow noticeably
through the breach and push entities convincingly. Tuning values are
determined during integration testing against the vacuum-chamber test room.

### Test room design

The current `Tilemap::test_room()` is a 12×10 room with perimeter walls and
internal obstacles. Gas at the grid edges has no neighbour — the diffusion
system only iterates within bounds, making edges implicitly impermeable.
Removing a perimeter wall has nowhere for gas to rush to.

The test room is redesigned to demonstrate decompression:

```
  0 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15
0 W W W W W W W W W W  W  W  W  W  W  W
1 W . . . . . . . . W  W  .  .  .  .  W
2 W . . . . . . . . W  W  .  .  .  .  W
3 W . . . W . . . . W  W  .  .  .  .  W
4 W . . . W . . . . W  W  .  .  .  .  W
5 W . . . W . W W W W  W  .  .  .  .  W
6 W . . . . . . . . W  W  .  .  .  .  W
7 W . . . . . . . . W  W  .  .  .  .  W
8 W . . . . . . . . W  W  .  .  .  .  W
9 W W W W W W W W W W  W  W  W  W  W  W
```

- **Left chamber** (cols 1–8): pressurised with standard atmosphere. Player
  and ball spawn here.
- **Right chamber** (cols 11–14): vacuum (0.0 moles). Floor tiles exist but
  `initialize_gas_grid` is updated to accept an optional vacuum region
  (defined as a bounding rect) — floor cells inside the vacuum region are
  left at 0.0 moles instead of being filled with standard pressure.
- **Separating wall** (cols 9–10): a two-tile-thick wall fully sealing the
  two chambers. The player must break through one of these walls to trigger
  decompression.
- The grid is widened from 12×10 to 16×10 to accommodate the vacuum chamber.

This layout lets a playtester remove a wall between chambers and watch gas
rush from high pressure to vacuum, pushing entities through the breach.

### Interactions module design

The interactions module (L6) provides a general-purpose right-click context
menu system. It reads `WorldHit` events emitted by lower-layer modules
(tiles, things) and maps them to available actions.

**Three-layer data flow:**

```
input (L0)              tiles/things (L1)           interactions (L6)
─────────────           ─────────────────           ──────────────────
PointerAction  ──►     raycast own domain
                        emit WorldHit       ──►     read WorldHit
                                                    look up actions
                                                    show context menu
                                                    on selection:
                        ◄── TileToggleRequest ──    fire action event
execute_tile_toggle
send TileToggle
on stream 4
```

**Action table (hard-coded, scriptable later):**

The interactions module contains a match over `WorldHit` variants that
returns available actions:

- `WorldHit::Tile { kind: Wall, .. }` → \["Remove Wall"\]
- `WorldHit::Tile { kind: Floor, .. }` → \["Build Wall"\]
- `WorldHit::Thing { .. }` → \[\] (no thing interactions in this plan)

This match is the future scripting hook — it can be replaced with a
script-driven registry without changing the surrounding event plumbing.

**Context menu lifecycle:**

- Spawned anchored to the hit target's world position via
  `WorldSpaceOverlay` from `ui` (L0)
- One button per available action (using `ui` button builder)
- Dismissed on: left-click outside, right-click elsewhere, Escape key
- Only one context menu can be open at a time
- Menu tracks the world position if the camera moves
- Anchor entity (the `OverlayTarget` target) is despawned when the menu is
  dismissed to prevent entity leaks

**Extensibility for plan 3:** The items module (L2) will add a
`raycast_items` system that emits `WorldHit::Thing` for item entities.
The interactions module's action table gains new `WorldHit::Thing`
matches ("Pick up", "Drop"). The event flow and menu logic remain
unchanged — only the action table grows.

### WorldSpaceOverlay design

The `player` module's nameplate code already solves "anchor a UI node to a
world-space position": it uses `Camera::world_to_viewport()` to project a
3D point to screen coordinates and sets `Node::left`/`Node::top` via
absolute positioning. The context menu needs the same pattern — anchor the
menu to the target wall's world position.

Rather than duplicating this logic, the pattern is extracted into the `ui`
module (L0) as a reusable system:

- **`WorldSpaceOverlay`** — marker component for UI nodes that track a
  world-space position.
- **`OverlayTarget(Entity)`** — component linking the UI node to the 3D
  entity whose position it tracks. For nameplates, this is the creature
  entity. For context menus, this is a temporary anchor entity spawned at
  the wall's world position.
- **`OverlayOffset(Vec3)`** — optional world-space offset above the target.
- **`update_world_space_overlays`** — system that projects each overlay's
  target position to screen space and updates the node's absolute position.
  Hides the node when the target is behind the camera.

The `player` module's `Nameplate`, `NameplateTarget`, and
`update_nameplate_positions` are refactored to use these shared components.
`spawn_nameplate` spawns with `WorldSpaceOverlay` + `OverlayTarget` +
`OverlayOffset(Vec3::Y * 2.0)` instead of custom positioning logic.

### Spikes

Two spikes precede implementation:

1. **Pressure-force spike** — Apply `ExternalForce` to a `RigidBody::Dynamic`
   entity in a minimal Avian scene. Verify: (a) force integrates correctly
   over FixedUpdate, (b) force can be updated every tick without
   accumulation issues, (c) `ExternalForce` can be inserted at runtime on
   entities that didn't have it at spawn. Question to answer: does
   `ExternalForce` persist or reset each frame? Output: whether we need to
   clear/reset the force each tick and whether runtime insertion works.

2. **Context-menu UI spike** — Spawn a Bevy UI `Node` with a
   `WorldSpaceOverlay` targeting a specific world position. Place two
   buttons inside it. Verify: (a) the node appears at the correct projected
   position, (b) buttons receive `Interaction` events, (c) moving the
   camera causes the menu to track the world position, (d) the node doesn't
   interfere with 3D camera raycasting when dismissed. Question to answer:
   does the world-to-viewport overlay pattern work for interactive menus
   (not just passive text), and does the 2D UI camera (order=1) correctly
   layer the menu above the 3D scene?

## Post-mortem

_(To be filled in after the plan ships.)_
