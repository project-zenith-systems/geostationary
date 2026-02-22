# Plan: Dedicated Server with Player Souls

> **Stage goal:** A headless server hosts the world. Clients connect, send a
> name, and get bound to a creature through a soul. Names render as billboard
> text above each character. Disconnecting unbinds the soul but leaves the
> creature in the world. The bouncing ball is server-spawned and replicated to
> all clients. Clients receive the tilemap and gas grid on connect instead of
> generating them locally.

## What "done" looks like

1. Running the binary with `--server` starts a headless server (no window, no
   rendering) that hosts on the configured port and generates the world
2. Running without `--server` shows the existing main menu (Play still works
   as a listen server; Join connects to a remote)
3. On connect, the client receives the tilemap and gas grid from the server
   and renders tiles and the atmos debug overlay from that data
4. Each client sends a name with its Hello message; the server creates a soul
   bound to a freshly spawned creature
5. A billboard nameplate floats above each creature showing the bound soul's
   display name
6. The bouncing ball is server-spawned with a NetId and its position replicates
   to all clients at 30 Hz
7. When a client disconnects, the soul unbinds but the creature entity remains
   in the world (visible to other clients, inert)
8. A second client connecting sees: all existing creatures (with nameplates),
   the ball, and correct tilemap/atmos state

## Strategy

The previous plan's post-mortem taught three lessons: draw the data flow before
designing the protocol, list systems not files in the layer table, and spike
ambiguous semantics. This plan follows all three.

**Multi-stream architecture.** Each QUIC connection carries multiple
independent streams, one per domain. QUIC guarantees reliable, in-order
delivery _within_ a stream (RFC 9000 §2.2) but not _between_ streams.
This gives each module its own ordered channel without head-of-line
blocking from other modules. Each QUIC stream is opened normally using
QUIC's own stream ID; the first byte of application data on that stream
is a module _stream tag_ (0/1/2/3 below) used for routing and is distinct
from QUIC's stream ID.

**Streams (server → client):**

| Stream tag | Owner          | Initial burst                | Ongoing (30 Hz)     |
| ---------- | -------------- | ---------------------------- | ------------------- |
| 0          | `network`      | Welcome, InitialStateDone    | —                   |
| 1          | `tiles`        | TilemapData + StreamReady    | (future: mutations) |
| 2          | `atmospherics` | GasGridData + StreamReady    | (future: updates)   |
| 3          | `things`       | EntitySpawned… + StreamReady | StateUpdate         |

**Streams (client → server):**

| Stream tag | Owner     | Content                             |
| ---------- | --------- | ----------------------------------- |
| 0          | `network` | Hello { name }, Input { direction } |

Stream tag 0 is a bidirectional control stream owned by the `network`
module. Server→client it carries `Welcome` and `InitialStateDone`;
client→server it carries `Hello` and `Input`. The `souls` module
writes `Hello` and `Input` through the network module's control stream
API, but does not own the stream itself.

**Connect handshake:**

```
Client opens stream 0:
  → Hello { name }

Server opens streams 0-3 to client:
  Stream 0 (control):  ← Welcome { client_id, expected_streams: 3 }
  Stream 1 (tiles):    ← TilemapData { width, height, tiles } ← StreamReady
  Stream 2 (atmos):    ← GasGridData { gas_moles }             ← StreamReady
  Stream 3 (things):   ← EntitySpawned (each existing entity)
                       ← EntitySpawned (new player's creature)
                       ← StreamReady

Client waits for InitialStateDone + all 3 StreamReady sentinels.
When all received → initial sync complete, game begins.
```

`InitialStateDone` is sent on the control stream after the server has
written all initial data to all module streams. The client considers
initial sync complete when it has received both `InitialStateDone` and
all `StreamReady` sentinels (one per module stream). This handles
transport-level reordering between streams.

**Ongoing replication (30 Hz):**

