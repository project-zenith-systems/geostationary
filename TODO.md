# TODO

## things: `SpawnThing` uses `EntityEvent` but is triggered globally

`SpawnThing` derives `EntityEvent` but is always fired via `commands.trigger()` /
`world.trigger()` (global) rather than entity-targeted
(`commands.trigger_targets()` / `world.trigger_targets()`). It carries the target
entity ID manually in its `entity` field.

Either switch to `trigger_targets` and use the observer's `event_target()`, or
change the derive to `Event` / `Message` if entity-targeting is not needed.

## Atmosphere initialisation after map load

`world_setup.rs` called `atmospherics::initialize_gas_grid()` immediately after
creating the `Tilemap`, passing `standard_pressure`, an optional vacuum region
`(IVec2::new(11, 1), IVec2::new(14, 8))`, and `diffusion_rate` from config.
It also inserted the `PressureForceScale` resource.

With `WorldPlugin` loading the tilemap from a `.station.ron` file, this
initialisation needs a new home — either a system in the atmospherics module
that runs after `WorldReady`, or a dedicated `"atmospherics"` map layer that
stores per-cell gas data and vacuum regions.

## Pre-loaded stash logic (can inside toolbox)

`world_setup.rs` spawned a can (kind 2) and a toolbox (kind 3) and then
executed deferred `World` access to stash the can inside the toolbox:

- Extracted the can's `Collider` and `GravityScale` into `StashedPhysics`.
- Set `Visibility::Hidden` and removed `RigidBody` / `LinearVelocity`.
- Set `Container.slots[0]` to the stashed can entity.

This pre-loading cannot be expressed by the spawns layer alone (which only
records kind + position). It will need either a richer spawns format, a
separate `"containers"` layer, or a post-load fixup system.

## World resource cleanup on state exit

`world_setup.rs` registered `cleanup_world` on `OnExit(AppState::InGame)` to
remove `Tilemap`, `GasGrid`, and `PressureForceScale` resources. This cleanup
should move to the owning modules (tiles, atmospherics) or to a
`WorldTeardown`-driven system in `WorldPlugin`.
