# Plan: Networked Multiplayer

> **Stage goal:** Two game instances communicate over the wire. The host runs a
> server-authoritative simulation — clients send input, the server runs physics
> for all peers and sends authoritative positions back. The L0 network module
> owns both transport and protocol — typed messages in domain-neutral terms, with
> serialization handled internally.

## What "done" looks like

1. Clicking Play hosts a listen server and spawns the host player (existing
   single-player experience preserved)
2. A "Join" button on the title screen connects to `localhost:<configured port>`
   and transitions to InGame
3. The joining client sees its own player and the host player in the room
4. WASD movement on either instance is visible on both screens
5. Movement is server-authoritative: the host runs physics, clients receive
   positions
6. A `NetworkRole` resource (`ListenServer` / `Client` / `None`) gates system
   behaviour
7. Client disconnection despawns the remote player on the host; host shutdown
   returns the client to the main menu
8. The network module's protocol uses domain-neutral terms (peers, positions,
   input vectors) — game code maps these to its own concepts
9. No `tokio`/`quinn`/`mpsc` types appear outside `modules/network/`
10. Existing tests pass; new tests cover protocol serialization and data channels

## Strategy

This plan sends game data over the wire for the first time. The network module
owns both transport and protocol — the same way the physics module owns both
the engine wrapper and the physics primitives (rigid bodies, colliders) without
knowing they represent creatures or walls. The protocol defines peers, spatial
state, and input vectors; the game code decides what those primitives represent.

Serialization (serde + bincode) lives entirely inside the network module. Upper
layers send and receive typed protocol messages, never raw bytes. This pushes
the serialization concern down to L0 where it belongs — game code doesn't need
bincode as a dependency.

Work proceeds bottom-up: extend the network module with protocol types and data
channels, rewire player spawning through the network flow, add the Join button,
then wire the full input→state loop.

Lessons from previous post-mortems: sequence PRs explicitly (physics-foundation),
decide entity ownership up front (playable-character), follow the branching
convention, and write tests not just visual checks.

### Layer participation

| Layer | Module | Plan scope |
|-------|--------|------------|
| L0 | `network` | `PeerId` type, `HostMessage`/`PeerMessage`/`PeerState` protocol types, `NetServerSender`/`NetClientSender` resources for typed messages, `NetEvent` variants for protocol delivery, serde+bincode serialization, `LengthDelimitedCodec` stream framing (from tokio-util), bi-directional QUIC streams per client |
| L0 | `physics` | Unchanged |
| L0 | `ui` | Unchanged |
| L1 | `tiles` | Unchanged |
| L1 | `things` | Unchanged |
| L1 | `main_menu` | `MenuEvent::Join` variant, Join button on title screen |
| L3 | `creatures` | `creature_movement_system` gated to run only on `ListenServer` |
| L6 | `camera` | Unchanged (follows `PlayerControlled`) |
| — | `net_game.rs` | **New.** Systems bridging network ↔ game state |
| — | `world_setup.rs` | Remove player capsule spawn (moves to network flow) |
| — | `main.rs` | `NetworkRole` resource, `net_game` systems, Join handling |

### Not in this plan

- **Client-side prediction / interpolation.** Clients snap to server positions.
- **Multiple simultaneous clients beyond two.** Architecture supports it, but
  testing focuses on one host + one joiner.
- **Unreliable channels (datagrams).** All data uses reliable bi-di streams.
- **Dedicated server mode.** Host is always a listen server.
- **Authentication, lobby, or remote connections.** Join is localhost-only.
- **Enhanced physics determinism.** Not needed until cross-machine reconciliation.
- **Reconnection.** Dropped connection returns to menu.

### Module placement

```
modules/
  network/
    Cargo.toml      # MODIFIED — add serde, bincode
    src/
      lib.rs        # MODIFIED — new NetEvent variants, PeerId, sender resources
      server.rs     # MODIFIED — bi-di streams, read/write loops, PeerId map
      client.rs     # MODIFIED — bi-di stream, read/write loops
      runtime.rs    # MODIFIED — sender channels in NetworkTasks
      protocol.rs   # NEW — HostMessage, PeerMessage, PeerState, encode/decode
      config.rs     # Unchanged
src/
  net_game.rs       # NEW — systems: input send, state broadcast, player join/leave
  world_setup.rs    # MODIFIED — remove player spawn
  creatures/mod.rs  # MODIFIED — gate movement on NetworkRole
  main_menu/
    mod.rs          # MODIFIED — handle MenuEvent::Join
    title_screen.rs # MODIFIED — add Join button
  main.rs           # MODIFIED — NetworkRole, net_game registration, Join flow
```

### Network protocol design

