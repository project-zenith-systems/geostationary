# Plan: Technically Playable Character

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
the full game will use. Each module is deliberately minimal — only the portion of
functionality needed for this milestone. The architecture docs define what each
module *will* become; this document defines the thin vertical cut through them.

### Layer participation

| Layer | Module | Plan scope |
|-------|--------|------------|
| L0 | `ui` | Already done (main menu buttons) |
| L0 | `network` | Already done (host + connect) |
| L1 | `tiles` | Grid storage, tile types (Floor/Wall), mesh spawning |
| L1 | `things` | `Thing` marker component, `WorldPosition` component |
| L1 | `main_menu` | Already done (Play triggers hosting) |
| L3 | `creatures` | `Creature` marker, movement speed, wall collision |
| L6 | `camera` | 3D follow camera at fixed angle |

**Not in this plan:** L2 structures/connectables (walls are just a tile type
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
or rotation controls in this plan.

---

## Post-mortem

### Outcome

The plan shipped everything it set out to deliver. Pressing Play hosts a local
server, auto-connects, transitions to InGame, and drops the player into a
lit 3D room with WASD movement and wall collision — exactly the "person in a
box you can walk around in" described at the top of this document. Every layer
in the participation table got its module, the architecture boundaries held,
and 28 tests cover the new code.

### What shipped beyond the plan

| Addition | Why |
|----------|-----|
| `AppConfig` + `load_config` (PR #32) | Centralised window-title and network-port settings early; not planned but prevents magic numbers from spreading. |
| `docs/testing-strategy.md` | Codified "test the seams, not the framework" before writing any module tests. Paid off immediately — tiles and creatures both have pure-logic tests that run without a Bevy `App`. |
| Network hardening (PRs #33, #34) | Task cancellation tokens and a per-frame event-drain cap weren't in scope, but both would have bitten us the moment a second plan touches networking. Cheap to add now, expensive to retrofit later. |

### Deviations from plan

- **`WorldPosition` dropped.** The plan called for a custom `WorldPosition`
  component in `things`. During implementation it became clear that Bevy's
  `Transform` already covers our needs; adding a parallel coordinate was pure
  overhead. `things` still provides the `Thing` marker but nothing spatial.
- **Camera2d stayed alive.** The plan assumed the main-menu camera would be
  cleaned up by `DespawnOnExit`. In practice the UI camera was moved into
  `UiPlugin` and persists across states (order 1), while the 3D camera spawns
  only for InGame (order 0). This is a better long-term design — UI overlays
  will need that camera in every state.

### Hurdles

**1. Black-screen lighting (issue #16, PR #30)**
3D meshes using `StandardMaterial` rendered solid black because no lights
existed. The fix was straightforward (directional + ambient light), but it
overlapped with the camera PR (#31) and required merge-conflict resolution
across both. Lesson: when two PRs touch the same spawn function, sequence them
or extract the shared entity into its own commit first.

**2. Duplicate camera spawn (PR #30)**
After the camera module was split out, `world_setup.rs` still contained its
own `Camera3d` spawn. Both PRs merged, producing two cameras. Caught during
the lighting PR integration and removed. Lesson: track entity ownership in one
place per entity kind — the camera module owns the camera, period.

**3. Tile cleanup false alarm (issue #27, PR #35)**
An issue was filed assuming tile entities would leak on state exit. Five
commits of investigation later, the existing `spawn_tile_meshes` system
already handled this: when the `Tilemap` resource is removed by
`cleanup_world`, the system detects the missing resource next frame and
despawns all tile entities. PR closed with no code changes, only a
verification test. Lesson: read the existing cleanup paths before assuming
they're missing.

**4. Network tasks leaking ports (issue #19, PR #33)**
`HostLocal` spawned a tokio task with no handle or cancellation. Pressing Play
twice bound the port twice. Fixed by introducing `CancellationToken`,
`NetworkTasks` resource, and `StopHosting` / `Disconnect` commands. Took six
commits and multiple review rounds to get stale-state detection and
connection-handler propagation right. Lesson: any long-lived async task needs
a kill switch from day one — retrofitting is harder.

**5. Unbounded event drain (issue #18, PR #34)**
`drain_net_events` consumed the entire channel each frame. Fine now with
negligible traffic, but a time bomb for when packet events arrive at scale.
Capped at 100 events/frame with carry-over and a one-time log warning.

### Remaining open issues

| Issue | Note |
|-------|------|
| #26 — Gate tile mesh spawning to InGame | Architecturally impure (runs every frame in every state) but functionally harmless. Should be a quick fix in the integration layer. |
| #20 — Configurable TLS server name | Only matters for remote connections; irrelevant to this local-only plan. |
| #17 — Config management expansion | Foundation exists; expansion is future work. |

### What went well

- **Small PRs.** ~15 focused PRs made review fast and isolated breakage.
- **Layer discipline held.** No layer-boundary violations. Tokio stayed sealed
  inside `modules/network`. Game code never imports async types.
- **Tests from the start.** Every new module shipped with tests. The tile
  cleanup "bug" was disproven by writing a test, not by guessing.

### What to do differently next time

- **Sequence overlapping PRs explicitly.** The camera/lighting collision was
  avoidable with a dependency edge in the issue tracker.
- **Verify before filing.** Issue #27 cost investigation time for a problem
  that didn't exist. A five-minute code read would have closed it immediately.
- **Decide entity ownership up front.** The duplicate camera spawn came from
  two modules both thinking they owned the camera. A one-line comment ("camera
  is owned by `camera.rs`") would have prevented it.
