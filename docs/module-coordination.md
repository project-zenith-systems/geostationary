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

1. **`NetworkSet::Receive`** — `drain_server_events` and `drain_client_events`
   pull messages from async channels into Bevy messages. On the client,
   `StreamReady` sentinels received in this batch are *deferred* (buffered in
   `StreamRegistry`) and emitted at the **start of the next frame's**
   `drain_client_events` call, guaranteeing at least one full frame of
   processing before downstream systems see the ready signal.

2. **Module on-connect sends** — Server-side systems that send initial data
   bursts to newly joined clients. These are ordered after
   `NetworkSet::Receive` (so `PlayerEvent::Joined` is readable) and before
   `NetworkSet::Send`. The canonical execution order within this window is
   declared via `SystemSet` constraints (see [Ordering Convention](#ordering-convention)
   below):

   ```
   TilesSet::SendOnConnect          ─┐
   AtmosSet::SendOnConnect           ├─ before ──▶ ThingsSet::HandleClientJoined
                                     │
   ThingsSet::HandleClientJoined    ─┘
        │
        ▼
   items::broadcast_stored_on_join   (after HandleClientJoined, before SendStreamReady)
        │
        ▼
   ThingsSet::SendStreamReady        (after HandleClientJoined)
        │
        ▼
   souls::bind_soul                  (after HandleClientJoined)
   ```

3. **`NetworkSet::Send`** — `process_net_commands` dispatches queued
   `NetCommand` messages to async tasks (host, connect, disconnect).

### StateTransition (between PreUpdate and Update)

Bevy processes pending state transitions (e.g. `NextState::set`). Systems
registered with `OnEnter(InGame)` run here — notably `setup_client_scene`
on the client.

### Update

Steady-state game logic: physics input, simulation ticks, interaction
dispatch, UI, and periodic broadcasts.

- `broadcast_state` and `broadcast_item_event` (things, server)
- `broadcast_gas_grid` (atmospherics, server — periodic snapshots/deltas)
- `handle_item_interaction` (items, server)
- `track_module_ready` (shared/server — collects `ModuleReadySent` events)
- Client-side: `handle_item_event`, `send_input`, debug overlays

### FixedUpdate

Runs at the simulation tick rate, independent of frame rate.

- `wall_sync_system` → `diffusion_step_system` → `apply_pressure_forces`
  (atmospherics, server)
- `wall_sync_system` → `diffusion_step_system` (atmospherics, client)

### PostUpdate

Post-processing that must run after all `Update` systems.

- `apply_tile_mutation` (tiles, listen-server only — after
  `dispatch_interaction` writes `TileMutated` events in `Update`)

## Readiness Convention: "Module X Is Ready"

Every registered server→client module stream follows the same protocol:

1. **Initial burst.** On `PlayerEvent::Joined`, the module's on-connect
   system sends all catch-up data for the joining client on its stream
   (e.g. `TilemapData`, `GasGridData`, `EntitySpawned` + `ItemEvent`).

2. **`StreamReady` sentinel.** After the burst, the module calls
   `StreamSender::send_stream_ready_to(client)` to send a special
   sentinel frame on the wire.

3. **`ModuleReadySent` event.** The module emits `ModuleReadySent { client }`
   so the orchestration layer can track progress.

4. **Orchestration.** `track_module_ready` (in `shared/server.rs`) counts
   `ModuleReadySent` per client. Once the count equals the number of
   registered server→client streams (`StreamRegistry::server_to_client_count()`),
   it sends `ServerMessage::InitialStateDone` on the control stream.

5. **Client barrier.** The client's `PendingSync` tracks both
   `InitialStateDone` and per-stream `StreamReady` sentinels. The
   transition to `AppState::InGame` fires only once both conditions are
   satisfied.

### StreamReady Deferral (Client-Side)

When `drain_client_events` encounters a `StreamReady` sentinel in the same
batch as the stream data it marks complete, the sentinel is **deferred** to
the next frame. This ensures module systems have had a full frame to process
the initial-burst data before any downstream system observes the ready
signal. The deferral is transparent — modules do not need to account for it.

### Adding a New Module Stream

To add a new server→client module stream that participates in the
initial-sync barrier:

1. Register the stream with `StreamRegistry::register` in your plugin's
   `build()`.
2. Implement an on-connect system gated on `resource_exists::<Server>` that
   listens for `PlayerEvent::Joined`, sends the initial data, then calls
   `send_stream_ready_to` and emits `ModuleReadySent`.
3. Place the system in an appropriately named `SystemSet` variant (see
   below) and configure it in `PreUpdate` between `NetworkSet::Receive` and
   `NetworkSet::Send`.
4. If the required resource may not exist on the first frame (e.g.
   listen-server startup), queue the client and retry, following the
   `PendingTilesSyncs` / `PendingAtmosSyncs` pattern.
5. If ordering relative to other modules' on-connect systems matters,
   declare the constraint in `shared/server.rs` where all module types are
   visible.

## Ordering Convention

### SystemSets

Cross-module ordering is expressed through named `SystemSet` enums. Each
module that exposes ordering points declares a public enum with descriptive
variants. This decouples ordering from private function names and makes
constraints visible in code.

| Module          | Set                            | Purpose |
|-----------------|--------------------------------|---------|
| `network`       | `NetworkSet::Receive`          | Drains async events into Bevy messages |
| `network`       | `NetworkSet::Send`             | Dispatches outbound commands |
| `tiles`         | `TilesSet::SendOnConnect`      | Tilemap snapshot + StreamReady to joining client |
| `atmospherics`  | `AtmosSet::SendOnConnect`      | Gas grid snapshot + StreamReady to joining client |
| `things`        | `ThingsSet::HandleClientJoined`| Entity-spawn catch-up for joining client |
| `things`        | `ThingsSet::SendStreamReady`   | Stream 3 StreamReady sentinel (after all catch-up) |

### Where Constraints Live

- **Intra-module constraints** are declared in the module's own plugin
  `build()`. Example: `ThingsSet::SendStreamReady.after(ThingsSet::HandleClientJoined)`.

- **Cross-module constraints** are declared in `shared/server.rs`
  (`ServerPlugin::build`), the only crate that depends on all module crates.
  Example: `TilesSet::SendOnConnect.before(ThingsSet::HandleClientJoined)`.

- **Layer-dependency constraints** follow the architecture's downward-only
  dependency rule. A higher-layer module may `.after()` a lower-layer set
  directly since it already depends on that crate.
  Example: `items::broadcast_stored_on_join.after(things::ThingsSet::HandleClientJoined)`.

### Current On-Connect Ordering Graph

```
NetworkSet::Receive
        │
        ▼
TilesSet::SendOnConnect  ──────┐
AtmosSet::SendOnConnect  ──────┤ (world-state streams, unordered with each other)
        │                      │
        ▼                      │
ThingsSet::HandleClientJoined ◀┘
        │
        ├──▶ items::broadcast_stored_on_join
        │
        ▼
ThingsSet::SendStreamReady
        │
        ├──▶ souls::bind_soul
        │
        ▼
NetworkSet::Send
```

### Adding a New Ordering Constraint

1. If your module's on-connect system must run before or after another
   module's, express this as `.before(OtherSet::Variant)` or
   `.after(OtherSet::Variant)`.
2. If the other module is in a lower layer (your crate already depends on
   it), add the constraint directly in your plugin's `build()`.
3. If neither module depends on the other, add the constraint in
   `shared/server.rs` where both types are visible.
4. Never invent a new coordination mechanism (ad-hoc events, frame-counting,
   polling resources) when a `SystemSet` ordering constraint will do.

## Summary of Coordination Mechanisms

| Mechanism | Scope | Used For |
|-----------|-------|----------|
| `SystemSet` ordering | PreUpdate (server) | Deterministic ordering of on-connect sends |
| `ModuleReadySent` event | PreUpdate/Update | Per-module "initial burst complete" signal |
| `StreamReady` sentinel | Wire protocol | Per-stream "data burst complete" marker (client-side) |
| `StreamReady` deferral | Client PreUpdate | Ensures one processing frame before ready signal |
| `PendingSync` barrier | Client PreUpdate | Gate for `AppState::InGame` transition |
| `PendingXSyncs` queues | Server PreUpdate | Retry queue when resource not yet available |