The protocol lives in `modules/network/src/protocol.rs` and uses domain-neutral
terminology. The same way the physics module provides `RigidBody` and `Collider`
without knowing they are creatures or walls, the network module provides protocol
primitives that the game maps to its own domain.

**Host → peer messages (`HostMessage`):**
- `Welcome { peer_id: PeerId }` — assigns the connecting peer their identity
- `PeerJoined { id: PeerId, position: [f32; 3] }` — a new peer entered
- `PeerLeft { id: PeerId }` — a peer disconnected
- `StateUpdate { peers: Vec<PeerState> }` — authoritative spatial state

**Peer → host messages (`PeerMessage`):**
- `Input { direction: [f32; 3] }` — input vector from the peer

**Shared types:**
- `PeerId` — `u64` newtype, `Copy`/`Eq`/`Hash`
- `PeerState { id: PeerId, position: [f32; 3], velocity: [f32; 3] }`

Serialization is internal to the module — `bincode` encodes/decodes these types.
Game code sends and receives typed messages, never raw bytes.

### Network data channel design

Two new resources for sending typed protocol messages:

- **`NetServerSender`** — methods: `send_to(peer, &HostMessage)` and
  `broadcast(&HostMessage)`. Internally serializes to bytes and routes via mpsc.
  Inserted when the server task starts, removed on stop.
- **`NetClientSender`** — method: `send(&PeerMessage)`. Internally serializes
  and sends via mpsc. Inserted when the client task starts, removed on disconnect.

Incoming messages arrive as typed `NetEvent` variants:
- `NetEvent::HostMessageReceived(HostMessage)` — client receives from host
- `NetEvent::PeerMessageReceived { from: PeerId, message: PeerMessage }` — host
  receives from a peer
- `NetEvent::PeerConnected { id: PeerId, addr: SocketAddr }` — replaces
  `ClientConnected`
- `NetEvent::PeerDisconnected { id: PeerId }` — new variant

Stream framing uses `tokio_util::codec::LengthDelimitedCodec` (already in the
dependency tree via `tokio-util = "0.7"`). QUIC bi-directional streams expose
separate read and write halves (`RecvStream` / `SendStream`), which are wrapped
in `FramedRead<RecvStream, LengthDelimitedCodec>` and `FramedWrite<SendStream,
LengthDelimitedCodec>` for automatic length-prefixed message delimiting. No
custom framing code needed.

The server maintains a `HashMap<PeerId, FramedWrite<SendStream,
LengthDelimitedCodec>>` internally for outbound messages. When `send_to(peer,
msg)` is called, the message is serialized and routed to the correct peer's
write handle. `broadcast` iterates all peers. Each peer also has a read loop
using `FramedRead<RecvStream, LengthDelimitedCodec>` for inbound messages.

### Player lifecycle

Player entities are spawned by the network flow, **not** by `setup_world`:

**Host (ListenServer):**
- `OnEnter(InGame)` + `ListenServer` → spawn host player (Dynamic body,
  `PlayerControlled`, `Creature`, `MovementSpeed`, `PeerId(0)` component)
- `PeerConnected { id }` → spawn remote player (Dynamic body, `Creature`,
  `MovementSpeed`, assigned `PeerId` component, **no** `PlayerControlled`). Send
  `Welcome` + `PeerJoined` for all existing peers to new peer. Send
  `PeerJoined` for new peer to all peers.
- `PeerDisconnected { id }` → despawn entity, broadcast `PeerLeft`

**Client:**
- `PeerJoined { id, pos }` → spawn entity (Kinematic body, no physics). If
  `id == my_id`, add `PlayerControlled` so the camera follows it.
- `PeerLeft { id }` → despawn entity
- `StateUpdate` → set `Transform.translation` on matching entities

### Server-authoritative input/state loop

**Host each frame:**
1. `creature_movement_system` reads keyboard → sets host player `LinearVelocity`
2. `apply_remote_input` reads `PeerMessageReceived` → extracts `Input` direction
   → sets remote player `LinearVelocity` via `MovementSpeed`
3. Avian physics steps all Dynamic bodies
4. `broadcast_state` collects all player positions → sends `StateUpdate` via
   `NetServerSender.broadcast()`

**Client each frame:**
1. `send_client_input` reads keyboard → sends `PeerMessage::Input` via
   `NetClientSender`
2. `receive_server_state` reads `HostMessageReceived` → applies `PeerJoined` /
   `PeerLeft` / `StateUpdate` to entities

`creature_movement_system` is gated:
`.run_if(|role: Res<NetworkRole>| *role == NetworkRole::ListenServer)`

### Join flow

Title screen gains a "Join" button → `MenuEvent::Join`. Handler sends
`NetCommand::Connect { addr: localhost:port }` (from `AppConfig`). Shows loading
screen. On `NetEvent::Connected`, set `NetworkRole::Client`, transition to
`InGame`. Parallels the Play flow but skips hosting.

