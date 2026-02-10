# Plan: Physics Foundation

> **Stage goal:** A bouncing ball demonstrates real rigid-body physics in the
> room, and the player character collides with walls through the physics engine
> instead of manual tilemap checks. The L0 `physics` module exists as a
> workspace crate that wraps Avian 3D, giving upper layers a game-flavoured
> API without exposing the underlying engine.

## What "done" looks like

1. A new `modules/physics` workspace crate provides `PhysicsPlugin`
2. Walls have colliders — the player bumps into them via the physics engine
3. The homebrew tilemap collision check in `creature_movement_system` is removed
4. A ball spawns in the room, falls under gravity, bounces off the floor and
   walls, and eventually comes to rest
5. The player character uses a kinematic rigid body moved by input, colliding
   with static wall geometry
6. No `avian3d` import appears outside `modules/physics` — game code only uses
   types re-exported by the physics module
7. Existing tests still pass; new tests cover the physics module's public API

## Strategy

The first plan proved the layer stack with manual collision. This plan
replaces that stopgap with a real physics engine, establishing the L0 physics
module that the rest of the game will build on. The bouncing ball is a
deliberately simple dynamic body that validates the full rigid-body pipeline
(gravity, collision response, restitution) without entangling gameplay systems.

The key architectural decision is **isolation**: `avian3d` is a dependency of
`modules/physics` only. The module re-exports the subset of types that game
code needs (rigid body markers, collider constructors, maybe a velocity type)
under our own namespace. If we ever swap physics engines, only this module
changes.

Creature movement shifts from "check tilemap then translate" to "apply velocity
to a kinematic body and let the physics engine resolve collisions." The
`Tilemap::is_walkable` check and the per-axis sliding logic in
`creature_movement_system` are removed. Walls get static-body colliders during
tile mesh spawning, and the player's capsule gets a kinematic collider at
spawn time.

### Layer participation

| Layer | Module | Plan scope |
|-------|--------|------------|
| L0 | `physics` | **New.** Workspace crate wrapping Avian 3D. Provides `PhysicsPlugin`, re-exports collider/body types, configures physics timestep and gravity. |
| L0 | `ui` | Unchanged |
| L0 | `network` | Unchanged |
| L1 | `tiles` | Add static colliders to all tile entities during mesh spawning (walls: box, floors: thin box) |
| L1 | `things` | Unchanged |
| L3 | `creatures` | Replace manual collision with kinematic body + physics velocity. Remove `Tilemap` dependency from movement system. |
| — | `world_setup` | Spawn a bouncing ball entity with dynamic rigid body |

### Not in this plan

- **Character controller crate** (bevy-tnua, bevy_ahoy). The kinematic body
  approach with direct velocity control is sufficient for WASD movement. A
  proper character controller is future work when slopes, steps, and ground
  detection matter.
- **Physics-based gravity module** (L2 `gravity`). The binary grounded/
  weightless toggle described in the architecture is a gameplay concept. This
  plan just uses Avian's built-in gravity vector.
- **Networked physics / determinism.** The `enhanced-determinism` feature flag
  exists but is not needed until the simulation is server-authoritative.
- **Collision layers / groups.** Everything collides with everything for now.
  Layer-based filtering is future work when projectiles, creatures, and items
  need different collision rules.
- **Spatial queries.** Raycasts and shape casts are part of Avian but not
  exercised in this plan.
- **Floor-as-gameplay-surface.** Floor tiles get colliders so the ball has
  something to land on, but using floor colliders for creature grounding
  (gravity, slopes) is a character-controller concern, deferred.

### Module placement

```
modules/
  physics/            # L0 workspace crate — NEW
    Cargo.toml        # depends on bevy 0.18, avian3d 0.5
    src/
      lib.rs          # PhysicsPlugin, re-exports, gravity/timestep config
src/
  creatures/mod.rs    # MODIFIED — kinematic body movement replaces tilemap checks
  world_setup.rs      # MODIFIED — spawns bouncing ball entity
modules/
  tiles/src/lib.rs    # MODIFIED — wall entities get static colliders
```

### Physics module design

The `modules/physics` crate owns the `avian3d` dependency. It exposes:

- `PhysicsPlugin` — wraps `avian3d::PhysicsPlugins`, configures gravity
  (negative Y, standard magnitude) and the fixed timestep.
- Re-exported types that game code needs: `RigidBody`, `Collider`,
  `LinearVelocity`, `Restitution`, and `ColliderDensity` (or whichever subset
  we actually use). These are re-exported as `physics::RigidBody` etc.
- No game logic lives here. This is pure plumbing.

The module does **not** re-export the entire `avian3d` prelude. Only types that
appear in another module's component bundles get re-exported. This keeps the
API surface small and makes engine swaps feasible.

### Tile collider design

Every tile entity gets a static collider during `spawn_tile_meshes`. The rule
is simple: if a tile has geometry, it has a collider. No invisible physics
planes, no special-case floor entities elsewhere — the tiles module owns all
static world geometry and all static world colliders.

- **Wall tiles:** `RigidBody::Static` + box `Collider` (1.0 x 1.0 x 1.0),
  matching the existing `Cuboid` mesh.
- **Floor tiles:** `RigidBody::Static` + thin box `Collider` (1.0 x 0.1 x 1.0)
  at y=0, matching the `Plane3d` mesh footprint. The thin box gives the ball
  a surface to bounce off and gives future dynamic bodies something to land on.

This approach means colliders and meshes are always in sync — adding a new
`TileKind` variant naturally requires deciding its collider shape in the same
match arm.

### Creature movement design

The player capsule spawns with:

- `RigidBody::Kinematic` — physics engine handles collision resolution, but
  movement is driven by game code, not forces.
- `Collider` — capsule matching the existing mesh dimensions (radius 0.3,
  length 1.0).
- Locked rotation — the capsule should not tip over.

The movement system changes from:

```
read input → compute delta → check tilemap → translate transform
```

to:

```
read input → compute desired velocity → write LinearVelocity
```

The physics engine steps the kinematic body, resolves collisions against static
wall colliders, and updates the transform. The per-axis tilemap sliding logic
and the `Tilemap` resource dependency are removed from `creature_movement_system`.

### Bouncing ball design

A sphere entity spawned in `world_setup` at a position above the floor:

- `RigidBody::Dynamic` — fully simulated by the physics engine
- `Collider` — sphere matching mesh radius
- `Restitution` — set high (0.8+) so it visibly bounces
- `Mesh3d` + `MeshMaterial3d` — bright colour to distinguish from walls/player

The ball proves that gravity, collision detection, and collision response all
work. It requires no game systems — Avian handles everything. It will
eventually come to rest on the floor due to damping.

---

## Post-mortem

*To be filled in after the plan ships.*
