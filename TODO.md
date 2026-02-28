# TODO — Pick Up and Drop Networked Items

**Plan:** `plan/networked-items` · [docs/plans/networked-items.md](docs/plans/networked-items.md)

## Spike: Reparenting and physics component removal in Avian

In a minimal Bevy + Avian scene, spawn a `RigidBody::Dynamic` entity with a
`Collider`, then at runtime: remove `RigidBody`/`Collider`/`LinearVelocity`/
`GravityScale`, reparent the entity as a child of another entity, and set a
local transform. Verify with unit tests:

- (a) The entity moves with its parent (transform propagation works)
- (b) Re-inserting physics components and deparenting restores normal
  physics behaviour (entity falls, collides)
- (c) No panics or warnings from Avian when components are removed and
  re-inserted at runtime
- (d) `Visibility::Hidden` on a reparented entity prevents rendering

**Question to answer:** does Avian handle dynamic component removal and
re-insertion cleanly, or do we need to despawn/respawn?

**Plan:** `plan/networked-items` · [docs/plans/networked-items.md](docs/plans/networked-items.md)

## Add `button` field to `WorldHit` and fire raycasters on both buttons

`modules/input/src/lib.rs`, `modules/things/src/lib.rs`,
`modules/tiles/src/lib.rs`.

- Add `button: MouseButton` field to `WorldHit` in the `input` module
- Update `raycast_things` in `things` to fire on both left-click and
  right-click `PointerAction` events (currently right-click only), passing
  `button` through to `WorldHit`
- Update `raycast_tiles` in `tiles` with the same both-buttons change,
  passing `button` through to `WorldHit`
- Update all existing `WorldHit` construction sites to include `button`

**Not included:** `ResolvedHit` or any interactions module changes (that
comes in the interactions task).

**Depends on:** nothing.

**Plan:** `plan/networked-items` · [docs/plans/networked-items.md](docs/plans/networked-items.md)

## HandSlot component and hand anchor child entity

`modules/things/src/lib.rs`.

- New `HandSide` enum (`Left`, `Right`)
- New `HandSlot { side: HandSide }` component
- `spawn_player_creature` gains a child entity with
  `HandSlot { side: Right }`, `Container { capacity: 1 }`, and
  `Transform::from_translation(HAND_OFFSET)` — this is the named anchor
  for held items
- The kind 0 `ThingRegistry` template (creature) must also spawn the
  `HandSlot` child on the client side (so joining clients reconstruct it)
- Unit tests: creature spawn produces a child entity with `HandSlot` and
  `Container { capacity: 1 }`

**Not included:** `ItemEvent` variants, `broadcast_item_event`,
`handle_item_event`, `StateUpdate` exclusion (those come in later tasks).

**Depends on:** nothing (but must merge before the items module task).

**Plan:** `plan/networked-items` · [docs/plans/networked-items.md](docs/plans/networked-items.md)

## Items module: `Item`, `Container`, `StashedPhysics`, and server-side item interaction handling

New module: `modules/items/`.

- `modules/items/Cargo.toml` — depends on `bevy`, `things` (for `HandSlot`,
  `NetId`), `physics` (for `RigidBody`, `Collider`, `LinearVelocity`)
- Root `Cargo.toml` — add `items` to workspace members and `[dependencies]`
- `Item` marker component
- `Container { capacity: usize, slots: Vec<Option<Entity>> }` component
- `StashedPhysics { collider: Collider, gravity: GravityScale }` component
- `ItemPickupRequest`, `ItemDropRequest`, `ItemStoreRequest`,
  `ItemTakeRequest` Bevy events
- Server system `handle_item_interaction` — reads all four request events,
  validates `interaction_range` + item existence + container space, executes:
  - Pickup: stash physics, remove physics components, reparent to HandSlot,
    update Container
  - Drop: restore physics from StashedPhysics (not ConstantForce), deparent,
    set world position
  - Store: deparent from HandSlot, hide, insert into target Container
  - Take: remove from target Container, show, reparent to HandSlot
- `ItemsPlugin` struct, added to plugin chain in `src/main.rs`
- Unit tests for all four operations: validation (in-range, out-of-range,
  hand-full, hand-empty, container-full, container-empty, non-item entity)
  and mechanics (physics removed/restored, Container slots updated,
  visibility toggled, StashedPhysics lifecycle)

**Not included:** `ItemEvent` broadcasting, wire types, replication, context
menu actions. The items module is server-side logic only in this task.

**Depends on:** HandSlot component task, spike results.

**Plan:** `plan/networked-items` · [docs/plans/networked-items.md](docs/plans/networked-items.md)

## ItemEvent replication on stream 3

`modules/things/src/lib.rs`.

- Add `ItemEvent` enum to `ThingsStreamMessage`:
  - `PickedUp { item: NetId, holder: NetId }`
  - `Dropped { item: NetId, position: [f32; 3] }`
  - `Stored { item: NetId, container: NetId }`
  - `Taken { item: NetId, holder: NetId }`
