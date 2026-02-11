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

### Outcome

The plan delivered everything it promised. `modules/physics` wraps Avian 3D
behind a sealed API, walls and floors have static colliders, the homebrew
tilemap collision is gone, and a bouncing ball validates the full rigid-body
pipeline. The player walks around the room with physics-resolved wall
collisions, and no `avian3d` import appears outside the physics module. The
architecture boundary held cleanly. Four PRs landed the work (#58 physics
module, #59 tile colliders, #60 bouncing ball, #61 player movement rewrite),
followed by five fix commits that corrected body-type and collider-size
mistakes discovered during manual testing.

### What shipped beyond the plan

| Addition | Why |
|----------|-----|
| `GravityScale` re-export | Needed for the zero-gravity player workaround (see hurdle 1). Not in the original re-export list but required by game code. |
| `PhysicsDebugPlugin` re-export | Essential for diagnosing every physics hurdle below. Should be gated behind a debug flag eventually (TODO in code). |
| Planning document refactoring (commit `a2f74b7`) | Updated plan-guide and workflows to align with conventions discovered during the first two plans. Cheap housekeeping, done between task PRs. |

### Deviations from plan

- **Dynamic body instead of Kinematic for the player.** The plan specified
  `RigidBody::Kinematic`. In practice, Avian's kinematic bodies do not receive
  collision response — the player walked straight through walls. The fix was
  `RigidBody::Dynamic` with `GravityScale(0.0)` and
  `LockedAxes::ROTATION_LOCKED.lock_translation_y()` to simulate kinematic-like
  behaviour while still getting collision pushback. This is the biggest
  deviation and the one most worth recording for future plans.
- **No physics module tests.** The plan called for "new tests cover the
  physics module's public API." The test module exists but is empty. All
  validation was manual (visual). This is a gap.
- **`ColliderDensity` not re-exported.** Mentioned in the plan as a candidate;
  turned out to be unnecessary. `GravityScale` took its place in the actual
  re-export set.
- **No plan branch.** Work happened on `feat/physics-player-movement` instead
  of the `plan/physics-foundation` branch described by the branching convention.
  Task PRs targeted this feature branch directly.
- **Floor collider Y-offset.** Floor colliders sit at y=0.1 (half the 0.1
  thickness), not y=0.0 as the design implied. This forced the player spawn to
  y=0.86 to clear the surface. A TODO comment marks this for cleanup.

### Hurdles

**1. Kinematic bodies ignore collision response (PR #61, fix commits)**
The plan assumed `RigidBody::Kinematic` would collide with static geometry.
Avian's kinematic bodies are moved by velocity but do not receive pushback from
collisions — the player phased through walls. Switched to
`RigidBody::Dynamic` with gravity disabled and Y-translation locked.
**Lesson:** verify physics engine body-type semantics before committing to one
in the plan. Avian's Kinematic ≠ Unity's Kinematic Controller.

**2. `Collider::cuboid` takes full dimensions, not half-extents (commit `0d61217`)**
The initial tile colliders used `Collider::cuboid(0.5, 0.5, 0.5)` for walls,
assuming half-extents (as Rapier does). Avian's API takes full dimensions, so
every collider was double the intended size. Walls bled into adjacent tiles and
the floor was 0.2 units thick instead of 0.1.
**Lesson:** read the doc signature, not the Rapier muscle memory. This class of
bug is silent — geometry looks "close enough" until something clips.

**3. Cascading symptoms from wrong collider sizes (commits `00d97b2`, `f5abafb`, `49b04a3`)**
Before the root cause (hurdle 2) was found, the undersized colliders caused two
visible symptoms that looked like separate bugs: the player bounced erratically
on floor tiles because it was catching the edges of colliders that didn't fully
tile the surface, and the bouncing ball fell straight through the gaps between
floor colliders. Both prompted intermediate "fixes" — `SweptCcd` on the ball
(thinking it was tunnelling), zero gravity on the player, removing the Y
translation lock — none of which addressed the actual problem. Once the
collider sizes were corrected in `0d61217`, the gaps closed, the bouncing
stopped, and SweptCcd became unnecessary and was removed.
**Lesson:** when multiple physics objects misbehave in the same scene, suspect
shared geometry before per-entity tuning. Debug-render the colliders first.

### What went well

- **Avian isolation held perfectly.** Not a single `avian3d` import outside
  `modules/physics`. The re-export boundary works as designed.
- **Clean removal of homebrew collision.** The `Tilemap` dependency, per-axis
  sliding logic, and `is_walkable` calls in `creature_movement_system` were
  deleted in one commit with no fallout. The module boundary made the cut
  surgical.
- **Bouncing ball proved the pipeline.** Gravity, collision detection,
  restitution, and damping all work. The ball falls, bounces, and settles —
  exactly the validation the plan called for, requiring zero game systems.
- **Small, focused PRs.** Four feature PRs plus targeted fix commits. Each was
  reviewable in isolation.

### What to do differently next time

- **Verify physics engine semantics before the plan.** The Kinematic → Dynamic
  pivot cost five fix commits and would have been avoided by a 10-minute
  prototype. Future plans involving a new engine dependency should include a
  spike task.
- **Write physics tests, not just visual checks.** Every hurdle above was
  caught by eyeballing the game window. An integration test spawning a body
  against a wall and asserting position-after-step would have caught the
  kinematic and collider-size bugs faster and prevented regressions.
- **Follow the branching convention.** This plan skipped the `plan/` branch
  and worked on a feature branch. The convention exists for a reason — squash-
  merging a plan branch into main keeps history clean. Use it next time.
- **Pin down coordinate conventions early.** The floor Y-offset issue cascaded
  into player spawn height calculations. A one-paragraph "world coordinate
  conventions" section in the architecture docs would prevent this class of
  problem across plans.
