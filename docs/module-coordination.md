# Module Coordination

Reference for cross-module scheduling, the game-loop timeline, and the
conventions that govern how modules signal readiness and declare ordering
constraints.

## Game-Loop Timeline

Every Bevy frame proceeds through a fixed sequence of schedules. The
subsections below describe what happens in each phase, focusing on the
network and module systems that participate in client–server coordination.

### PreUpdate

The earliest schedule in which game code runs. Bevy's internal bookkeeping
(time, events, etc.) has already executed.

### NetworkReceive (custom schedule, after PreUpdate)

Drains inbound network data and processes it in a single schedule. Ordering
within the schedule uses `NetworkSet` sets:

1. **`NetworkSet::Drain`** — `drain_server_events` and `drain_client_events`
   pull messages from async channels into Bevy messages.

2. **Module on-connect sends** — Server-side systems that react to
   `PlayerEvent::Joined` by sending initial data to newly joined clients.
   Each module independently listens for the event and sends its data when
   ready. These systems run after `NetworkSet::Drain` and before
   `NetworkSet::Commands` (the default slot — no explicit set annotation
   needed).

3. **`NetworkSet::Commands`** — `process_net_commands` dispatches connection
   lifecycle commands (`NetCommand::Host`, `Connect`, `Disconnect`) to
   async tasks. This does **not** send game data — module stream writes go
   directly to async channels and are schedule-independent.

### StateTransition (between NetworkReceive and Update)

Bevy processes pending state transitions (e.g. `NextState::set`). Systems
registered with `OnEnter(InGame)` run here — notably `setup_client_scene`
on the client.

### Update

Steady-state game logic. This is where the bulk of gameplay systems run.

- **Gameplay** — interaction dispatch, item handling, input processing
- **Visual** — animation playback, particle effects, debug overlays
  (client/listen-server only)
- **Orchestration** — `track_module_ready` collects `ModuleReadySent`
  events and sends `InitialStateDone` when all streams are ready

### FixedUpdate

Runs at the simulation tick rate, independent of frame rate.

- **Physics** — velocity integration, force application
- **Atmospherics** — wall sync, gas diffusion, pressure forces

### PostUpdate

Runs after all `Update` systems have finished. Use this for work that
depends on the final state of the frame.

- **Deferred mutations** — `apply_tile_mutation` (listen-server only)

### NetworkSend (custom schedule, after PostUpdate)

Modules place their outbound broadcast / replication systems here so that
all gameplay systems in `Update` and late mutations in `PostUpdate` have
committed their changes before the state is serialised and sent to clients.

- **Network broadcasts** — periodic state sync (entity positions, gas grid
  snapshots, item events)

> **Rationale:** Running outbound broadcasts in `NetworkSend` (after
> `PostUpdate`) guarantees that every system has had a chance to modify the
> world state before it is synced to clients. This avoids sending stale or
> partially-updated data.

## Server-Only and Client-Only Systems

The network layer inserts marker resources that identify the role of the
current application instance. Systems use these as run conditions to ensure
they only execute in the appropriate context.

| Resource    | Present When                            |
|-------------|-----------------------------------------|
| `Server`    | Hosting (dedicated server or listen-server) |
| `Client`    | Connected to a server (client or listen-server) |
| `Headless`  | Dedicated server mode (no rendering)    |

Note: On a listen-server, both `Server` and `Client` are present.

### Run Condition Patterns

```rust
// Server-only: authoritative simulation, broadcasting state
app.add_systems(NetworkSend, broadcast_state.run_if(resource_exists::<Server>));

// Client-only: receiving replicated state, sending input
app.add_systems(Update, send_input.run_if(resource_exists::<Client>));

// Visual-only: rendering, VFX, animation — skip on headless servers
app.add_systems(Update, spawn_meshes.run_if(not(resource_exists::<Headless>)));

// Client-side receiving (runs in NetworkReceive after drain)
app.add_systems(NetworkReceive, handle_entity_lifecycle.run_if(resource_exists::<Client>));
```

### When to Use Each

- **`resource_exists::<Server>`** — Systems that run authoritative logic:
  handling player joins, processing interactions, broadcasting state. These
  run on dedicated servers and listen-servers.

- **`resource_exists::<Client>`** — Systems that process replicated data or
  send client input. These run on clients and listen-servers.

- **`not(resource_exists::<Headless>)`** — Systems that require rendering
  or windowing: spawning meshes, playing animations, showing debug overlays.
  These skip on headless dedicated servers but run on clients and
  listen-servers (which have a window).

### Module Scope

Some modules are entirely server-side or client-side by nature:

- **Server-only modules** gate all their systems on `Server`. Example:
  interaction dispatch, physics authority, atmos simulation stepping.
- **Client-only modules** gate on `Client` or `not(Headless)`. Example:
  input capture, camera control, UI rendering.
- **Shared modules** register both server and client systems, gating each
  appropriately. Example: `things` registers `broadcast_state` for `Server`
  and `handle_entity_lifecycle` for `Client`.