`handle_net_events` extended: on `HostingStarted`, also set
`NetworkRole::ListenServer`.

### Dependencies

- Add `serde = { version = "1", features = ["derive"] }` and `bincode = "1"` to
  `modules/network/Cargo.toml`
- No new dependencies in root `Cargo.toml`

## Post-mortem

### Outcome

The plan shipped what it promised. Two game instances communicate over QUIC
using a server-authoritative model: the host runs physics, clients send input
and receive positions. Protocol types, serialization, sender resources, and the
full input→state loop all work as designed. The network module boundary held —
no tokio/quinn types leak. However, the plan branch has two compilation errors
in tests and two dead-code warnings that must be fixed before the squash-merge
into main.

### What shipped beyond the plan

| Addition | Why |
|----------|-----|
| `bytes` and `futures-util` crate dependencies in `modules/network/Cargo.toml` | Required by `FramedRead`/`FramedWrite`/`SinkExt`/`StreamExt` for the length-delimited codec wrapping of QUIC streams. Not anticipated in the plan's dependency section. |
| 30 Hz network update throttling (`StateBroadcastTimer`, `InputSendTimer`) | Without throttling, state broadcasts and input sends fire every frame (~144 Hz+), flooding the wire. Not mentioned in the plan but essential for any practical use. |
| `MissingHostPlayerWarned` resource | Prevents log spam when the host player entity hasn't been created yet. Minor defensive addition. |
| Per-peer bounded write channels (`PER_PEER_BUFFER_SIZE`) and `CLIENT_BUFFER_SIZE` | Provides backpressure and prevents memory exhaustion from slow peers or fast game code. The plan described the mpsc routing but didn't specify bounded vs unbounded per-peer. |
| Per-peer `CancellationToken` for coordinated read/write loop shutdown | Ensures both the read and write tasks for a peer stop cleanly when either one exits. Not in the plan but necessary for correct async lifecycle. |
| `MAX_NET_EVENTS_PER_FRAME` cap with warning deduplication | Prevents the drain loop from stalling the game if a burst of network events arrives. |

### Deviations from plan

- **Listen server connects to itself.** The plan said "OnEnter(InGame) +
  ListenServer → spawn host player." The implementation instead has the host
  send `NetCommand::Connect { addr: localhost }` immediately after
  `HostingStarted`, making the host a peer of its own server. The host player
  is spawned by `handle_peer_connected` when the self-connection arrives, then
  `spawn_host_player` tags it with `PlayerControlled`. This is architecturally
  cleaner (one code path for all peers) but diverges from the plan's direct
  spawn model.

- **Host PeerId is 1, not 0.** The plan specified `PeerId(0)` for the host
  player. Because the host connects as a regular peer, it gets the first
  auto-incremented ID (`PeerId(1)`). The `LocalPeerId` resource tracks which
  ID belongs to "us" regardless of value.

- **`spawn_host_player` doesn't spawn.** It only tags an existing entity with
  `PlayerControlled` once the `LocalPeerId` is known. The actual entity spawn
  happens in `handle_peer_connected`. The plan described a direct spawn system.

- **`FramedRead`/`FramedWrite` instead of `Framed`.** QUIC bi-directional
  streams expose separate `RecvStream`/`SendStream` halves, so wrapping them
  individually is necessary. The plan was updated mid-flight to reflect this
  (the original text said `Framed<stream, LengthDelimitedCodec>`).

- **`creature_movement_system` run condition.** The plan specified
  `.run_if(|role: Res<NetworkRole>| *role == NetworkRole::ListenServer)`.
  Implementation uses a named function `is_listen_server_in_game` that also
  checks `AppState::InGame`. This is arguably better (prevents the system from
  running during menus) but differs from the plan.

- **`receive_host_messages` runs on both Client and ListenServer.** The plan
  implied client-only. The implementation guards against double-spawning with
  `if matches!(*network_role, NetworkRole::ListenServer) { continue; }` checks
  inside the handler. This is needed because the listen server receives its own
  broadcast messages via its self-connection.

- **No `TODO.md` on the plan branch.** The plan-guide says to create a
  `TODO.md` that the workflow converts to issues. The tasks were created as PRs
  directly without a `TODO.md` intermediary.

### Hurdles

1. **QUIC bi-directional stream halves can't share a `Framed` wrapper.** Quinn
   exposes `RecvStream` and `SendStream` as separate types, so
   `Framed<BiStream, Codec>` doesn't work. Had to use `FramedRead` and
   `FramedWrite` separately. **Lesson:** Check the concrete types of the async
   I/O library before planning codec wrapping.

