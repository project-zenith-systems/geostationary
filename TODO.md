# TODO — Playable Character Slice

## Tiles module — grid storage and tile types

> See [docs/plans/slice-playable-character.md](docs/plans/slice-playable-character.md) for
> the overall goal, strategy, and design rationale behind these tasks.

Create `modules/tiles` as a workspace crate. Define `TileKind` enum (Floor,
Wall) and a `Tilemap` resource that stores a 2D grid of tile kinds. Provide
methods for querying tile kind at a position and whether a position is
walkable. This is L1 — no rendering, no game logic, just the spatial data
structure.

## Tiles module — mesh spawning

> See [docs/plans/slice-playable-character.md](docs/plans/slice-playable-character.md) for
> the overall goal, strategy, and design rationale behind these tasks.

Add a system to `TilesPlugin` that reads the `Tilemap` resource and spawns 3D
entities for each tile. Floors are flat planes at y=0, walls are unit cubes at
y=0.5. Use simple coloured `StandardMaterial` placeholders (dark grey floors,
lighter grey walls). Tiles should be tagged with a `Tile` component carrying
their grid position.

## Things module — base world object primitives

> See [docs/plans/slice-playable-character.md](docs/plans/slice-playable-character.md) for
> the overall goal, strategy, and design rationale behind these tasks.

Create `modules/things` as a workspace crate. Define a `Thing` marker
component and a `WorldPosition` component that stores grid-aware position
(tile coordinate + sub-tile offset). This is deliberately minimal — it
establishes the L1 convention that all non-grid-bound world objects are Things.
Higher layers (creatures, items) build on top of this.

## Test map — hardcoded room layout

> See [docs/plans/slice-playable-character.md](docs/plans/slice-playable-character.md) for
> the overall goal, strategy, and design rationale behind these tasks.

Create a small hardcoded map for testing: a rectangular room (roughly 12x10)
with floor tiles in the interior and wall tiles around the perimeter. Add a
couple of internal walls to test collision from multiple angles. This can live
in `src/world_setup.rs` as a function that populates the `Tilemap` resource.

## Creature module — components and movement

> See [docs/plans/slice-playable-character.md](docs/plans/slice-playable-character.md) for
> the overall goal, strategy, and design rationale behind these tasks.

Create `src/creatures/` module. Define a `Creature` marker component and a
`MovementSpeed` component. Add a movement system that reads keyboard input
(WASD), calculates a movement vector, checks the target position against the
`Tilemap` for walkability, and updates the creature's `Transform`. Block each
axis independently so the character slides along walls rather than stopping
dead.

## Creature module — player spawning

> See [docs/plans/slice-playable-character.md](docs/plans/slice-playable-character.md) for
> the overall goal, strategy, and design rationale behind these tasks.

Add a `PlayerControlled` marker component. In `world_setup.rs`, spawn a
creature entity with a placeholder mesh (capsule or cube), `PlayerControlled`,
`Creature`, `MovementSpeed`, and `Thing` components. Place it on a walkable
floor tile. The movement system should only process entities with
`PlayerControlled`.

## Camera — 3D follow camera

> See [docs/plans/slice-playable-character.md](docs/plans/slice-playable-character.md) for
> the overall goal, strategy, and design rationale behind these tasks.

Create `src/camera.rs`. Spawn a `Camera3d` on `OnEnter(AppState::InGame)` with
a fixed-angle perspective looking down at roughly 50 degrees. Add a system that
smoothly follows the `PlayerControlled` entity using lerp. Tag it with
`DespawnOnExit(AppState::InGame)` for automatic cleanup.

## Wire InGame state — world setup and teardown

> See [docs/plans/slice-playable-character.md](docs/plans/slice-playable-character.md) for
> the overall goal, strategy, and design rationale behind these tasks.

Create `src/world_setup.rs` with a `WorldSetupPlugin`. On
`OnEnter(AppState::InGame)`: insert the `Tilemap` resource with the test map,
spawn tile meshes, spawn the player creature, spawn the camera. All InGame
entities should use `DespawnOnExit(AppState::InGame)` so returning to the main
menu cleans up everything. Wire the plugin in `main.rs`.

## Lighting — basic scene illumination

> See [docs/plans/slice-playable-character.md](docs/plans/slice-playable-character.md) for
> the overall goal, strategy, and design rationale behind these tasks.

Add a directional light (sun-like) and ambient light to the InGame scene so
the 3D geometry is visible. Without lighting, `StandardMaterial` meshes render
black. This can be part of `world_setup.rs`. Use `DespawnOnExit(AppState::InGame)`.