- Server-side `broadcast_item_event` system — runs after
  `handle_item_interaction`, reads a Bevy event (fired by the items module
  after each successful operation), sends `ItemEvent` on stream 3 to all
  clients
- Client-side `handle_item_event` system — drains `ItemEvent` from stream 3,
  applies reparenting/deparenting, strips/restores physics components, sets
  visibility, updates local `Container.slots` on both hand and target
  containers
- Exclude held items from `StateUpdate`: items parented to a `HandSlot`
  skip the 30 Hz position broadcast
- Initial sync on connect: after all `EntitySpawned` messages, server
  replays `ItemEvent::PickedUp` for held items and `ItemEvent::Stored` for
  items inside containers

**Depends on:** items module task.

**Plan:** `plan/networked-items` · [docs/plans/networked-items.md](docs/plans/networked-items.md)

## Item spawning and `InteractionConfig`

`src/world_setup.rs`, `src/config.rs`.

- Add `InteractionConfig { interaction_range: f32 }` section to `AppConfig`
  in `src/config.rs`, default `2.0`
- Register item templates in `ThingRegistry`:
  - Kind 2 (can): small cylinder mesh, metallic material,
    `Collider::cylinder(0.15, 0.1)`, `RigidBody::Dynamic`,
    `GravityScale(1.0)`, `Item` marker, `Name::new("Can")`
  - Kind 3 (toolbox): box mesh, coloured material,
    `Collider::cuboid(0.3, 0.15, 0.2)`, `RigidBody::Dynamic`,
    `GravityScale(1.0)`, `Item` marker, `Name::new("Toolbox")`,
    `Container { capacity: 6 }`
- Spawn two cans and one toolbox in the pressurised chamber
- Toolbox spawns pre-loaded with one can inside it: can entity ref in
  `Container.slots[0]`, `Visibility::Hidden`, `StashedPhysics`, no
  `RigidBody`/`Collider`/`LinearVelocity`

**Depends on:** items module task.

**Plan:** `plan/networked-items` · [docs/plans/networked-items.md](docs/plans/networked-items.md)

## Unified interactions stream and tile-toggle migration

`modules/interactions/src/lib.rs`, `modules/tiles/src/lib.rs`.

- **Tiles removal:** delete `TILE_TOGGLE_STREAM_TAG`, `TileToggle` wire
  type, stream 4 registration, `execute_tile_toggle`, `handle_tile_toggle`,
  and `TileToggleRequest` from `tiles`. Retain `Tilemap`, `TileKind`,
  `TileMutated` wire message type, tile rendering, stream 1,
  `apply_tile_mutation`.
- **Interactions stream 4:** register stream 4 (client→server) in the
  `interactions` module. Define `InteractionRequest` wire enum with
  variants: `TileToggle`, `ItemPickup`, `ItemDrop`, `StoreInContainer`,
  `TakeFromContainer`.
- **Client:** `send_interaction` system serialises `InteractionRequest` on
  stream 4.
- **Server:** `dispatch_interaction` system drains stream 4. Handles tile
  toggle directly (validates, mutates `Tilemap`, broadcasts `TileMutated`
  on stream 1 as wire message AND fires it as local Bevy event for
  `apply_tile_mutation`). For item operations, fires the corresponding
  Bevy events (`ItemPickupRequest`, `ItemDropRequest`, `ItemStoreRequest`,
  `ItemTakeRequest`).
- Add `items` dependency to `modules/interactions/Cargo.toml`
- Unit test: `dispatch_interaction` handles `TileToggle` correctly
  (existing tile-toggle behaviour preserved after migration)

**Not included:** `ResolvedHit`, `default_interaction`, extended context
menu (those come in the next task).

**Depends on:** items module task, WorldHit button field task.

**Plan:** `plan/networked-items` · [docs/plans/networked-items.md](docs/plans/networked-items.md)

## Raycast resolution, default interaction, and extended context menu

`modules/interactions/src/lib.rs`.

- **Raycast resolution (fixes #169):** `resolve_world_hits` system collects
  all `WorldHit` messages per frame, picks the hit closest to the camera,
  emits `ResolvedHit { hit: WorldHit }`. Runs after `raycast_tiles` and
  `raycast_things`.
- **Default action:** `default_interaction` system reads left-click
  `ResolvedHit`. If hit entity has `Item` component, sends
  `InteractionRequest::ItemPickup { target }` on stream 4.
- **Extended context menu:** `build_context_menu` reads right-click
  `ResolvedHit` (instead of `WorldHit` directly). Action table gains:
  - `Item` on floor → ["Pick up"]
  - `Tile(Floor)` while holding item → ["Drop", "Build Wall"]
  - `Container` entity while holding item → ["Store in {name}"]
  - `Container` entity with items, hand empty → ["Take from {name}"]
- `handle_menu_selection` sends the corresponding `InteractionRequest`
  variant on stream 4 for each action
- Uses `Name` component for context menu labels
- Unit tests: raycast resolution (single hit, multiple hits picks closest,
  no hits); default interaction (left-click Item sends pickup, left-click
  non-Item does nothing); context menu entries for each combination of
  hand state and target entity

**Depends on:** unified interactions stream task.