2. **Server accept loop blocks command processing.** The original design had
   `endpoint.accept()` in a simple loop. Server commands (`SendTo`,
   `Broadcast`) need to be processed concurrently with connection acceptance.
   Solved with `tokio::select!` over both `endpoint.accept()` and
   `server_cmd_rx.recv()`. **Lesson:** Any server that both accepts connections
   and routes messages needs a multiplexed event loop, not sequential steps.

3. **Host needs to be its own peer.** The plan's direct-spawn model for the
   host player would have created a separate code path from remote peers.
   Connecting the host to itself unifies spawning, state broadcast, and input
   handling. The cost is one extra local QUIC connection, which is negligible.
   **Lesson:** Favour uniform code paths over special cases, even if it means
   a small architectural deviation from the plan.

4. **`Timer::finished()` became a private field in Bevy 0.18.** Two tests
   assert `!timer.0.finished()` but this is no longer a public method.
   **Lesson:** Always compile tests against the target Bevy version before
   merging.

### Remaining open issues

| Issue | Impact | Notes |
|-------|--------|-------|
| Player character is not controllable | Blocker | Race condition between `PeerConnected` event and peer write-channel registration in the server task. The server sends `PeerConnected` to Bevy (step 2 in the per-connection task) *before* the bi-di stream is accepted and the write channel is registered in `peer_senders` (step 4). When `handle_peer_connected` reacts by sending `Welcome` via `NetServerSender`, the server command loop can't find the peer in the senders map and silently drops the message. Without `Welcome`, `LocalPeerId` is never set, `PlayerControlled` is never added, and `creature_movement_system` / `send_client_input` have no target. |
| Camera does not track the player character | Blocker | Same root cause as above. The camera follows `PlayerControlled` entities. Since `PlayerControlled` is never inserted (depends on `LocalPeerId` from the lost `Welcome` message), the camera has no target and stays at its initial position. |
| Two test compilation errors (`Timer::finished()` not a method) in `net_game.rs:495,502` | Blocks `cargo test` | Must be fixed before squash-merge. Use `timer.0.just_finished()` or remove the assertion. |
| Two dead-code warnings (`ServerCommandSender`, `ClientMessageSender` in `runtime.rs:50,54`) | Warning noise | These structs were superseded by `NetServerSender`/`NetClientSender` in `lib.rs`. Delete them. |
| Irrefutable `if let` warning in `net_game.rs:217` | Warning noise | `PeerMessage` has only one variant (`Input`), making `if let` irrefutable. Use a direct `let` destructure. |
| Host player spawns at hardcoded position `(8.0, 0.86, 5.0)` | Minor | Should probably use the same spawn position logic as single-player or be configurable. |
| No integration test for actual QUIC round-trip | Test gap | Protocol serialization is well-tested, but no test verifies end-to-end message delivery over a real connection. |

### What went well

- **Protocol types matched the plan exactly.** `HostMessage`, `PeerMessage`,
  `PeerState`, `PeerId` all shipped as designed with no rework.
- **Sealed async boundary held.** No tokio/quinn types appear outside
  `modules/network/`. The `NetServerSender`/`NetClientSender` resources
  expose a clean synchronous API.
- **Bottom-up task sequencing worked.** Each PR built on the previous one
  cleanly: protocol → API → server streams → client streams → game systems →
  Join button. No PR had to be reworked due to a missing dependency.
- **Good test coverage.** 8 protocol roundtrip tests, 5 sender/event tests,
  10 runtime lifecycle tests, 7 net_game unit tests — all covering the new
  code. The serialization boundary is especially well-tested.
- **Domain-neutral protocol.** The network module truly doesn't know about
  creatures, players, or game state — it deals in peers, positions, and input
  vectors.

### What to do differently next time

- **Compile tests before each PR merge.** The `Timer::finished()` errors
  would have been caught immediately. Add a CI check or at minimum run
  `cargo test` locally before marking a task as done.
- **Account for self-connection architecture in the plan.** The "host connects
  to itself" pattern is a better design but wasn't planned. If the plan had
  explored this option, the `spawn_host_player` system wouldn't have needed
  rework. Spike architectural options before committing to a player lifecycle
  design.
- **List all transitive dependencies.** `bytes` and `futures-util` were
  obvious needs for the codec wrapping but weren't in the plan's dependency
  section. A quick scan of the API signatures would have caught this.
- **Create `TODO.md` per the plan-guide.** Skipping it meant the
  `validate-pr-target` workflow and issue-labelling automation weren't used.
  Follow the documented process even for plans where the task breakdown seems
  obvious.
- **Clean up dead code immediately.** The `ServerCommandSender` and
  `ClientMessageSender` vestiges in `runtime.rs` should have been removed in
  the same PR that introduced their replacements.
