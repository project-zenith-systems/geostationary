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
| L0 | `network` | `PeerId` type, `HostMessage`/`PeerMessage`/`PeerState` protocol types, `NetServerSender`/`NetClientSender` resources for typed messages, `NetEvent` variants for protocol delivery, length-prefixed wire framing, serde+bincode serialization, bi-directional QUIC streams per client |
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
      framing.rs    # NEW — read_frame / write_frame (u32 length prefix)
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

Wire framing (`framing.rs`): `write_frame` writes `[u32 len][payload]`;
`read_frame` reads one frame. Internal to the module.

The server maintains a `HashMap<PeerId, SendStream>` internally. When
`send_to(peer, msg)` is called, the message is serialized and routed to the
correct stream. `broadcast` iterates all peers.

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

*To be filled in after the plan ships.*
