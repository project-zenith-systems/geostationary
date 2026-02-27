# Plan: Pick Up and Drop Networked Items

> **Stage goal:** Item entities spawn in the world, can be picked up and put
> down by players, and placed into containers. Picking up an item reparents
> it to the player creature's hand anchor, visible to all clients. Containers
> (hands, toolboxes) can nest: a hand holds a toolbox, which holds cans. All
> interactions are server-authoritative with range validation, and every
> client sees the same thing. This is plan 3 of the Networked World State arc.

## What "done" looks like

1. The test room contains pre-placed items: two cans and a toolbox on the
   pressurised side of the room
2. Items are Dynamic rigid bodies — decompression pushes loose cans and the
   toolbox toward the breach, just like the ball and creatures
3. Left-clicking an item within range picks it up instantly (no context menu);
   the item disappears from the floor and appears attached to the player
   creature's hand
4. Right-clicking an item shows a context menu with "Pick up"; selecting it
   has the same effect as left-clicking
5. Right-clicking a floor tile while holding an item shows "Drop" in the
   context menu; selecting it drops the item at the clicked floor position
6. All pickup and drop actions are server-authoritative — the client sends a
   request, the server validates range and executes the mutation, and all
   clients see the result
7. The toolbox has a `Container` component with capacity 6 — the data model
   supports nesting. Container UI and store/take actions are deferred to a
   future plan; for now the toolbox is just a pickable item that happens to
   have container data
8. A player creature has a `HandSlot` child entity at a fixed offset; held
   items are parented to this entity and move with the creature
9. Picking up a toolbox that contains items works — the toolbox and its
   contents move together (nested containers)
10. A second player sees all of the above in real time: items appear/disappear
    from the floor, appear in the other player's hand, and drop back to the
    floor correctly
11. Items dropped near a pressure breach get pushed by the pressure gradient

## Strategy

The break-a-wall post-mortem confirmed bottom-up sequencing, spike-first
design, and the established mutation flow pattern (client fires event →
stream 4 → server validates → broadcast result). This plan follows the same
pattern for item interactions.

**Core abstraction — containers.** The key insight is that a hand is a
container. A toolbox is a container. "Pick up" means "move item from floor
into hand container." "Drop" means "move item from hand container to floor."
"Store" means "move item from hand container to toolbox container." All
operations are the same primitive: **move item from source to destination.**
The server validates the operation and broadcasts the result.

**Item lifecycle — reparenting, not despawn/respawn.** When an item is picked
up, it stays as a live entity. Its `Transform` is reset to a local offset,
it is reparented as a child of the `HandSlot` anchor entity, and its physics
components (`RigidBody`, `Collider`, `LinearVelocity`) are removed to
prevent physics simulation while held. When dropped, physics components are
re-inserted and the item is deparented to world space at the creature's
position. This preserves entity identity and avoids the complexity of
despawn/respawn replication.

**Replication — dedicated ItemEvent messages.** Pickup and drop are
replicated via `ItemEvent` messages on stream 3 (entity stream), not via
`StateUpdate`. `ItemEvent::PickedUp { item: NetId, holder: NetId }` tells
clients to reparent the item under the holder's `HandSlot`.
`ItemEvent::Dropped { item: NetId, position: Vec3 }` tells clients to
deparent and place the item at the given world position. This is more
explicit than extending `StateUpdate` with optional parent data, and
generalises cleanly to future container operations.

**Interaction model — left-click default + context menu.** The `interactions`
module gains the concept of a "default action" — left-clicking a target
performs its primary interaction without showing a menu. For items on the
floor, the default action is "Pick up." Right-click still shows the full
context menu with all available actions. This extends the existing
`WorldHit` → action lookup → fire event pipeline without changing the
architecture.

**Unified interactions stream (stream 4).** The break-a-wall plan placed
stream 4 in `tiles` with a note that it was temporary. This plan promotes
stream 4 to a general-purpose **interactions stream** owned by the
`interactions` module (L6). All client→server interaction requests —
tile toggles, item pickups, item drops, and any future interactions — flow
through a single `InteractionRequest` wire enum on stream 4. The
`interactions` module registers the stream, serialises requests on the
client, and dispatches them on the server by firing domain-specific Bevy
events (`TileToggleRequest`, `ItemPickupRequest`, `ItemDropRequest`)
downward. Lower-layer modules (`tiles`, `items`) never touch stream 4
directly — they only read Bevy events. This removes `execute_tile_toggle`
from `tiles` and moves that responsibility into `interactions`.

