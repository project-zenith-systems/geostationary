
## Creature pressure-force slip

Creatures currently hard-set their velocity every frame via
`apply_input_velocity`, which completely overrides pressure forces. When
the pressure gradient force on a creature exceeds a configurable
threshold, the controller should lose authority and let the force carry
the creature (slipping toward the breach). Below the threshold the
controller wins as it does now.

- Add a slip threshold constant to `config.toml`
  (`atmospherics.slip_force_threshold` or similar)
- In `apply_input_velocity` (or a new system after it), compare the
  pressure force magnitude against the threshold; if exceeded, blend or
  skip the velocity override so `ConstantForce` takes effect
- Consider a gradual blend rather than a hard cutover to avoid jarring
  transitions

**Note:** After implementing this, the simulation constants
(`pressure_force_scale`, `diffusion_rate`, `pressure_constant`) will
need to be re-tuned so that the slip feels right — the threshold
interacts with all three values.

## Hot-reload config.toml at runtime

Add a system that detects changes to `config.toml` and re-applies values
to their corresponding resources without restarting the game. Could be
file-mtime polling, filesystem notify, or a debug keypress (e.g. F6).

Any config value backed by a Bevy resource or mutable field is a
candidate: simulation tuning constants, network settings, debug flags,
UI preferences, etc. Init-only values (like `standard_pressure`, which
seeds the gas grid once) would need a separate "reset" action rather
than live update.

## Hot-reload assets at runtime

Enable Bevy's asset hot-reloading so that changes to asset files are
picked up at runtime without restarting the game. Bevy supports this via
`AssetPlugin { watch_for_changes: true, .. }` or the equivalent 0.18
configuration.

Use case: editing tile materials or creature meshes in an external tool
and seeing the result in the running game immediately, without a
restart–reconnect cycle.

## Tile entity spatial lookup

`apply_tile_mutation` does a linear scan of all tile entities to find the
one at a given grid position. Fine for the 16x10 test room but will not
scale. Replace with a `HashMap<IVec2, Entity>` resource or a grid-indexed
component query.

**Files:** `modules/tiles/src/lib.rs` (`apply_tile_mutation`)

## WorldHit raycast priority

Both `raycast_tiles` and `raycast_things` can emit `WorldHit` events for
the same right-click when a tile and a thing overlap. The interactions
module takes the last hit in the frame, so behavior depends on system
execution order. Add explicit priority (things > tiles) or filter
duplicate hits so the context menu is deterministic.