```
  Stream 3 (things):   ← StateUpdate { entities[] }
  Stream 0 (client→server): → Input { direction }
```

**Disconnect:**

```
  Server: unbind soul from creature, despawn soul entity,
          clear InputDirection on creature
       → EntityDespawned is NOT sent (creature stays)
       → Broadcast updated StateUpdate on stream 3 (creature now inert)
```

Work proceeds bottom-up: protocol changes first, then world state replication,
then soul binding, then nameplate rendering, then headless mode.

### Layer participation

| Layer | Module               | Systems / changes                                                                                                                                                                                                                                                                                                                                                                        |
| ----- | -------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| L0    | `network`            | Multi-stream connection lifecycle. `StreamRegistry` for modules to register named streams with a stream tag byte, direction, and message types. Opens/accepts streams per client, routes framed messages to per-stream Bevy events. Provides `StreamSender<T>` for modules to write to their stream. Control stream carries `Hello`, `Welcome { expected_streams }`, `InitialStateDone`. |
| L1    | `tiles`              | Registers stream 1 (server→client). Server-side: sends `TilemapData` + `StreamReady` on connect. Client-side: observes stream 1 messages, calls `Tilemap::from_bytes()`, inserts resource. Adds `to_bytes()` / `from_bytes()` serialization methods.                                                                                                                                     |
| L1    | `things`             | Registers stream 3 (server→client). `DisplayName(String)` component. Owns full client-side entity lifecycle: `EntitySpawned` (spawn via `SpawnThing`, insert `DisplayName`, track in `NetIdIndex`), `EntityDespawned`, `StateUpdate` (position sync). Server-side: broadcasts entity state on stream 3.                                                                                  |
| L2    | `atmospherics`       | Registers stream 2 (server→client). Server-side: sends `GasGridData` + `StreamReady` on connect. Client-side: observes stream 2 messages, calls `GasGrid::from_moles_vec()`, inserts resource. Adds `moles_vec()` / `from_moles_vec()` serialization methods.                                                                                                                            |
| L3    | `creatures`          | No changes — creatures are unaware of souls.                                                                                                                                                                                                                                                                                                                                             |
| L4    | `souls`              | **New module.** `Soul { name, client_id, bound_to }` component on a dedicated entity. `bind_soul` / `unbind_soul` systems. Replaces `ControlledByClient` and `PlayerControlled` as the authority on which client controls which creature. Writes `Hello` and `Input` to the control stream (tag 0) via network API. Depends on `creatures` and `network`.                                    |
| L4    | `player`             | Nameplate rendering: `spawn_nameplate` observer creates UI overlay entities, `update_nameplate_positions` projects from world space to screen space via `Camera::world_to_viewport()`. Nameplates are top-level UI entities with `NameplateTarget(Entity)`, not children of the 3D entity.                                                                                                                                                                                                                                                    |
| —     | `src/server.rs`      | On client connect: notify registered stream handlers. Sends `InitialStateDone` on control stream after all module streams have written initial data. `handle_disconnect`: despawn soul entity. Ball spawned with `NetId`.                                                                                                                                                                |
| —     | `src/client.rs`      | Tracks `StreamReady` count and `InitialStateDone` receipt. Initial sync complete when both conditions met. Thin — stream message routing handled by `network` module, domain logic in respective modules.                                                                                                                                                                                |
| —     | `src/main.rs`        | Parse `--server` CLI arg. When headless: use `MinimalPlugins` instead of `DefaultPlugins`, auto-host, skip main menu.                                                                                                                                                                                                                                                                    |
| —     | `src/world_setup.rs` | Becomes server-only (gated on `resource_exists::<Server>`). Ball gets `NetId`.                                                                                                                                                                                                                                                                                                           |

### Not in this plan

- **Full component replication framework.** EntitySpawned gets a `name` field;
  generalized reflection-based replication is future work.