**Data flow — item pickup:**

```
Client                          Server                          Other clients
  |                               |                                |
  | left-click on item            |                                |
  | -> input: PointerAction       |                                |
  | -> things: raycast_things     |                                |
  |    -> WorldHit { entity }     |                                |
  | -> interactions: default_action|                               |
  |    -> InteractionRequest      |                                |
  |       ::ItemPickup { target } |                                |
  | --- stream 4, c->s --------->|                                |
  |                               | interactions: dispatch         |
  |                               |   fires ItemPickupRequest      |
  |                               | items: handle_item_interaction  |
  |                               |   validate: in range?          |
  |                               |   validate: hand has space?    |
  |                               |                                |
  |                               | reparent item -> hand anchor   |
  |                               | remove physics components      |
  |                               | update Container data          |
  |                               |                                |
  |                               | --- ItemEvent::PickedUp ------>|
  | <-- ItemEvent::PickedUp ------|     (stream 3, s->c)           |
  |                               |                                |
  | reparent item locally         |                                | reparent item locally
  | remove physics locally        |                                | remove physics locally
```

**Raycast priority (fixes #169).** Both `raycast_tiles` and
`raycast_things` fire independently and may emit `WorldHit` for the same
click. The `interactions` module resolves conflicts by comparing
`world_pos.y` (or distance to camera) — the closest hit wins. This is
implemented as a `resolve_world_hits` system that collects all `WorldHit`
events in a frame and emits a single `ResolvedHit` event. Downstream
systems (`build_context_menu`, `default_interaction`) read `ResolvedHit`
instead of `WorldHit` directly. This solves #169 generically — any future
raycaster (items, machines) just emits `WorldHit` and the resolution logic
handles priority.

**Stream architecture (changes in bold):**

| Stream tag | Owner              | Direction           | Content                                              |
| ---------- | ------------------ | ------------------- | ---------------------------------------------------- |
| 0          | `network`          | bidirectional       | Welcome, InitialStateDone / Hello, Input (unchanged) |
| 1          | `tiles`            | server -> client    | TilemapData, TileMutated, StreamReady (unchanged)    |
| 2          | `atmospherics`     | server -> client    | GasGridData, GasGridDelta, StreamReady (unchanged)   |
| 3          | `things`           | server -> client    | EntitySpawned, StateUpdate, **ItemEvent**, StreamReady|
| **4**      | **`interactions`** | **client -> server**| **InteractionRequest { TileToggle, ItemPickup, ItemDrop }** |

Stream 4 ownership moves from `tiles` to `interactions`. The `TileToggle`
wire type is replaced by `InteractionRequest::TileToggle` — same data,
different envelope. `tiles` loses its stream 4 registration,
`execute_tile_toggle`, and `handle_tile_toggle`; all tile-toggle logic
(validation, tilemap mutation, broadcast) moves into `interactions`'
`dispatch_interaction` system. `interactions` gains `send_interaction`
(client) and `dispatch_interaction` (server).

### Layer participation

| Layer | Module          | Systems / changes | Schedule / run condition |
|-------|-----------------|-------------------|--------------------------|
| L0    | `input`         | No changes. `PointerAction` and `WorldHit` already sufficient. | — |
| L1    | `tiles`         | **Remove all stream 4 and tile-toggle systems.** Delete `TILE_TOGGLE_STREAM_TAG`, `TileToggle` wire type, stream 4 registration, `execute_tile_toggle`, `handle_tile_toggle`, `TileToggleRequest`, and `TileMutated` Bevy events. All tile-toggle logic (validation, tilemap mutation, `TileMutated` broadcast on stream 1) moves to `interactions`. `tiles` retains `Tilemap`, `TileKind`, tile mesh spawning, `apply_tile_mutation`, and stream 1 (server→client tilemap data). | No schedule changes to remaining systems. |
| L1    | `things`        | **HandSlot component.** New `HandSlot { side: HandSide }` component and `HandSide` enum (`Left`, `Right`). **Hand anchor spawning:** `spawn_player_creature` gains a child entity with `HandSlot { side: Right }` and `Transform::from_translation(HAND_OFFSET)`. **ItemEvent on stream 3:** new enum variants `ItemEvent::PickedUp { item: NetId, holder: NetId }` and `ItemEvent::Dropped { item: NetId, position: [f32; 3] }` added to `ThingsStreamMessage`. Server-side: `broadcast_item_event` sends `ItemEvent` after item mutations. Client-side: `handle_item_event` receives and applies reparenting/deparenting. **Exclude held items from StateUpdate:** items parented to a `HandSlot` should not be included in the 30 Hz position broadcast (they inherit transform from parent). | `broadcast_item_event`: `Update`, after item systems. `handle_item_event`: `Update`, after stream drain, gated on `Client`. |
| L2    | `items`         | **New module.** `Item` marker component (on all item entities). `Container` component with `slots: Vec<Option<Entity>>` and `capacity: usize`. `ItemPickupRequest` and `ItemDropRequest` Bevy events (fired by `interactions`, read here). **Server systems:** `handle_item_interaction` reads `ItemPickupRequest`/`ItemDropRequest` events, validates range + item existence + container space, executes pickup (reparent + strip physics) or drop (deparent + restore physics + set position). **No client-side simulation** — clients apply `ItemEvent` messages from stream 3. | `handle_item_interaction`: `Update`, after `dispatch_interaction`, gated on `Server`. |
| L6    | `interactions`  | **Unified interactions stream (stream 4).** Owns `InteractionRequest` wire enum and stream 4 registration (client→server). Client: `send_interaction` serialises requests. Server: `dispatch_interaction` drains stream 4 and handles all interaction types directly — tile toggle (validates, mutates `Tilemap`, broadcasts `TileMutated` on stream 1), item pickup (fires `ItemPickupRequest` for `items` module), item drop (fires `ItemDropRequest` for `items` module). **Raycast resolution (fixes #169).** `resolve_world_hits` system collects all `WorldHit` events per frame, picks closest to camera, emits `ResolvedHit`. **Default action system.** `default_interaction` reads left-click `ResolvedHit`; for items on floor, sends `InteractionRequest::ItemPickup`. **Extended context menu.** Action table gains: `Item` -> ["Pick up"]; `Tile(Floor)` while holding -> ["Drop", "Build Wall"]. | `resolve_world_hits`: `Update`, after `raycast_tiles` and `raycast_things`. `send_interaction`: `Update`, gated on `not(Headless)`. `dispatch_interaction`: `Update`, after stream drain, gated on `Server`. |
| —     | `src/world_setup.rs` | **Item and container spawning.** Register item templates in `ThingRegistry`: can (kind 2), toolbox (kind 3). Spawn two cans and one toolbox in the pressurised chamber. Toolbox entity gets `Container { capacity: 6 }`. | `setup_world`: `OnEnter(InGame)`, gated on `Server`. |

### Not in this plan

- **Container UI.** No visual panel for viewing container contents. Items
  are moved between containers via context menu actions only. The "Open"
  action on toolbox is a stub.
- **Store in / take from container actions.** Only pickup from floor and
  drop from hand are implemented. Moving items between containers (hand ->
  toolbox, toolbox -> hand) is deferred until container UI exists.
- **Equipment slots.** No body slots, no clothing, no wielding. Hand is
  just a container with capacity 1.
- **Item-specific behaviour.** Cans and toolboxes are inert. No drinking,
  no tool usage.
- **Item stacking or quantity.** Each item is a unique entity.
- **Client-side prediction.** Client waits for server confirmation.
- **HUD / inventory display.** No UI showing what the player is holding.
- **Sound effects.** No audio for pickup or drop.
- **Line-of-sight validation.** Range check only (N meters). LoS is a
  future concern.
- **Death/drop-all.** No mechanic for dropping items on disconnect or death.
  Soul unbinding leaves the creature (and its held items) in the world.

### Module placement

```
modules/
  items/                         # NEW MODULE (L2)
    Cargo.toml
    src/
      lib.rs                     # Item, Container components.
                                 #   ItemPickupRequest, ItemDropRequest events.
                                 #   Server: handle_item_interaction, pickup_item,
                                 #   drop_item helpers.
                                 #   Container query helpers.
  things/
    src/
      lib.rs                     # MODIFIED — HandSlot, HandSide components.
                                 #   Hand anchor child entity in
                                 #   spawn_player_creature.
                                 #   ItemEvent variants in ThingsStreamMessage.
                                 #   broadcast_item_event, handle_item_event.
  tiles/
    src/
      lib.rs                     # MODIFIED — remove stream 4 registration,
                                 #   TileToggle wire type, execute_tile_toggle,
                                 #   handle_tile_toggle, TileToggleRequest,
                                 #   TileMutated events. Retain Tilemap, tile
                                 #   rendering, stream 1, apply_tile_mutation.
  interactions/
    src/
      lib.rs                     # MODIFIED — owns stream 4 (InteractionRequest
                                 #   wire enum). Client: send_interaction.
                                 #   Server: dispatch_interaction (handles tile
                                 #   toggle directly, fires ItemPickupRequest /
                                 #   ItemDropRequest for items module).
                                 #   default_interaction for left-click pickup.
                                 #   resolve_world_hits for raycast priority.
  input/
    src/
      lib.rs                     # NO CHANGES
  network/
    src/
      lib.rs                     # NO CHANGES — stream infrastructure sufficient
  atmospherics/
    src/
      lib.rs                     # NO CHANGES — pressure forces already apply
                                 #   to all Dynamic RigidBody entities
src/
  world_setup.rs                 # MODIFIED — register can/toolbox templates,
                                 #   spawn items in test room
  server.rs                      # NO CHANGES expected
  client.rs                      # NO CHANGES expected
  main.rs                        # MODIFIED — add ItemsPlugin to plugin chain
```

### Dependency and wiring changes

- **Root `Cargo.toml`:** Add `items` to workspace members and root
  `[dependencies]`
- **`modules/items/Cargo.toml`:** New. Depends on `bevy`, `things` (for
  `HandSlot`, `NetId`, `spawn_thing`), `physics` (for `RigidBody`,
  `Collider`, `LinearVelocity`)
- **`modules/interactions/Cargo.toml`:** Add `items` (for
  `ItemPickupRequest`, `ItemDropRequest`, `Item` component), `network`
  (already present — for `StreamSender`, `StreamReader`, `StreamDef`
  to own stream 4)
- **`modules/tiles/Cargo.toml`:** Remove `input` dep if no longer needed
  (raycast_tiles still uses `PointerAction` from `input`, so likely kept)
- **`modules/things/Cargo.toml`:** No new deps — `HandSlot` is internal
- **`src/main.rs`:** Add `ItemsPlugin` to both headless and client plugin
  chains (items module has both server and client systems)

### Item entity design

Items are `Thing` entities (spawned via `spawn_thing`) with an additional
`Item` marker component. The `ThingRegistry` template for each item kind
inserts `Item` and kind-specific visual/physical properties.

**Can (kind 2):**
- Small cylinder mesh, metallic material
- `Collider::cylinder(0.15, 0.1)` (half-height 0.15, radius 0.1)
- `RigidBody::Dynamic`, `GravityScale(1.0)`
- `Item` marker

**Toolbox (kind 3):**
- Box mesh, coloured material
- `Collider::cuboid(0.3, 0.15, 0.2)` (half-extents)
- `RigidBody::Dynamic`, `GravityScale(1.0)`
- `Item` marker
- `Container { capacity: 6, slots: vec![None; 6] }`

### Container design

A `Container` component holds a fixed-size vector of optional entity
references:

```
Container {
    capacity: usize,
    slots: Vec<Option<Entity>>,
}
```

**A hand is a container.** Each creature's `HandSlot` entity also gets a
`Container { capacity: 1 }`. "Pick up" = insert item entity into the hand's
container. "Drop" = remove item from hand's container, restore to world.

**Containers can nest.** A toolbox in a hand means the hand's
`Container.slots[0]` holds the toolbox entity, and the toolbox entity's own
`Container` still holds its items. The data model is a tree of entities
linked by `Container.slots` references. Reparenting in the ECS hierarchy
mirrors this — a picked-up toolbox becomes a child of `HandSlot`, and the
toolbox's contained items (if any) are already children of the toolbox (or
virtual — stored as entity refs in `Container.slots` without being ECS
children). For this plan, contained items inside a toolbox are **data-only
references** in `Container.slots` — they don't need to be visible, so they
don't need to be ECS children of the toolbox. Only the actively held item
(the one in the hand) is visually parented.

### Pickup and drop mechanics

**Pickup (server-side):**
1. Validate: item entity exists, has `Item` component
2. Validate: requester's creature is within `INTERACTION_RANGE` meters
3. Validate: requester's hand `Container` has an empty slot
4. Remove `RigidBody`, `Collider`, `LinearVelocity`, `ConstantForce` from
   item entity
5. Set item's `Transform` to local offset (small, near hand)
6. Reparent item entity as child of `HandSlot` entity
7. Insert item entity ref into hand `Container.slots[0]`
8. Broadcast `ItemEvent::PickedUp { item, holder }` on stream 3

**Drop (server-side):**
1. Validate: requester's hand `Container.slots[0]` is `Some(entity)`
2. Validate: requested drop position is within `INTERACTION_RANGE` of creature
3. Remove item from hand `Container.slots[0]`
4. Deparent item from `HandSlot`
5. Set item's `Transform` to the requested drop position (validated in range)
6. Re-insert `RigidBody::Dynamic`, `Collider`, `LinearVelocity::ZERO`,
   etc. (restore physics)
7. Broadcast `ItemEvent::Dropped { item, position }` on stream 3

**Client-side (on receiving ItemEvent):**
- `PickedUp`: Look up item and holder by `NetId`, reparent item under
  holder's `HandSlot`, remove physics components, set local transform
- `Dropped`: Look up item by `NetId`, deparent, set world transform,
  re-insert physics components

### Default interaction design

The `interactions` module gains a `default_interaction` system that runs on
left-click `WorldHit` events (as opposed to `build_context_menu` which runs
on right-click):

1. Read `WorldHit` from left-click `PointerAction`
2. Query the hit entity for components: `Item`, `Tile`, etc.
3. If entity has `Item` → fire `ItemPickupRequest { target: entity }`
4. Future: other default actions for other entity types

The `interactions` module handles this directly: it resolves the entity's
`NetId` and sends `InteractionRequest::ItemPickup { target }` on stream 4.

For drop: the context menu action table checks if the player is holding an
item (hand container is non-empty) and the right-click target is a floor
tile. If both conditions hold, "Drop" appears alongside "Build Wall" in the
context menu. Selecting it sends `InteractionRequest::ItemDrop { hand_side,
position }` on stream 4. On the server, `dispatch_interaction` fires
`ItemDropRequest` which the `items` module handles.

### Replication design

**On connect (initial sync):** Items are already replicated as `Thing`
entities via stream 3 (`EntitySpawned`). The `ThingRegistry` template
reconstructs them with correct mesh/collider/Item marker. Held items need
additional information: after sending all `EntitySpawned` messages, the
server sends `ItemEvent::PickedUp` for each currently held item so the
joining client reparents them correctly.

**Ongoing:** `ItemEvent` messages on stream 3 handle all pickup/drop state
changes. `StateUpdate` (position/velocity at 30 Hz) continues for items on
the floor. Held items don't need `StateUpdate` — they inherit transform
from their parent.

### WorldHit resolution design (fixes #169)

Currently `raycast_tiles` and `raycast_things` independently emit `WorldHit`
events. With items on the floor, a single right-click can produce two hits
(the item entity and the floor tile beneath it). The `interactions` module
needs to pick one.

**`ResolvedHit` event.** A new `ResolvedHit` message in the `interactions`
module wraps `WorldHit` after priority resolution. The `resolve_world_hits`
system:

1. Collects all `WorldHit` events from the current frame
2. Groups by originating `PointerAction` (same `screen_pos` and `button`)
3. For each group, picks the hit closest to the camera (smallest distance
   from camera `Transform.translation` to `world_pos`)
4. Emits `ResolvedHit { hit: WorldHit, button: MouseButton }`

Downstream systems (`build_context_menu`, `default_interaction`) read
`ResolvedHit` instead of `WorldHit`. This is a minimal change — the
interactions module already sits between input and action dispatch.

The `WorldHit` message type stays in `input` (L0). `ResolvedHit` lives in
`interactions` (L6). No lower-layer changes needed.

### Interaction range validation

Server-side pickup validation checks Euclidean distance between the
requester's creature `Transform` and the target item's `Transform`. The
range constant `INTERACTION_RANGE` (e.g. `2.0` meters) is defined in the
items module. Line-of-sight is noted as a future concern but not implemented.

### Spikes

One spike precedes implementation:

1. **Reparenting spike** — In a minimal Bevy scene, spawn a dynamic rigid
   body, then at runtime: remove `RigidBody`/`Collider`/`LinearVelocity`,
   reparent it as a child of another entity, set a local transform. Verify:
   (a) the entity moves with its parent, (b) re-inserting physics components
   and deparenting restores normal physics behaviour, (c) no panics or
   warnings from Avian when components are removed/re-inserted. Question to
   answer: does Avian handle dynamic component removal and re-insertion
   cleanly, or do we need to despawn/respawn?

## Post-mortem

_(To be filled in after the plan ships.)_
