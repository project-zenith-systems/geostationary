## Spike: Quinn multi-stream

Open 3 serverâ†’client unidirectional streams from one `quinn::Connection`.
Verify the client can `accept_uni()` all 3, that stream ID tag bytes arrive
correctly, and that `StreamReady` sentinels can arrive in any order relative
to the control stream. Confirm per-stream `LengthDelimitedCodec` framing
works independently. 30 min.

- No dependencies

**Plan:** `plan/dedicated-server-with-souls` Â· [docs/plans/dedicated-server-with-souls.md](docs/plans/dedicated-server-with-souls.md)

## Spike: Bevy 0.18 billboard text

Can `Text` be placed in world space as a child of a 3D entity with billboard
behavior? Test with a `Text` + `Transform` child entity. Determines whether
nameplates are 3D world-space children or 2D UI overlays. 30 min.

- No dependencies

**Plan:** `plan/dedicated-server-with-souls` Â· [docs/plans/dedicated-server-with-souls.md](docs/plans/dedicated-server-with-souls.md)

## Spike: Headless Avian3D

Does `PhysicsPlugin` work with `MinimalPlugins` instead of `DefaultPlugins`?
Spawn a dynamic body and step the schedule twice. Identify the minimal plugin
set needed. 30 min.

- No dependencies

**Plan:** `plan/dedicated-server-with-souls` Â· [docs/plans/dedicated-server-with-souls.md](docs/plans/dedicated-server-with-souls.md)

## Network: StreamRegistry and multi-stream protocol

Files: `modules/network/src/lib.rs`, `modules/network/src/protocol.rs`,
`modules/network/src/server.rs`, `modules/network/src/client.rs`

- Add `StreamRegistry` resource: modules register streams with a `StreamId`
  tag byte, name, direction, and message types
- Add `StreamSender<T>` typed resource for modules to write to their stream
- Server: on client connect, open all registered serverâ†’client streams (each
  prefixed with tag byte), accept clientâ†’server streams
- Client: accept serverâ†’client streams, route framed messages to per-stream
  Bevy events by tag byte
- Per-stream `LengthDelimitedCodec` framing (replaces single-stream framing)
- `Hello` gains `name: String` field
- `Welcome` gains `expected_streams: u8` field
- Add `InitialStateDone` and `StreamReady` message types
- Control stream (0) carries `Hello`, `Welcome`, `InitialStateDone` only

Not included: domain-specific stream handlers (those belong to each module's
task).

- Depends on: Spike: Quinn multi-stream

**Plan:** `plan/dedicated-server-with-souls` Â· [docs/plans/dedicated-server-with-souls.md](docs/plans/dedicated-server-with-souls.md)

## Tiles: Serialization and stream 1 handler

Files: `modules/tiles/src/lib.rs`

- Add `to_bytes()` / `from_bytes()` serialization methods on `Tilemap`
- Register stream 1 (serverâ†’client) with `StreamRegistry`
- Server-side: send `TilemapData { width, height, tiles }` + `StreamReady`
  on connect
- Client-side: observe stream 1 messages, call `Tilemap::from_bytes()`,
  insert resource
- Define `TilesStreamMessage` enum for stream 1 wire format

