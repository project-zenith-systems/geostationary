# Slice: Technically Playable Character

> **Stage goal:** A 3D character walks around a small tile-based room with walls.
> Graphics are placeholder. The full creature/tile/thing systems are stubbed to
> their minimum viable shape — just enough to prove the layer stack works
> end-to-end.

## What "done" looks like

Clicking Play hosts a local server, connects, transitions to InGame, and spawns:

1. A small room made of floor tiles and wall tiles (3D cubes/planes)
2. A player character (placeholder capsule) standing in the room
3. A 3D camera looking down at an angle, following the player
4. WASD movement that slides the character across floor tiles
5. Walls block movement (simple collision)

No animations, no creatures AI, no items, no atmospherics. Just a person in a
box that you can walk around in.

## Strategy

Build upward through the layer stack, establishing the module boundaries that
the full game will use. Each module is deliberately minimal — only the slice of
functionality needed for this milestone. The architecture docs define what each
module *will* become; this document defines the thin vertical cut through them.

### Layer participation

| Layer | Module | Slice scope |
|-------|--------|-------------|
| L0 | `ui` | Already done (main menu buttons) |
| L0 | `network` | Already done (host + connect) |
| L1 | `tiles` | Grid storage, tile types (Floor/Wall), mesh spawning |
| L1 | `things` | `Thing` marker component, `WorldPosition` component |
| L1 | `main_menu` | Already done (Play triggers hosting) |
| L3 | `creatures` | `Creature` marker, movement speed, wall collision |
| L6 | `camera` | 3D follow camera at fixed angle |

**Not in this slice:** L2 structures/connectables (walls are just a tile type
for now), L4 mechanics, L5 player/souls distinction (player *is* the creature
for now), L0 input abstraction (raw Bevy input is fine), L0 physics (manual
grid collision is enough).

### Module placement

Following the existing convention (`modules/` for workspace crates,
`src/` for game systems):

```
modules/
  tiles/          # L1 workspace crate — grid primitives
  things/         # L1 workspace crate — world object primitives
src/
  creatures/      # L3 — creature components and movement
  camera.rs       # L6 — follow camera (single file for now)
  world_setup.rs  # InGame state initialisation — spawns map + player
```

### 3D transition

The current app uses `Camera2d` for the main menu. InGame state needs 3D.
The approach: keep `Camera2d` for UI (main menu), spawn a `Camera3d` when
entering InGame. The 2D camera is already cleaned up automatically via
`DespawnOnExit(AppState::MainMenu)`.

### Tile system design

The tile grid is a flat 2D array stored in a `Tilemap` resource. Each cell
holds a `TileKind` enum (Floor, Wall). On map load, the system iterates the
grid and spawns a 3D entity per tile:

- **Floor:** flat plane at y=0, dark grey
- **Wall:** unit cube at y=0.5 (half-height offset), lighter grey

Tile coordinates are integer (i32, i32). World position maps 1 tile = 1 unit.

### Movement design

Free movement within the grid (not tile-snapped). The creature has a
`Transform` and moves continuously. Before applying movement, the system
checks whether the target position's tile is walkable. If not, the axis is
blocked (slide along walls).

This keeps movement feeling natural while respecting the grid structure.
Proper physics-based collision is a future concern (L0 physics).

### Camera design

Fixed-angle perspective camera looking down at roughly 45-60 degrees.
Smoothly follows the player position with a slight lag (lerp). No zoom
or rotation controls in this slice.
