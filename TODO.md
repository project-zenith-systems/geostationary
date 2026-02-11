## Add wire framing to network module

**Plan:** `plan/networked-multiplayer` · [docs/plans/networked-multiplayer.md](docs/plans/networked-multiplayer.md)

**Files:** `modules/network/src/framing.rs` (new)

- Add `framing.rs` with `write_frame` and `read_frame` functions
- `write_frame`: writes `[u32 length][payload]` to an `AsyncWriteExt`
- `read_frame`: reads one length-prefixed frame from an `AsyncReadExt`
- Unit tests for roundtrip encode/decode and edge cases (empty payload, max size)
- **Not included:** integration with server/client (next task)

## Extend network API with PeerId, data events, and sender resources

**Plan:** `plan/networked-multiplayer` · [docs/plans/networked-multiplayer.md](docs/plans/networked-multiplayer.md)

**Files:** `modules/network/src/lib.rs`, `modules/network/src/runtime.rs`

Depends on: "Add wire framing to network module"

- Add `PeerId` newtype (`u64`, `Copy`, `Clone`, `Debug`, `Eq`, `Hash`)
- Add `NetEvent::DataReceived { from: PeerId, data: Vec<u8> }` variant
- Add `NetEvent::ClientDisconnected { id: PeerId }` variant
- Change `NetEvent::ClientConnected` to include `id: PeerId`
- Add `NetServerSender` resource with `send_to(peer, data)` and `broadcast(data)` methods
- Add `NetClientSender` resource with `send(data)` method
- Update `runtime.rs` with internal channel types for server/client data sending
- **Not included:** server.rs / client.rs changes (next tasks)

## Rewrite server task for bi-directional data streams

**Plan:** `plan/networked-multiplayer` · [docs/plans/networked-multiplayer.md](docs/plans/networked-multiplayer.md)

**Files:** `modules/network/src/server.rs`, `modules/network/src/lib.rs`

Depends on: "Extend network API with PeerId, data events, and sender resources"

- Maintain `HashMap<PeerId, SendStream>` for connected clients
- Assign incrementing PeerIds (starting at 1) on connection
- Open bi-directional QUIC stream per client
- Per-client read loop: `read_frame` → emit `NetEvent::DataReceived { from, data }`
- Per-client write loop: read from routing channel → `write_frame` to stream
- Emit `NetEvent::ClientDisconnected { id }` on disconnect
- Create `NetServerSender` resource before spawning server task
- Insert/remove `NetServerSender` from Bevy world via event channel
- Route `send_to(peer, data)` and `broadcast(data)` through internal channels

## Rewrite client task for bi-directional data stream

**Plan:** `plan/networked-multiplayer` · [docs/plans/networked-multiplayer.md](docs/plans/networked-multiplayer.md)

**Files:** `modules/network/src/client.rs`, `modules/network/src/lib.rs`

Depends on: "Extend network API with PeerId, data events, and sender resources"

- After connecting, open bi-directional QUIC stream
- Read loop: `read_frame` → emit `NetEvent::DataReceived { from: PeerId(0), data }`
- Write loop: read from channel → `write_frame` to stream
- Create `NetClientSender` resource before spawning client task
- Insert/remove `NetClientSender` from Bevy world via event channel
- Handle disconnect/cancellation cleanly

## Create game protocol module

**Plan:** `plan/networked-multiplayer` · [docs/plans/networked-multiplayer.md](docs/plans/networked-multiplayer.md)

**Files:** `src/protocol.rs` (new), `Cargo.toml`

Depends on: network module tasks above

- Add `bincode = "1"` to root `Cargo.toml`
- Define `ServerMessage` enum: `Welcome`, `PlayerJoined`, `PlayerLeft`, `StateUpdate`
- Define `ClientMessage` enum: `InputUpdate`
- Define `PlayerState` struct: `id`, `position`, `velocity`
- Add `encode` / `decode` helper functions wrapping bincode
- Unit tests for serialization roundtrips

## Create network game systems and refactor player spawning

**Plan:** `plan/networked-multiplayer` · [docs/plans/networked-multiplayer.md](docs/plans/networked-multiplayer.md)

**Files:** `src/net_game.rs` (new), `src/world_setup.rs`, `src/creatures/mod.rs`

Depends on: "Create game protocol module"

- Add `NetworkRole` resource (`None`, `ListenServer`, `Client`)
- Add `PeerId` component for player entities
- Add `LocalPlayerId` resource to store the client's own PeerId
- Create `net_game.rs` with systems:
  - `spawn_host_player`: on `OnEnter(InGame)` + `ListenServer`, spawn host player
  - `handle_client_connected`: spawn remote player, send Welcome/PlayerJoined
  - `handle_client_disconnected`: despawn entity, broadcast PlayerLeft
  - `apply_remote_input`: deserialize InputUpdate, set remote player velocity
  - `broadcast_state`: serialize all player positions, send via NetServerSender
  - `send_client_input`: serialize keyboard input, send via NetClientSender
  - `receive_server_state`: deserialize ServerMessage, spawn/despawn/update entities
- Remove player capsule spawn from `world_setup.rs`
- Gate `creature_movement_system` on `NetworkRole::ListenServer`

## Add Join button and wire main.rs

**Plan:** `plan/networked-multiplayer` · [docs/plans/networked-multiplayer.md](docs/plans/networked-multiplayer.md)

**Files:** `src/main_menu/mod.rs`, `src/main_menu/title_screen.rs`, `src/main.rs`

Depends on: "Create network game systems and refactor player spawning"

- Add `MenuEvent::Join` variant
- Add "Join" button to title screen between Play and Settings
- Handle `MenuEvent::Join`: send `NetCommand::Connect { addr: localhost:port }`
- Extend `handle_net_events`:
  - On `HostingStarted`: set `NetworkRole::ListenServer`
  - On `Connected` when role is `None`: set `NetworkRole::Client`
  - On `ClientDisconnected` / `Disconnected`: handle cleanup
- Register `net_game` systems with appropriate run conditions
- Init `NetworkRole` resource
- Add `net_game` module declaration