- **Client-side prediction or interpolation.** Clients snap to server truth.
- **Name input UI.** Names come from config or a default; no text input widget.
- **Soul transfer / rebinding.** Souls bind once on connect and unbind on
  disconnect. No possession, no ghost mode.
- **Creature AI or idle animation.** Unbound creatures are inert (zero velocity).
- **Tilemap mutation replication.** That's arc step 2.

### Module placement

```
modules/
  network/
    src/
      lib.rs                   # MODIFIED — StreamRegistry, StreamSender<T>,
                               #   multi-stream connection lifecycle
      protocol.rs              # MODIFIED — Hello gains name, Welcome gains
                               #   expected_streams, InitialStateDone,
                               #   StreamReady, per-stream message types
      server.rs                # MODIFIED — open per-module streams on connect
      client.rs                # MODIFIED — accept per-module streams on connect
  tiles/src/lib.rs             # MODIFIED — to_bytes / from_bytes, stream 1
                               #   registration and handler
  things/src/lib.rs            # MODIFIED — DisplayName component, stream 3
                               #   registration and handler, NetIdIndex,
                               #   full EntitySpawned/Despawned/StateUpdate
  atmospherics/src/lib.rs      # MODIFIED — moles_vec / from_moles_vec, stream 2
                               #   registration and handler
  atmospherics/src/gas_grid.rs # MODIFIED — serialization helpers
  souls/                       # NEW MODULE (L4)
    Cargo.toml
    src/lib.rs                 # Soul { name, client_id, bound_to } component,
                               #   bind/unbind systems, Hello/Input via
                               #   control stream (tag 0)
  player/src/lib.rs            # MODIFIED — add nameplate rendering systems
src/
  server.rs                    # MODIFIED — connect orchestration, soul lifecycle,
                               #   InitialStateDone after streams flush
  client.rs                    # MODIFIED — StreamReady tracking, initial sync
                               #   barrier (expected_streams + InitialStateDone)
  world_setup.rs               # MODIFIED — server-only gate, ball gets NetId
  main.rs                      # MODIFIED — CLI parsing, headless mode
  config.rs                    # MODIFIED — player_name field
```

### Multi-stream design

Each QUIC connection carries multiple independent streams. Modules register
streams with the `network` module at startup via `StreamRegistry`, declaring
a stream tag byte, direction, and message types. The network module
handles connection lifecycle (opening/accepting streams, framing, routing)
while each module owns its own protocol over its assigned stream.

**`StreamRegistry` API (L0):**

```rust
app.world_mut().resource_mut::<StreamRegistry>()
    .register(StreamDef {
        tag: 1,
        name: "tiles",
        direction: StreamDirection::ServerToClient,
        // Network module opens/accepts the stream and routes framed
        // messages as Bevy events tagged with the stream tag.
    });
```

The network module opens all registered server→client streams when a client
connects (server side) or accepts them (client side). Each stream's first
byte is the stream tag, allowing the receiver to route it. Messages are
framed with `LengthDelimitedCodec` as today, but per-stream rather than
per-connection.

Modules write to their stream via `StreamSender<T>`, a typed resource
provided by the network module after stream setup. Modules read from their
stream via Bevy events emitted by the network module's async read loops.

**Stream-level protocol:**

Each module defines its own message enum for its stream. For example:

```
// Stream 1 (tiles)
TilesStreamMessage::TilemapData { width, height, tiles: Vec<u8> }
TilesStreamMessage::StreamReady

// Stream 2 (atmospherics)
AtmosStreamMessage::GasGridData { gas_moles: Vec<f32> }
AtmosStreamMessage::StreamReady

// Stream 3 (things)
ThingsStreamMessage::EntitySpawned { net_id, kind, position, velocity, name, owner }
ThingsStreamMessage::EntityDespawned { net_id }
ThingsStreamMessage::StateUpdate { entities: Vec<EntityState> }
ThingsStreamMessage::StreamReady
```

