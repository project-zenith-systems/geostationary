## Create L0 physics module (`modules/physics`)

New workspace crate wrapping Avian 3D. See [slice plan](docs/plans/slice-physics-foundation.md).

- `Cargo.toml` with `bevy = "0.18"` and `avian3d = "0.5"`
- `PhysicsPlugin` that wraps `avian3d::PhysicsPlugins`, configures gravity (negative Y) and fixed timestep
- Re-export only the types other modules will use: `RigidBody`, `Collider`, `LinearVelocity`, `Restitution`, `LockedAxes` (adjust set during implementation — fewer is better)
- Add `physics` to workspace members in root `Cargo.toml`
- Add `physics` dependency to root crate and register `PhysicsPlugin` in `main.rs`
- Tests: plugin builds without panic, re-exported types are accessible

No game logic. No colliders on existing entities. Just the module and the plugin wiring.

## Add static colliders to all tile entities

Depends on the physics module existing. Only touches `modules/tiles`.

- Add `physics` as a dependency of the `tiles` crate
- In `spawn_tile_meshes`, every tile gets a `RigidBody::Static` and a `Collider`:
  - Wall tiles: box collider (1.0 x 1.0 x 1.0) matching the `Cuboid` mesh
  - Floor tiles: thin box collider (1.0 x 0.1 x 1.0) at y=0 matching the `Plane3d` footprint
- Rule: if a tile has geometry, it has a collider. No invisible planes spawned elsewhere.
- Existing tile tests must still pass

## Spawn bouncing ball

Depends on tile colliders being present. Only touches `src/world_setup.rs`.

- Spawn a dynamic sphere above the floor: `RigidBody::Dynamic`, sphere `Collider`, `Restitution` of 0.8 or higher, `Mesh3d` sphere with a bright material, `DespawnOnExit(AppState::InGame)`
- The ball should visibly fall, bounce off floor tiles and wall tiles, and settle

This task validates the full rigid-body pipeline (gravity, collision detection, restitution) without touching any gameplay systems.

## Replace homebrew collision with physics-driven player movement

Depends on tile colliders and the physics module. Touches `src/world_setup.rs` and `src/creatures/mod.rs`.

In `world_setup.rs`:
- Add `RigidBody::Kinematic`, capsule `Collider` (radius 0.3, length 1.0), and `LockedAxes` (all rotation locked) to the player entity spawn

In `creatures/mod.rs`:
- Rewrite `creature_movement_system`: read input, compute desired velocity vector, write `LinearVelocity`. No tilemap checks.
- Remove the `Tilemap` resource dependency from the movement system
- Remove the per-axis sliding logic and `tilemap.is_walkable` calls
- Remove `use tiles::Tilemap` import

After this task, confirm no `avian3d` import exists outside `modules/physics`. All physics types used in `tiles`, `creatures`, and `world_setup` come from `physics::*`.
