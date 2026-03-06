# TODO

## Persist `SpawnPoint.contents` through load-save round-trip

`SpawnsLayer::save` currently writes an empty `contents` vec for every spawn
point. Once container pre-loading is implemented, the original contents from the
map file should survive save and reload.

Source: `modules/things/src/lib.rs:263` — inline TODO comment in `SpawnsLayer::save`.

## Pre-loaded stash logic (can inside toolbox)

The old `world_setup.rs` spawned a can and a toolbox and then stashed the can
inside the toolbox via deferred `World` access:

- Extracted the can's `Collider` and `GravityScale` into `StashedPhysics`.
- Set `Visibility::Hidden` and removed `RigidBody` / `LinearVelocity`.
- Set `Container.slots[0]` to the stashed can entity.

This pre-loading cannot be expressed by the spawns layer alone (which only
records kind + position). It will need either a richer spawns format (the
`SpawnPoint.contents` field), a separate `"containers"` layer, or a post-load
fixup system.

Source: deleted `bins/shared/src/world_setup.rs`; `SpawnPoint.contents` field in
`modules/things/src/lib.rs:107`.

## Network: refactor module streams from `open_uni()` to bidirectional `open_bi()`

Module streams currently use `open_uni()` with a `StreamDirection` enum. Should be
refactored to bidirectional (`open_bi()`) per module tag — this removes
`StreamDirection` entirely and supports both server-to-client snapshots and
client-to-server mutations on a single stream.

Source: `modules/network/src/server.rs:167`, `modules/network/src/client.rs:141` —
`open_uni()` call sites; `StreamDirection` enum used across `network`, `things`,
and `interactions` modules.

## Network: simplify client/server connection code

Network client/server code (`client.rs`, `server.rs`) has deeply nested
`tokio::select!` blocks, many cloned cancellation tokens, and verbose error
handling. Needs a simplification pass to improve readability and maintainability.

Source: `modules/network/src/client.rs`, `modules/network/src/server.rs`.