The `StreamReady` sentinel is the last message each module sends during
initial sync. The control stream (0) carries `Welcome`, `InitialStateDone`,
`Hello`, and `Input` — no domain-specific replication data.

**Initial sync barrier (client-side):**

The client tracks two conditions:

1. `InitialStateDone` received on control stream (means server has written
   all initial data to all module streams)
2. All `expected_streams` `StreamReady` sentinels received (one per module
   stream, handles transport-level reordering)

Initial sync is complete when both conditions are met. Until then, the
client may buffer or apply data as it arrives (tiles and gas grid can be
inserted immediately; they don't depend on ordering relative to entities).

**Why per-module streams:**

- **No head-of-line blocking.** A large tilemap transfer doesn't delay
  position updates. A stalled atmos stream doesn't block entity spawns.
- **Module isolation.** Each module owns its wire format, serialization,
  and message handling. No shared `ServerMessage` enum that grows with
  every feature. `network` stays at L0 — it knows about streams and
  framing, not about tiles, gas, or souls.
- **Independent flow control.** QUIC applies per-stream flow control,
  so a slow consumer on one stream doesn't back-pressure others.
- **Future-proof.** Adding a new replicating module means registering a
  new stream — no changes to existing modules or the network core.

**`ClientMessage::Hello`** gains a `name: String` field, sent on stream 0.
The server uses this to set the soul's display name.

**`Welcome`** gains `expected_streams: u8` — the number of `StreamReady`
sentinels the client should wait for.

**`EntitySpawned`** (stream 3) gains `name: Option<String>`. When present,
the `things` module inserts `DisplayName` on the entity.

### Soul design

A soul is its own ECS entity — the networked identity that a player controls.
It is not an entity in the world (no `Transform`, no physics, no mesh). It
exists purely as the binding between a client connection and a creature body.
This replaces the current `ControlledByClient` / `PlayerControlled` pattern
with a first-class identity concept.

Souls live in a new `souls` module at L4 (Mechanics). The module depends on
`creatures` (L3) and `network` (L0). Creatures have no knowledge of souls —
the dependency is strictly downward.

**Soul entity component (server-side):**

```rust
struct Soul {
    name: String,
    client_id: ClientId,
    bound_to: Entity,
}
```

A single component — every soul always has all three fields, so splitting
them into separate components would add query boilerplate for no benefit.

**Creature entity (unchanged by souls):**

- `Creature`, `Thing`, `NetId`, physics components — as before
- `InputDirection` — written by the souls module when input arrives for the
  bound client, zeroed on unbind
- `DisplayName(String)` — set by the souls module when binding, kept on unbind

The souls module provides the glue: when the server receives a `Hello` with a
name, it spawns a soul entity and a creature entity, binds them, and sets
`DisplayName` on the creature. When input arrives for a `ClientId`, the souls
module queries for the soul with that `client_id`, follows `bound_to` to
find the creature, and writes `InputDirection`. The creatures module reads
`InputDirection` as it always has — it does not know or care that a soul wrote
it.

**On disconnect:** The souls module despawns the soul entity and clears
`InputDirection` on the creature. The creature keeps its `Thing`, `Creature`,
`DisplayName`, `NetId`, and physics components. It continues to appear in
`StateUpdate` broadcasts (position won't change because `InputDirection` is
zeroed). Other clients see it standing still with its nameplate.

This is deliberately minimal. The architecture's full soul system supports
transfer, possession, and observer modes. This plan implements the seed —
bind on connect, unbind on disconnect — and defers the rest.

### Nameplate rendering

Nameplate rendering lives in the `player` module, not in `src/`. The `player`
module already handles player-specific concerns (input, `PlayerControlled`
marker) and nameplates are a player-facing presentation feature.

**Spike result:** Text2d renders in the 2D pipeline and requires a Camera2d.
It cannot be placed as a child of a 3D entity with only a Camera3d. Nameplates
use a world-to-viewport UI overlay approach instead.

Each entity with a `DisplayName` component gets a **top-level UI entity** (not
a child) with `Text`, `TextFont`, `TextColor`, an absolutely-positioned `Node`,
a `Nameplate` marker, and a `NameplateTarget(Entity)` linking it back to the
tracked 3D entity. The `update_nameplate_positions` system each frame:

1. Queries the tracked entity's `GlobalTransform` and adds a vertical offset
2. Projects from world space to screen space via `Camera::world_to_viewport()`
3. Centers the node by offsetting by half `ComputedNode::size()`
4. Rounds pixel values to reduce sub-pixel jitter
5. Hides the nameplate when the target is behind the camera

**Limitation (future work):** `world_to_viewport` uses the local camera, so
nameplate positions are computed client-side. This is correct — each client
projects to its own screen — but the nameplate task (#118) should note that
nameplates are client-only UI and must not be replicated.

### Headless server mode

Parse `--server` from `std::env::args()` at startup. When set:

1. Replace `DefaultPlugins` with `MinimalPlugins` (includes time, scheduling)
   plus `AssetPlugin`, `ScenePlugin` if needed by physics
2. Skip `WindowPlugin`, `UiPlugin`, `MainMenuPlugin`, `CameraPlugin`,
   `TilesPlugin` visual systems, `AtmosphericsPlugin` debug overlay
3. Set initial state to `AppState::InGame` (skip menu)
4. Auto-send `NetCommand::Host { port }` on startup

**Spike needed:** Verify that Avian3D physics works without `DefaultPlugins`
(specifically without rendering). If it requires `AssetPlugin` or `TypeRegistry`
setup, identify the minimal plugin set.

### Ball replication

Currently the ball is spawned in `world_setup.rs` without a `NetId`. Change:

1. Server spawns ball with `NetId` from `server.next_net_id()`
2. Register ball as thing kind 1 in `ThingRegistry` (kind 0 = creature)
3. Broadcast `EntitySpawned { kind: 1, ... }` to all clients
4. Ball included in `StateUpdate` broadcasts (already queries all `NetId` entities)
5. Client spawns ball via `SpawnThing` with kind 1 (sphere mesh, dynamic body)

### Client world setup changes

Currently `world_setup.rs` runs `setup_world` on `OnEnter(InGame)` for all
instances. After this plan:

- **Server/listen server:** `setup_world` still runs (generates tilemap, gas
  grid, light, ball) but ball now gets a NetId
- **Client:** `setup_world` does NOT run. Instead, `handle_world_state` in
  `client.rs` inserts `Tilemap` and `GasGrid` from server data. Tiles spawn
  via the existing `spawn_tile_meshes` change-detection system. The light is
  spawned client-side in a small `setup_client_scene` system (just lighting,
  no world state).

Gate `setup_world` with `.run_if(resource_exists::<Server>)`.

## Spikes

1. ~~**Quinn multi-stream**~~ **Done** (PR #120). Three `open_uni()` streams
   from one connection, tag-byte routing, independent `LengthDelimitedCodec`
   framing, and `StreamReady` sentinels all work as designed. Notable:
   the server must `connection.closed().await` (not drop) to avoid a
   reset race that truncates in-flight stream data. No new dependencies
   needed — Quinn's `SendStream`/`RecvStream` expose `write_all`/`read_exact`
   as inherent methods.

2. ~~**Bevy 0.18 billboard text**~~ — **Resolved.** Text2d requires Camera2d
   and cannot billboard under Camera3d. Use world-to-viewport UI overlay
   instead. See #109.

3. **Headless Avian3D** — Does `PhysicsPlugin` work with `MinimalPlugins`
   instead of `DefaultPlugins`? Spawn a dynamic body and step the schedule
   twice. 30 min.

## Post-mortem

_To be filled in after the plan ships._
