# Plan: Networked Multiplayer

> **Stage goal:** Two game instances communicate over the wire. The host runs a
> server-authoritative simulation — clients send WASD input, the server runs
> physics for all players and sends authoritative positions back. The L0 network
> module gains domain-agnostic byte channels while all game protocol knowledge
> stays in the root crate.

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
8. No game types exist inside `modules/network/` — it moves raw `Vec<u8>` only
9. No `tokio`/`quinn`/`mpsc` types appear outside `modules/network/`
10. Existing tests pass; new tests cover data channels and protocol serialization

## Strategy

This plan sends game data over the wire for the first time. The core tension is
between the network module (L0, domain-agnostic) and the game protocol (root
crate, domain-aware). The network module gains the ability to shuttle bytes
between peers — nothing more. All serialization, entity spawning, and state
replication live in the root crate.

Work proceeds bottom-up: extend the network module with data channels, define the
game protocol, rewire player spawning through the network flow, add the Join
button, then wire the full input→state loop.

Lessons from previous post-mortems: sequence PRs explicitly (physics-foundation),
decide entity ownership up front (playable-character), follow the branching
convention, and write tests not just visual checks.

### Layer participation

| Layer | Module | Plan scope |
|-------|--------|------------|
| L0 | `network` | `PeerId` type, `NetServerSender`/`NetClientSender` resources for raw byte channels, `NetEvent::DataReceived`/`ClientDisconnected` variants, length-prefixed wire framing, bi-directional QUIC streams per client |
| L0 | `physics` | Unchanged |
| L0 | `ui` | Unchanged |
| L1 | `tiles` | Unchanged |
| L1 | `things` | Unchanged |
| L1 | `main_menu` | `MenuEvent::Join` variant, Join button on title screen |
| L3 | `creatures` | `creature_movement_system` gated to run only on `ListenServer` |
| L6 | `camera` | Unchanged (follows `PlayerControlled`) |
| — | `protocol.rs` | **New.** `ServerMessage`, `ClientMessage`, `PlayerState` — serde + bincode |
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
    src/
      lib.rs          # MODIFIED — new variants, PeerId, sender resources
      server.rs       # MODIFIED — bi-di streams, read/write loops, PeerId map
      client.rs       # MODIFIED — bi-di stream, read/write loops
      runtime.rs      # MODIFIED — sender channels in NetworkTasks
      framing.rs      # NEW — read_frame / write_frame (u32 length prefix)
      config.rs       # Unchanged
src/
  protocol.rs         # NEW — ServerMessage, ClientMessage, bincode serde
  net_game.rs         # NEW — systems: input send, state broadcast, player join/leave
  world_setup.rs      # MODIFIED — remove player spawn
  creatures/mod.rs    # MODIFIED — gate movement on NetworkRole
  main_menu/
    mod.rs            # MODIFIED — handle MenuEvent::Join
    title_screen.rs   # MODIFIED — add Join button
  main.rs             # MODIFIED — NetworkRole, net_game registration, Join flow
```

### Network data channel design

Two new resources for sending raw bytes:

- **`NetServerSender`** — wraps `mpsc::UnboundedSender`. Methods: `send_to(peer,
  data)` and `broadcast(data)`. Inserted when the server task starts, removed on
  stop. Game code uses `Option<Res<NetServerSender>>`.
- **`NetClientSender`** — wraps `mpsc::UnboundedSender<Vec<u8>>`. Method:
  `send(data)`. Inserted when the client task starts, removed on disconnect.

Incoming data: `NetEvent::DataReceived { from: PeerId, data: Vec<u8> }`. On the
client, `from` is always `PeerId(0)` (the server). On the server, `from` is the
client's assigned `PeerId`.

Wire framing (`framing.rs`): `write_frame` writes `[u32 len][payload]`;
`read_frame` reads one frame. This is the only serialization inside the network
module — it knows nothing about what the bytes contain.

The server maintains a `HashMap<PeerId, SendStream>` internally. When
`NetServerSender.send_to(peer, data)` is called, the data is routed to the
correct stream. `broadcast` iterates all peers.

### Game protocol design

`src/protocol.rs` (root crate only, never enters network module):

- `ClientMessage::InputUpdate { direction: [f32; 3] }` — normalized WASD vector
- `ServerMessage::Welcome { your_id: u64 }` — sent to newly connected client
- `ServerMessage::PlayerJoined { id: u64, position: [f32; 3] }`
- `ServerMessage::PlayerLeft { id: u64 }`
- `ServerMessage::StateUpdate { players: Vec<PlayerState> }` — authoritative
  positions each tick

Serialization: `bincode` (compact binary, serde-compatible). Added as root crate
dependency only.

### Player lifecycle

Player entities are spawned by the network flow, **not** by `setup_world`:

**Host (ListenServer):**
- `OnEnter(InGame)` + `ListenServer` → spawn host player (Dynamic body,
  `PlayerControlled`, `Creature`, `MovementSpeed`, `PeerId(0)` component)
- `ClientConnected { id }` → spawn remote player (Dynamic body, `Creature`,
  `MovementSpeed`, assigned `PeerId` component, **no** `PlayerControlled`). Send
  `Welcome` + `PlayerJoined` for all existing players to new client. Send
  `PlayerJoined` for new player to all clients.
- `ClientDisconnected { id }` → despawn entity, broadcast `PlayerLeft`

**Client:**
- `PlayerJoined { id, pos }` → spawn entity (Kinematic body, no physics). If
  `id == my_id`, add `PlayerControlled` so the camera follows it.
- `PlayerLeft { id }` → despawn entity
- `StateUpdate` → set `Transform.translation` on matching entities

### Server-authoritative input/state loop

**Host each frame:**
1. `creature_movement_system` reads keyboard → sets host player `LinearVelocity`
2. `apply_remote_input` reads `DataReceived` → deserializes `InputUpdate` → sets
   remote player `LinearVelocity` via `MovementSpeed`
3. Avian physics steps all Dynamic bodies
4. `broadcast_state` collects all player positions → serializes `StateUpdate` →
   sends via `NetServerSender.broadcast()`

**Client each frame:**
1. `send_client_input` reads keyboard → serializes `InputUpdate` → sends via
   `NetClientSender`
2. `receive_server_state` reads `DataReceived` → deserializes `ServerMessage` →
   applies positions to entities

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

- Add `bincode = "1"` and `serde = { version = "1", features = ["derive"] }` to
  root `Cargo.toml` (serde is already there)
- No new dependencies in `modules/network/Cargo.toml`

## Post-mortem

*To be filled in after the plan ships.*