Not included: tilemap mutation replication (that's arc step 2).

- Depends on: Network: StreamRegistry and multi-stream protocol

**Plan:** `plan/dedicated-server-with-souls` Â· [docs/plans/dedicated-server-with-souls.md](docs/plans/dedicated-server-with-souls.md)

## Things: DisplayName, stream 3, and entity lifecycle

Files: `modules/things/src/lib.rs`

- Add `DisplayName(String)` component
- Register stream 3 (serverâ†’client) with `StreamRegistry`
- Move `NetIdIndex` from `src/client.rs` into `things` module
- Own full client-side entity lifecycle:
  - `EntitySpawned`: spawn via `SpawnThing`, insert `DisplayName`, track in
    `NetIdIndex`
  - `EntityDespawned`: despawn entity, remove from `NetIdIndex`
  - `StateUpdate`: position sync from server state
- Server-side: broadcast entity state on stream 3
- Define `ThingsStreamMessage` enum for stream 3 wire format
- `EntitySpawned` gains `name: Option<String>` field

Not included: soul binding logic (that's in the souls task).

- Depends on: Network: StreamRegistry and multi-stream protocol

**Plan:** `plan/dedicated-server-with-souls` Â· [docs/plans/dedicated-server-with-souls.md](docs/plans/dedicated-server-with-souls.md)

## Atmospherics: Serialization and stream 2 handler

Files: `modules/atmospherics/src/lib.rs`, `modules/atmospherics/src/gas_grid.rs`

- Add `moles_vec()` / `from_moles_vec()` serialization methods on `GasGrid`
- Register stream 2 (serverâ†’client) with `StreamRegistry`
- Server-side: send `GasGridData { gas_moles }` + `StreamReady` on connect
- Client-side: observe stream 2 messages, call `GasGrid::from_moles_vec()`,
  insert resource
- Define `AtmosStreamMessage` enum for stream 2 wire format

- Depends on: Network: StreamRegistry and multi-stream protocol

**Plan:** `plan/dedicated-server-with-souls` Â· [docs/plans/dedicated-server-with-souls.md](docs/plans/dedicated-server-with-souls.md)

## Souls: New module with bind/unbind

Files: `modules/souls/Cargo.toml` (new), `modules/souls/src/lib.rs` (new),
`src/config.rs`

- Create new L4 module `souls` with dependencies on `creatures` and `network`
- `Soul { name: String, client_id: ClientId, bound_to: Entity }` component
  on a dedicated entity (single struct â€” all fields always required)
- `bind_soul` system: spawn soul entity, spawn creature, set `DisplayName`
  on creature, bind soul to creature
- `unbind_soul` system: despawn soul entity, clear `InputDirection` on
  creature (creature stays in world)
- Route `Hello` and `Input` messages on stream 0 (clientâ†’server)
- Replace `ControlledByClient` and `PlayerControlled` as the authority on
  which client controls which creature
- Add `player_name` field to `AppConfig` in `src/config.rs`
- Add `souls` to workspace `Cargo.toml`

Not included: soul transfer, possession, ghost mode, or observer mode.

- Depends on: Things: DisplayName, stream 3, and entity lifecycle

**Plan:** `plan/dedicated-server-with-souls` Â· [docs/plans/dedicated-server-with-souls.md](docs/plans/dedicated-server-with-souls.md)

## World setup: Server-only gate and ball replication

Files: `src/world_setup.rs`

- Gate `setup_world` with `.run_if(resource_exists::<Server>)` so clients
  don't generate local world state
- Ball spawned with `NetId` from `server.next_net_id()`
- Register ball as thing kind 1 in `ThingRegistry` (kind 0 = creature)
- Ball included in `EntitySpawned` broadcast (kind 1) and `StateUpdate`

- Depends on: Things: DisplayName, stream 3, and entity lifecycle

**Plan:** `plan/dedicated-server-with-souls` Â· [docs/plans/dedicated-server-with-souls.md](docs/plans/dedicated-server-with-souls.md)

## Server and client: Connect orchestration and initial sync

Files: `src/server.rs`, `src/client.rs`

- `src/server.rs`: on client connect, notify all registered stream handlers
  to send initial data; send `InitialStateDone` on control stream after all
  module streams have written initial data
- `src/server.rs`: `handle_disconnect` despawns soul entity (delegates to
  souls module)
- `src/client.rs`: track `StreamReady` count and `InitialStateDone` receipt;
  initial sync complete when both conditions met
- `src/client.rs`: thin â€” stream message routing handled by `network` module,
  domain logic in respective modules
- Add `setup_client_scene` system for client-only lighting (replaces local
  world generation)

- Depends on: Tiles, Things, Atmospherics stream handlers; Souls module

**Plan:** `plan/dedicated-server-with-souls` Â· [docs/plans/dedicated-server-with-souls.md](docs/plans/dedicated-server-with-souls.md)

## Player: Nameplate rendering

Files: `modules/player/src/lib.rs`

- `spawn_nameplate` system: for each entity with `DisplayName`, spawn child
  entity with `Text`, `TextFont`, `TextColor`, and `Nameplate` marker
- `update_nameplate_positions` system: position nameplates above parent
  entities each frame (billboard text or world-to-viewport projection,
  depending on spike result)

Not included: name input UI (names come from config).

- Depends on: Spike: Bevy 0.18 billboard text; Things: DisplayName

**Plan:** `plan/dedicated-server-with-souls` Â· [docs/plans/dedicated-server-with-souls.md](docs/plans/dedicated-server-with-souls.md)

## Headless server mode

Files: `src/main.rs`

- Parse `--server` from `std::env::args()` at startup
- When set: use `MinimalPlugins` instead of `DefaultPlugins` (plus any
  additional plugins identified by the headless Avian3D spike)
- Skip `WindowPlugin`, `UiPlugin`, `MainMenuPlugin`, `CameraPlugin`,
  visual-only systems from `TilesPlugin` and `AtmosphericsPlugin`
- Set initial state to `AppState::InGame` (skip main menu)
- Auto-send `NetCommand::Host { port }` on startup

- Depends on: Spike: Headless Avian3D; Server and client: Connect
  orchestration and initial sync

**Plan:** `plan/dedicated-server-with-souls` Â· [docs/plans/dedicated-server-with-souls.md](docs/plans/dedicated-server-with-souls.md)
