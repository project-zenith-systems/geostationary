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

