## Add protocol types to network module

**Plan:** `plan/networked-multiplayer` · [docs/plans/networked-multiplayer.md](docs/plans/networked-multiplayer.md)

**Files:** `modules/network/src/protocol.rs` (new), `modules/network/Cargo.toml`

- Add `serde` and `bincode` dependencies to `modules/network/Cargo.toml`
- Add `protocol.rs` with domain-neutral types: `PeerId`, `HostMessage`, `PeerMessage`, `PeerState`
- `HostMessage` variants: `Welcome`, `PeerJoined`, `PeerLeft`, `StateUpdate`
- `PeerMessage` variants: `Input { direction }`
- Internal `encode`/`decode` functions wrapping bincode
- Stream framing uses `tokio_util::codec::LengthDelimitedCodec` (already in deps) — no custom framing code
- Unit tests for serialization roundtrips
- **Not included:** integration with server/client tasks or NetEvent changes (next tasks)

## Extend network API with typed message events and sender resources

**Plan:** `plan/networked-multiplayer` · [docs/plans/networked-multiplayer.md](docs/plans/networked-multiplayer.md)

**Files:** `modules/network/src/lib.rs`, `modules/network/src/runtime.rs`

Depends on: "Add protocol types to network module"

- Add `NetEvent::HostMessageReceived(HostMessage)` variant (client receives from host)
- Add `NetEvent::PeerMessageReceived { from: PeerId, message: PeerMessage }` variant (host receives from peer)
- Rename `NetEvent::ClientConnected` → `PeerConnected { id: PeerId, addr }`
- Add `NetEvent::PeerDisconnected { id: PeerId }` variant
- Add `NetServerSender` resource with `send_to(peer, &HostMessage)` and `broadcast(&HostMessage)` — internally serializes and routes via mpsc
- Add `NetClientSender` resource with `send(&PeerMessage)` — internally serializes and sends via mpsc
- Re-export public protocol types (`PeerId`, `HostMessage`, `PeerMessage`, `PeerState`) from `lib.rs`
- Update `runtime.rs` with internal channel types for server/client data sending
- **Not included:** server.rs / client.rs changes (next tasks)

## Rewrite server task for bi-directional data streams

**Plan:** `plan/networked-multiplayer` · [docs/plans/networked-multiplayer.md](docs/plans/networked-multiplayer.md)

**Files:** `modules/network/src/server.rs`, `modules/network/src/lib.rs`

Depends on: "Extend network API with typed message events and sender resources"

- Maintain `HashMap<PeerId, SendStream>` for connected peers
- Assign incrementing PeerIds (starting at 1) on connection
- Open bi-directional QUIC stream per peer
- Wrap streams with `LengthDelimitedCodec` via `Framed` for automatic message delimiting
- Per-peer read loop: decode `PeerMessage` from framed stream → emit `NetEvent::PeerMessageReceived`
- Per-peer write loop: read from routing channel → encode `HostMessage` → write to framed stream
- Emit `NetEvent::PeerDisconnected { id }` on disconnect
- Create `NetServerSender` mpsc channel before spawning server task; insert resource
- Route `send_to` and `broadcast` through internal channels to per-peer write loops

## Rewrite client task for bi-directional data stream

**Plan:** `plan/networked-multiplayer` · [docs/plans/networked-multiplayer.md](docs/plans/networked-multiplayer.md)

**Files:** `modules/network/src/client.rs`, `modules/network/src/lib.rs`

Depends on: "Extend network API with typed message events and sender resources"

- After connecting, open bi-directional QUIC stream wrapped with `LengthDelimitedCodec`
- Read loop: decode `HostMessage` from framed stream → emit `NetEvent::HostMessageReceived`
- Write loop: read from channel → encode `PeerMessage` → write to framed stream
- Create `NetClientSender` mpsc channel before spawning client task; insert resource
- Handle disconnect/cancellation cleanly

## Create network game systems and refactor player spawning

**Plan:** `plan/networked-multiplayer` · [docs/plans/networked-multiplayer.md](docs/plans/networked-multiplayer.md)

**Files:** `src/net_game.rs` (new), `src/world_setup.rs`, `src/creatures/mod.rs`

Depends on: network module tasks above

- Add `NetworkRole` resource (`None`, `ListenServer`, `Client`)
- Add `LocalPeerId` resource to store the client's own PeerId
- Create `net_game.rs` with systems:
  - `spawn_host_player`: on `OnEnter(InGame)` + `ListenServer`, spawn host player
  - `handle_peer_connected`: spawn remote player, send Welcome/PeerJoined via NetServerSender
  - `handle_peer_disconnected`: despawn entity, broadcast PeerLeft
  - `apply_remote_input`: read PeerMessageReceived, set remote player velocity
  - `broadcast_state`: collect all player positions, send StateUpdate via NetServerSender
  - `send_client_input`: read keyboard, send PeerMessage::Input via NetClientSender
  - `receive_host_messages`: read HostMessageReceived, spawn/despawn/update entities
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
  - On `PeerDisconnected` / `Disconnected`: handle cleanup
- Register `net_game` systems with appropriate run conditions
- Init `NetworkRole` resource
- Add `net_game` module declaration
