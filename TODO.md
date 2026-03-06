# TODO

## things: `SpawnThing` uses `EntityEvent` but is triggered globally

`SpawnThing` derives `EntityEvent` but is always fired via `commands.trigger()` /
`world.trigger()` (global) rather than entity-targeted
(`commands.trigger_targets()` / `world.trigger_targets()`). It carries the target
entity ID manually in its `entity` field.

Either switch to `trigger_targets` and use the observer's `event_target()`, or
change the derive to `Event` / `Message` if entity-targeting is not needed.

## Pre-loaded stash logic (can inside toolbox)

`world_setup.rs` spawned a can (kind 2) and a toolbox (kind 3) and then
executed deferred `World` access to stash the can inside the toolbox:

- Extracted the can's `Collider` and `GravityScale` into `StashedPhysics`.
- Set `Visibility::Hidden` and removed `RigidBody` / `LinearVelocity`.
- Set `Container.slots[0]` to the stashed can entity.

This pre-loading cannot be expressed by the spawns layer alone (which only
records kind + position). It will need either a richer spawns format, a
separate `"containers"` layer, or a post-load fixup system.

## Vacuum region specification in map files

`world_setup.rs` hardcoded a vacuum region
`(IVec2::new(11, 1), IVec2::new(14, 8))` when initializing the gas grid.
`WorldInitPlugin` currently initializes all walkable cells with uniform
standard pressure (no vacuum). Vacuum regions will need a dedicated
`"atmospherics"` map layer that stores per-cell gas data or region
definitions.