## Readiness Convention: "Module X Is Ready"

Every registered server→client module stream follows the same protocol:

1. **Initial burst.** On `PlayerEvent::Joined`, the module's on-connect
   system sends catch-up data for the joining client on its stream
   (e.g. `TilemapData`, `GasGridData`, `EntitySpawned` + `ItemEvent`).
   The initial burst **may span multiple frames** — modules must not assume
   all data is sent in a single frame. For large worlds, data may be
   streamed in chunks across several frames before the module signals
   readiness.

2. **`StreamReady` guard.** Once the module has finished sending all of its
   initial data (which may have taken several frames), it calls
   `StreamSender::send_stream_ready_to(client)` to send a guard frame on
   the wire marking the end of the initial burst.

3. **`ModuleReadySent` event.** The module emits `ModuleReadySent { client }`
   so the orchestration layer can track progress.

4. **Orchestration.** `track_module_ready` (in `shared/server.rs`) counts
   `ModuleReadySent` per client. Once the count equals the number of
   registered server→client streams (`StreamRegistry::server_to_client_count()`),
   it sends `ServerMessage::InitialStateDone` on the control stream.

5. **Client barrier.** The client's `PendingSync` tracks both
   `InitialStateDone` and per-stream `StreamReady` guards. The transition
   to `AppState::InGame` fires only once both conditions are satisfied.

> **Important:** Do not assume that a module's initial burst completes in
> one frame. The `StreamReady` guard is the only reliable signal that a
> module has finished sending its initial data. Both the server
> orchestration (`ModuleReadySent` counting) and the client barrier
> (`PendingSync`) are designed to work correctly regardless of how many
> frames the initial burst spans.

### Adding a New Module Stream

To add a new server→client module stream that participates in the
initial-sync barrier:

1. Register the stream with `StreamRegistry::register` in your plugin's
   `build()`.
2. Implement an on-connect system gated on `resource_exists::<Server>` that
   listens for `PlayerEvent::Joined`, sends the initial data, then calls
   `send_stream_ready_to` and emits `ModuleReadySent`.
3. Place the system in the `NetworkReceive` schedule.
4. If the required resource may not exist on the first frame (e.g.
   listen-server startup), queue the client and retry, following the
   `PendingTilesSyncs` / `PendingAtmosSyncs` pattern.

## Ordering Convention

### SystemSets

Ordering between systems is expressed through named `SystemSet` enums. Each
module that exposes ordering points declares a public enum with descriptive
variants. This decouples ordering from private function names and makes
constraints visible in code.

| Module          | Set / Schedule                 | Purpose |
|-----------------|--------------------------------|---------|
| `network`       | `NetworkReceive` (schedule)    | Drains inbound messages, processes them, dispatches commands |
| `network`       | `NetworkSend` (schedule)       | Outbound broadcasts after all gameplay has run |
| `network`       | `NetworkSet::Drain`            | Ordering set within `NetworkReceive`: drain systems run first |
| `network`       | `NetworkSet::Commands`         | Ordering set within `NetworkReceive`: command dispatch runs last |
| `tiles`         | `TilesSet::SendOnConnect`      | Tilemap snapshot + StreamReady guard to joining client |
| `atmospherics`  | `AtmosSet::SendOnConnect`      | Gas grid snapshot + StreamReady guard to joining client |
| `things`        | `ThingsSet::HandleClientJoined`| Entity-spawn catch-up for joining client |
| `things`        | `ThingsSet::SendStreamReady`   | Stream 3 StreamReady guard (after all catch-up) |

### Where Constraints Live

Each module declares its own ordering constraints in its plugin `build()`.
If a module needs to run after another module's set, it adds the constraint
itself — this keeps ordering decisions co-located with the systems they
affect.

```rust
// In items/src/lib.rs — items depends on things, so it can reference ThingsSet directly
app.add_systems(
    NetworkReceive,
    broadcast_stored_on_join
        .run_if(resource_exists::<Server>)
        .after(things::ThingsSet::HandleClientJoined)
        .before(things::ThingsSet::SendStreamReady),
);
```

### Adding a New Ordering Constraint

When a new system needs to run in a defined order relative to other modules:

1. Declare a `SystemSet` variant in your module if one doesn't already exist.
2. Add `.before()` or `.after()` constraints referencing the other module's
   set in your plugin's `build()`.

## Summary of Coordination Mechanisms

| Mechanism | Scope | Used For |
|-----------|-------|----------|
| `SystemSet` ordering | `NetworkReceive` (server) | Deterministic ordering of on-connect sends |
| `ModuleReadySent` event | `NetworkReceive`/`Update` | Per-module "initial burst complete" signal |
| `StreamReady` guard | Wire protocol | Per-stream "data burst complete" marker (client-side) |
| `PendingSync` barrier | Client `NetworkReceive` | Gate for `AppState::InGame` transition |
| `PendingXSyncs` queues | Server `NetworkReceive` | Retry queue when resource not yet available |
| `resource_exists` run conditions | All schedules | Server-only / client-only / visual-only gating |
