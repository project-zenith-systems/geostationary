# TODO

## things: `SpawnThing` uses `EntityEvent` but is triggered globally

`SpawnThing` derives `EntityEvent` but is always fired via `commands.trigger()` /
`world.trigger()` (global) rather than entity-targeted
(`commands.trigger_targets()` / `world.trigger_targets()`). It carries the target
entity ID manually in its `entity` field.

Either switch to `trigger_targets` and use the observer's `event_target()`, or
change the derive to `Event` / `Message` if entity-targeting is not needed.
