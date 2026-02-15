## Return Result from NetClientSender::send instead of bool

**Plan:** `plan/networked-multiplayer` · [docs/plans/networked-multiplayer.md](docs/plans/networked-multiplayer.md)

`NetClientSender::send()` returns `bool`, losing the distinction between
buffer-full and channel-closed. Replace with a `Result` type so callers can
handle each case appropriately.

**File:** `modules/network/src/lib.rs` (the existing `// TODO return an error`
comment at line 169)

## Document ThingRegistry plugin ordering requirement

**Plan:** `plan/networked-multiplayer` · [docs/plans/networked-multiplayer.md](docs/plans/networked-multiplayer.md)

`CreaturesPlugin::build()` calls `resource_mut::<ThingRegistry>()` directly,
which panics if `ThingsPlugin` hasn't been added yet. The current plugin order
in `main.rs` is correct, but this is a silent invariant. Add a doc comment to
both `ThingsPlugin` and `CreaturesPlugin` stating the ordering requirement.

**Files:** `modules/things/src/lib.rs`, `src/creatures/mod.rs`

## Server must spawn entities locally for dedicated server mode

**Plan:** `plan/networked-multiplayer` · [docs/plans/networked-multiplayer.md](docs/plans/networked-multiplayer.md)

The server broadcasts `EntitySpawned` but does not spawn the entity on the
server side. Currently this works because the host connects to itself as a
client and receives its own broadcast. A future dedicated server (no
self-connection) will need to spawn entities locally so that physics simulation
and `broadcast_state` can find them. Add a server-side `SpawnThing` trigger
alongside the broadcast in `handle_client_message`.

**File:** `src/server.rs`

## Add NetId lookup index for StateUpdate

**Plan:** `plan/networked-multiplayer` · [docs/plans/networked-multiplayer.md](docs/plans/networked-multiplayer.md)

`StateUpdate` handling in `client.rs` uses a nested loop (O(n*m)) to match
`NetId` to entities. For the current two-player scope this is fine, but will
become a bottleneck with many replicated entities. Add a `HashMap<NetId, Entity>`
resource or use Bevy's index queries when entity count grows.

**File:** `src/client.rs`
