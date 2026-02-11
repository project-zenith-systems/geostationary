## Fix floor collider Y-offset so floor surface sits at y=0.0

Floor tile colliders are currently centered at the tile's Y position, putting the
surface at y=0.05 instead of y=0.0. This forced the player spawn height to y=0.86
to clear the surface. Either offset the collider downward or adopt a spawn-height
convention.

Files: `modules/tiles/src/lib.rs` (collider spawn), `src/world_setup.rs` (player spawn Y)

**Context:** [docs/plans/physics-foundation.md](docs/plans/physics-foundation.md) post-mortem, remaining open issues

## Gate `PhysicsDebugPlugin` behind debug builds or a config flag

`PhysicsDebugPlugin` is currently always active, rendering collider wireframes
in all builds. It should only run in debug/dev builds or when enabled via
`AppConfig`.

Files: `src/main.rs` (plugin registration)

**Context:** [docs/plans/physics-foundation.md](docs/plans/physics-foundation.md) post-mortem, remaining open issues

## Document world coordinate conventions

Add a short section to the architecture docs defining the coordinate system:
which axis is up, what y=0 means, where tile surfaces sit, and how spawn heights
are calculated. This prevents the floor-offset class of bug from recurring across
plans.

Files: `docs/architecture.md`

**Context:** [docs/plans/physics-foundation.md](docs/plans/physics-foundation.md) post-mortem, what to do differently
