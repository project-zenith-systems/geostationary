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
7. The toolbox has a `Container` component with capacity 6 — store/take
   actions work via context menu. Container UI (drag-and-drop, inventory
   grid) is deferred to a future plan
8. A player creature has a `HandSlot` child entity at a fixed offset; held
   items are parented to this entity and move with the creature
9. The toolbox spawns pre-loaded with one can inside it; picking up the
   toolbox moves it and its contents together (nested containers)
10. While holding an item, right-clicking a container (e.g. toolbox on the
    floor) shows "Store in toolbox" — selecting it stores the held item
    inside the container and frees the hand
11. While the hand is empty, right-clicking a container that has items shows
    "Take from [name]" — selecting it moves the first item from the container
    into the hand
12. A second player sees all of the above in real time: items appear/disappear
    from the floor, appear in the other player's hand, and drop back to the
    floor correctly
13. Items dropped near a pressure breach get pushed by the pressure gradient

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
The server validates the operation and broadcasts the result. This plan
implements all four: pickup (floor → hand), drop (hand → floor), store
(hand → container), and take (container → hand).

**Item lifecycle — reparenting, not despawn/respawn.** When an item is picked
up, it stays as a live entity. Its `Transform` is reset to a local offset,
it is reparented as a child of the `HandSlot` anchor entity, and its physics
components (`RigidBody`, `Collider`, `LinearVelocity`, `GravityScale`,
`ConstantForce`) are removed to prevent physics simulation while held. The
original collider and gravity are preserved in a `StashedPhysics` component.
When dropped, physics components are restored from `StashedPhysics` and the
item is deparented to world space at the creature's
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
events (`TileToggleRequest`, `ItemPickupRequest`, `ItemDropRequest`,
`ItemStoreRequest`, `ItemTakeRequest`) downward. Lower-layer modules (`tiles`, `items`) never touch stream 4
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
click. The `interactions` module resolves conflicts by comparing the
distance from the camera to each hit — the smallest camera-to-hit distance
wins. This is implemented as a `resolve_world_hits` system that collects
all `WorldHit` events in a frame and emits a single `ResolvedHit` event.
Downstream systems (`build_context_menu`, `default_interaction`) read
`ResolvedHit` instead of `WorldHit` directly. This solves #169 generically —
any future raycaster (items, machines) just emits `WorldHit` and the
resolution logic handles priority.

**Stream architecture (changes in bold):**

| Stream tag | Owner              | Direction           | Content                                              |
| ---------- | ------------------ | ------------------- | ---------------------------------------------------- |
| 0          | `network`          | bidirectional       | Welcome, InitialStateDone / Hello, Input (unchanged) |
| 1          | `tiles`            | server -> client    | TilemapData, TileMutated, StreamReady (unchanged)    |
| 2          | `atmospherics`     | server -> client    | GasGridData, GasGridDelta, StreamReady (unchanged)   |
| 3          | `things`           | server -> client    | EntitySpawned, StateUpdate, **ItemEvent**, StreamReady|
| **4**      | **`interactions`** | **client -> server**| **InteractionRequest { TileToggle, ItemPickup, ItemDrop, StoreInContainer, TakeFromContainer }** |

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
| L0    | `input`         | **Add `button` field to `WorldHit`.** `WorldHit { entity, world_pos, button: MouseButton }` so downstream systems can distinguish left-click (default action) from right-click (context menu) without correlating with `PointerAction`. | — |
| L1    | `tiles`         | **Remove stream 4 and tile-toggle request handling.** Delete `TILE_TOGGLE_STREAM_TAG`, `TileToggle` wire type, stream 4 registration, `execute_tile_toggle`, `handle_tile_toggle`, and `TileToggleRequest`. Tile-toggle validation, tilemap mutation, and `TileMutated` broadcast logic moves to `interactions`. `tiles` retains `Tilemap`, `TileKind`, tile mesh spawning, `apply_tile_mutation`, stream 1 (server→client), and the `TileMutated` wire message type (owned by `tiles`, used by `interactions` to broadcast on stream 1). **`raycast_tiles` fires on both buttons** (same change as `raycast_things`), passes `button` through to `WorldHit`. | No schedule changes to remaining systems. |
| L1    | `things`        | **`raycast_things` fires on both buttons.** Currently only processes right-click `PointerAction`; must also fire on left-click so `default_interaction` can pick up items. Passes `button` through to `WorldHit`. **HandSlot component.** New `HandSlot { side: HandSide }` component and `HandSide` enum (`Left`, `Right`). **Hand anchor spawning:** `spawn_player_creature` gains a child entity with `HandSlot { side: Right }` and `Transform::from_translation(HAND_OFFSET)`. **ItemEvent on stream 3:** new enum variants `ItemEvent::PickedUp { item: NetId, holder: NetId }`, `ItemEvent::Dropped { item: NetId, position: [f32; 3] }`, `ItemEvent::Stored { item: NetId, container: NetId }`, and `ItemEvent::Taken { item: NetId, holder: NetId }` added to `ThingsStreamMessage`. Server-side: `broadcast_item_event` sends `ItemEvent` after item mutations. Client-side: `handle_item_event` receives and applies reparenting/deparenting. **Exclude held items from StateUpdate:** items parented to a `HandSlot` should not be included in the 30 Hz position broadcast (they inherit transform from parent). | `broadcast_item_event`: `Update`, after item systems. `handle_item_event`: `Update`, after stream drain, gated on `Client`. |
| L2    | `items`         | **New module.** `Item` marker component (on all item entities). `Container` component with `slots: Vec<Option<Entity>>` and `capacity: usize`. `ItemPickupRequest`, `ItemDropRequest`, `ItemStoreRequest`, and `ItemTakeRequest` Bevy events (fired by `interactions`, read here). **Server systems:** `handle_item_interaction` reads all four request events, validates range + item existence + container space, executes the operation (reparent/deparent, strip/restore physics, update Container slots, set visibility). **No client-side simulation** — clients apply `ItemEvent` messages from stream 3. | `handle_item_interaction`: `Update`, after `dispatch_interaction`, gated on `Server`. |
| L6    | `interactions`  | **Unified interactions stream (stream 4).** Owns `InteractionRequest` wire enum and stream 4 registration (client→server). Client: `send_interaction` serialises requests. Server: `dispatch_interaction` drains stream 4 and handles all interaction types directly — tile toggle (validates, mutates `Tilemap`, broadcasts `TileMutated` on stream 1 as a wire message AND fires it as a local Bevy event for `apply_tile_mutation`), item pickup/drop/store/take (fires corresponding Bevy events for `items` module). **Raycast resolution (fixes #169).** `resolve_world_hits` system collects all `WorldHit` messages per frame, picks closest to camera, emits `ResolvedHit`. **Default action system.** `default_interaction` reads left-click `ResolvedHit`; for items on floor, sends `InteractionRequest::ItemPickup`. **Extended context menu.** Action table gains: `Item` on floor -> ["Pick up"]; `Tile(Floor)` while holding -> ["Drop", "Build Wall"]; `Container` entity while holding -> ["Store in {name}"]; `Container` entity with items while hand empty -> ["Take from {name}"]. | `resolve_world_hits`: `Update`, after `raycast_tiles` and `raycast_things`. `send_interaction`: `Update`, gated on `not(Headless)`. `dispatch_interaction`: `Update`, after stream drain, gated on `Server`. |
| —     | `src/world_setup.rs` | **Item and container spawning.** Register item templates in `ThingRegistry`: can (kind 2), toolbox (kind 3). Spawn two cans and one toolbox in the pressurised chamber. Toolbox entity gets `Container { capacity: 6 }` and spawns pre-loaded with one can inside it (can entity ref in `Container.slots[0]`, `Visibility::Hidden`, `StashedPhysics`, no `RigidBody`/`Collider`/`LinearVelocity`). | `setup_world`: `OnEnter(InGame)`, gated on `Server`. |

### Not in this plan

- **Container UI.** No visual panel for viewing container contents. Store
  and take use context menu actions — no drag-and-drop or inventory grid.
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
      lib.rs                     # Item, Container, StashedPhysics components.
                                 #   ItemPickupRequest, ItemDropRequest,
                                 #   ItemStoreRequest, ItemTakeRequest events.
                                 #   Server: handle_item_interaction, pickup_item,
                                 #   drop_item, store_item, take_item helpers.
                                 #   Container query helpers.
  things/
    src/
      lib.rs                     # MODIFIED — raycast_things fires on both
                                 #   buttons. HandSlot, HandSide components.
                                 #   Hand anchor child entity in
                                 #   spawn_player_creature.
                                 #   ItemEvent variants in ThingsStreamMessage.
                                 #   broadcast_item_event, handle_item_event.
  tiles/
    src/
      lib.rs                     # MODIFIED — remove stream 4 registration,
                                 #   TileToggle wire type, execute_tile_toggle,
                                 #   handle_tile_toggle, TileToggleRequest.
                                 #   raycast_tiles fires on both buttons.
                                 #   Retain Tilemap, TileMutated wire type,
                                 #   tile rendering, stream 1, apply_tile_mutation.
  interactions/
    src/
      lib.rs                     # MODIFIED — owns stream 4 (InteractionRequest
                                 #   wire enum). Client: send_interaction.
                                 #   Server: dispatch_interaction (handles tile
                                 #   toggle directly, fires ItemPickupRequest /
                                 #   ItemDropRequest / ItemStoreRequest /
                                 #   ItemTakeRequest for items module).
                                 #   default_interaction for left-click pickup.
                                 #   resolve_world_hits for raycast priority.
  input/
    src/
      lib.rs                     # MODIFIED — add `button: MouseButton`
                                 #   field to `WorldHit`.
  network/
    src/
      lib.rs                     # NO CHANGES — stream infrastructure sufficient
  atmospherics/
    src/
      lib.rs                     # NO CHANGES — pressure forces already apply
                                 #   to all Dynamic RigidBody entities
src/
  config.rs                      # MODIFIED — add InteractionConfig section
                                 #   to AppConfig with interaction_range field.
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
  `ItemPickupRequest`, `ItemDropRequest`, `ItemStoreRequest`,
  `ItemTakeRequest`, `Item`, `Container` components). Existing dep on
  `tiles` is now load-bearing — `dispatch_interaction` mutates `Tilemap`,
  reads `TileKind`, and sends `TileMutated` on stream 1. Existing dep on
  `network` — for `StreamSender`, `StreamReader`, `StreamDef` to own
  stream 4.
- **`modules/tiles/Cargo.toml`:** Remove `input` dep if no longer needed
  (raycast_tiles still uses `PointerAction` from `input`, so likely kept)
- **`modules/things/Cargo.toml`:** No new deps — `HandSlot` is internal
- **`src/main.rs`:** Add `ItemsPlugin` to both headless and client plugin
  chains (items module has both server and client systems)

### Item entity design

Items are `Thing` entities (spawned via `spawn_thing`) with an additional
`Item` marker component. The `ThingRegistry` template for each item kind
inserts `Item`, a `Name` component (Bevy's built-in `Name` for display
purposes — e.g. `Name::new("Can")`, `Name::new("Toolbox")`), and
kind-specific visual/physical properties. The context menu uses `Name` for
labels like "Store in Toolbox" and "Take from Toolbox".

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

### Client-side container state

The context menu needs to know whether the local player is holding an item
(hand container full/empty) and whether a target entity is a container with
items. This requires `Container` components on both server and client.

**Template-inserted.** The `ThingRegistry` template for kind 3 (toolbox)
inserts `Container { capacity: 6 }` on both server and client. The
`HandSlot` child entity gets `Container { capacity: 1 }` during creature
spawning — on the server via `spawn_player_creature`, on the client via
the kind 0 `ThingRegistry` template (which runs when `EntitySpawned` is
received and the creature is reconstructed).

**Client keeps `Container.slots` in sync.** Each `handle_item_event`
handler updates the local `Container` component:
- `PickedUp`: insert item entity into holder's hand `Container.slots[0]`
- `Dropped`: clear hand `Container.slots[0]`
- `Stored`: clear hand `Container.slots[0]`, insert into target container
- `Taken`: clear source container slot, insert into hand `Container.slots[0]`

This gives the interactions module enough local state to build correct
context menus without querying the server.

### Pickup and drop mechanics

**Physics stashing.** When an item is picked up, its physics components
need to be restored on drop with the correct shape (cylinder for cans,
cuboid for toolboxes). Before removing physics components, the server
inserts a `StashedPhysics { collider: Collider, gravity: GravityScale }`
component that preserves the item's original physics data. On drop,
`StashedPhysics` is read to restore the correct collider and gravity, then
removed. This is simpler than a registry lookup and works for any item.

**Pickup (server-side):**
1. Validate: item entity exists, has `Item` component
2. Validate: requester's creature is within `interaction_range` meters
3. Validate: requester's hand `Container` has an empty slot
4. Stash: insert `StashedPhysics` with clone of current `Collider` and
   `GravityScale`
5. Remove `RigidBody`, `Collider`, `LinearVelocity`, `GravityScale`,
   `ConstantForce` from item entity
6. Set item's `Transform` to local offset (small, near hand)
7. Reparent item entity as child of `HandSlot` entity
8. Insert item entity ref into hand `Container.slots[0]`
9. Broadcast `ItemEvent::PickedUp { item, holder }` on stream 3

**Drop (server-side):**
1. Validate: requester's hand `Container.slots[0]` is `Some(entity)`
2. Validate: requested drop position is within `interaction_range` of creature
3. Remove item from hand `Container.slots[0]`
4. Deparent item from `HandSlot`
5. Set item's `Transform` to the requested drop position (validated in range)
6. Restore physics from `StashedPhysics`: re-insert `RigidBody::Dynamic`,
   stashed `Collider`, stashed `GravityScale`, `LinearVelocity::ZERO`.
   Do **not** re-insert `ConstantForce` (atmospherics handles that).
   Remove `StashedPhysics`.
7. Broadcast `ItemEvent::Dropped { item, position }` on stream 3

**Store (server-side):**
1. Validate: requester's hand `Container.slots[0]` is `Some(item)`
2. Validate: target container entity exists, has `Container`, is within
   `interaction_range`. The target must be on the floor (not the held item
   — storing into a held container requires container UI, deferred)
3. Validate: target container has an empty slot
4. Remove item from hand `Container.slots[0]`
5. Deparent item from `HandSlot`
6. Set `Visibility::Hidden` on item entity
7. Insert item entity ref into target `Container.slots`
8. Broadcast `ItemEvent::Stored { item, container }` on stream 3

**Take (server-side):**
1. Validate: requester's hand `Container.slots[0]` is `None`
2. Validate: target container entity exists, has `Container`, is within
   `interaction_range`
3. Validate: target container has at least one occupied slot
4. Remove first occupied item from target `Container.slots`
5. Set `Visibility::Inherited` on item entity
6. Reparent item as child of `HandSlot`
7. Set item's `Transform` to local hand offset
8. Insert item into hand `Container.slots[0]`
9. Broadcast `ItemEvent::Taken { item, holder }` on stream 3

**Client-side (on receiving ItemEvent):**
- `PickedUp`: Look up item by `NetId`. Look up holder creature by `NetId`,
  then query its children for the entity with `HandSlot` component. Reparent
  item under that `HandSlot` entity, remove physics components, set local
  transform. Update hand `Container.slots[0]`.
- `Dropped`: Look up item by `NetId`, deparent, set world transform,
  re-insert physics components (not `ConstantForce` — atmospherics handles
  that). Clear hand `Container.slots[0]`.
- `Stored`: Look up item by `NetId`, deparent if currently parented (no-op
  during initial sync), set `Visibility::Hidden`, update local `Container`
  slots
- `Taken`: Look up item and holder by `NetId`, reparent under holder's
  `HandSlot`, set `Visibility::Inherited`, set local transform. Update
  source container and hand `Container.slots`.

### Default interaction design

The `interactions` module gains a `default_interaction` system that runs on
left-click `ResolvedHit` events (as opposed to `build_context_menu` which
runs on right-click `ResolvedHit`):

1. Read `ResolvedHit` where `hit.button == Left`
2. Query the hit entity for components: `Item`, `Tile`, etc.
3. If entity has `Item` → send `InteractionRequest::ItemPickup { target }`
   on stream 4
4. Future: other default actions for other entity types

For drop: the context menu action table checks if the player is holding an
item (hand container is non-empty) and the right-click target is a floor
tile. If both conditions hold, "Drop" appears alongside "Build Wall" in the
context menu. Selecting it sends `InteractionRequest::ItemDrop { hand_side,
position }` on stream 4. On the server, `dispatch_interaction` fires
`ItemDropRequest` which the `items` module handles.

For store: if the player is holding an item and right-clicks an entity with
a `Container` component (e.g. a toolbox on the floor), "Store in [name]"
appears in the context menu. Selecting it sends
`InteractionRequest::StoreInContainer { container: NetId }` on stream 4.

For take: if the player's hand is empty and right-clicks an entity with a
non-empty `Container`, "Take from [name]" appears. Selecting it sends
`InteractionRequest::TakeFromContainer { container: NetId }` on stream 4.
The server takes the first occupied slot.

### Replication design

**On connect (initial sync):** Items are replicated as `Thing` entities via
stream 3 (`EntitySpawned`). The `ThingRegistry` template reconstructs them
with correct mesh/collider/Item marker. After all `EntitySpawned` messages
are sent, the server replays the current item-container state:
- `ItemEvent::PickedUp { item, holder }` for each item held in a hand
  (client reparents under HandSlot, removes physics, shows item)
- `ItemEvent::Stored { item, container }` for each item stored inside a
  container (client hides item, records container slot)
This reuses the same client-side `handle_item_event` code path — no
separate sync message needed. Order matters: `EntitySpawned` for all
entities first (so NetIds are resolved), then item events.

**Stored items on spawn.** Items that start inside a container (e.g. the
pre-loaded can in the toolbox) are spawned server-side with
`Visibility::Hidden`, no physics components, and a `StashedPhysics`
component. Their `EntitySpawned` message carries position `Vec3::ZERO`
(irrelevant — they're hidden). The client's `ThingRegistry` template spawns
them normally (visible, with physics), then the subsequent
`ItemEvent::Stored` hides them and strips physics. The brief visible frame
is acceptable — `EntitySpawned` and `ItemEvent::Stored` arrive in the same
stream 3 batch before the client renders.

**Ongoing:** `ItemEvent` messages on stream 3 handle all state changes:
pickup, drop, store, take. `StateUpdate` (position/velocity at 30 Hz)
continues for items on the floor. Held items don't need `StateUpdate` —
they inherit transform from their parent. Stored items don't need
`StateUpdate` — they are hidden and data-only.

### WorldHit resolution design (fixes #169)

Currently `raycast_tiles` and `raycast_things` independently emit `WorldHit`
events. With items on the floor, a single right-click can produce two hits
(the item entity and the floor tile beneath it). The `interactions` module
needs to pick one.

**`ResolvedHit` event.** A new `ResolvedHit` message in the `interactions`
module wraps `WorldHit` after priority resolution. The `resolve_world_hits`
system:

1. Collects all `WorldHit` messages from the current frame (each raycaster
   — tiles, things — may emit one per click)
2. Picks the hit closest to the camera (smallest distance from camera
   `Transform.translation` to `world_pos`)
3. Emits `ResolvedHit { hit: WorldHit }` (button is inside `WorldHit`)

Downstream systems (`build_context_menu`, `default_interaction`) read
`ResolvedHit` instead of `WorldHit`. This is a minimal change — the
interactions module already sits between input and action dispatch.

The `WorldHit` message type stays in `input` (L0) but gains a `button:
MouseButton` field so `ResolvedHit` can carry it. `ResolvedHit` lives in
`interactions` (L6). `raycast_things` (L1) and `raycast_tiles` (L1) both
set `button` from the originating `PointerAction`.

### Interaction range validation

Server-side pickup validation checks Euclidean distance between the
requester's creature `Transform` and the target item's `Transform`. The
range constant `interaction_range` is a field on a new `InteractionConfig`
section in `AppConfig` (the existing configurable resource in
`src/config.rs`) so that any module can reference it. Default value: `2.0`
meters. Line-of-sight is noted as a
future concern but not implemented.

### Testing strategy

Unit tests follow [docs/testing-strategy.md](../testing-strategy.md) and use
Arrange–Act–Assert throughout.

**Items module tests (server-side logic):**
- Pickup validation: in-range succeeds, out-of-range fails, hand-full fails,
  non-item entity fails
- Drop validation: hand-empty fails, drop-position out-of-range fails
- Store validation: hand-empty fails, container-full fails, container
  not-in-range fails
- Take validation: hand-full fails, container-empty fails, container
  not-in-range fails
- Pickup mechanics: physics components removed, item reparented to HandSlot,
  Container slot occupied
- Drop mechanics: physics restored from `StashedPhysics` (correct collider
  shape, no `ConstantForce` — see note below), item deparented, Container
  slot cleared, `StashedPhysics` removed
- Store mechanics: item deparented from HandSlot, hidden, added to target
  Container slot
- Take mechanics: item removed from Container slot, shown, reparented to
  HandSlot
- Nested containers: picking up a toolbox with items inside preserves
  Container data

**Interactions module tests:**
- Raycast resolution: single hit passes through; multiple hits picks closest;
  no hits emits nothing
- Default interaction: left-click on Item entity sends ItemPickup request;
  left-click on non-Item does nothing
- Context menu: item on floor shows "Pick up"; floor tile while holding shows
  "Drop"; container while holding shows "Store in"; container with items
  while hand empty shows "Take from"
- Tile toggle migration: dispatch_interaction handles TileToggle correctly
  (existing behaviour preserved)

**ConstantForce on drop.** When an item is picked up, `ConstantForce` is
removed along with other physics components. On drop, physics components are
restored — but `ConstantForce` should **not** be re-inserted. The
atmospherics system inserts and updates `ConstantForce` each tick for
entities in pressure gradients. A freshly dropped item will receive
`ConstantForce` from atmospherics on the next tick if it's in a gradient
zone. Restoring a stale `ConstantForce` on drop would briefly apply an
incorrect force. Unit tests should verify that drop restores `RigidBody`,
`Collider`, and `LinearVelocity` but not `ConstantForce`.

### Spikes

One spike precedes implementation:

1. **Reparenting spike** — In a minimal Bevy scene, spawn a dynamic rigid
   body, then at runtime: remove `RigidBody`/`Collider`/`LinearVelocity`,
   reparent it as a child of another entity, set a local transform. Verify
   with unit tests:
   (a) the entity moves with its parent (transform propagation)
   (b) re-inserting physics components and deparenting restores normal
   physics behaviour
   (c) no panics or warnings from Avian when components are removed/
   re-inserted
   (d) `Visibility::Hidden` on a reparented entity prevents rendering
   Question to answer: does Avian handle dynamic component removal and
   re-insertion cleanly, or do we need to despawn/respawn?

## Post-mortem

### Outcome

The plan shipped everything it promised across PRs #186–#200 plus several
follow-up commits on the plan branch. Items spawn in the world, can be picked
up (left-click default action or context menu), dropped at a clicked position,
stored into containers, and taken from containers. All interactions are
server-authoritative with range validation. Every client sees the same state.
The toolbox spawns pre-loaded with a can, pickup/drop reparents under the
hand anchor with physics stashing, and pressure forces push loose items through
breaches. All 13 "what done looks like" outcomes are satisfied. One spike
preceded implementation (reparenting + physics removal in Avian) and confirmed
the design before any production code depended on it.

### What shipped beyond the plan

| Addition | Why |
|----------|-----|
| `StoredInContainer(Entity)` component on stored items | O(1 lookup for which container holds an item, avoiding a linear scan of all `Container.slots` when taking or validating. Not in the plan but emerged naturally from the store/take implementation. |
| `ItemEvent` on its own stream (stream 5) instead of stream 3 | The plan specified `ItemEvent` as part of `ThingsStreamMessage` on stream 3. Implementation moved item events to a dedicated stream 5 owned by the `items` module. This keeps the items module self-contained — it registers, writes, and reads its own stream without modifying `things`. Cleaner module boundaries. |
| `TemplatesPlugin` consolidation | Thing template registrations (creature, ball, can, toolbox) were scattered across `world_setup.rs`. PR #1f4f52c consolidated them into a shared `TemplatesPlugin` so templates are registered once and reused by both server and client. Not planned but reduced duplication. |
| `ItemsConfig` instead of `InteractionConfig` | The plan called the config section `InteractionConfig`. The implementation named it `ItemsConfig` under `[items]` in config.toml, which better reflects that the range is item-specific. The interactions module reads it via the `InteractionRange` resource inserted by items. |
| `broadcast_held_on_join` and `broadcast_stored_on_join` | The plan described initial sync via replaying `ItemEvent` messages after `EntitySpawned`, but didn't name dedicated systems. Two explicit systems handle catch-up: one for held items, one for stored items. Clearer than a single monolithic sync function. |
| `SystemSet` adoption and module coordination doc (PR #200) | The plan didn't call for formalising system ordering beyond the layer participation table. PR #200 introduced `SystemSet` enums per module and wrote `docs/module-coordination.md` as a standing reference. This was prompted by ordering bugs during integration. |
| Split into separate client and server binaries (PR #192) | The plan assumed a single binary with headless mode. PR #192 split into `geostationary-client` and `geostationary-server` crates, clarifying which plugins load where. Unplanned but reduced confusion about which systems run in which context. |

### Deviations from plan

- **`ItemEvent` lives in `items` module (stream 5), not `things` module
  (stream 3).** The plan placed `ItemEvent` variants inside
  `ThingsStreamMessage` on stream 3 with `broadcast_item_event` and
  `handle_item_event` in the things module. The implementation correctly
  recognised that items should own their own stream and event types. This
  is a better module boundary — things doesn't need to know about item
  semantics.

- **`ItemsConfig` instead of `InteractionConfig`.** The plan specified an
  `InteractionConfig` section in `AppConfig` with `interaction_range`. The
  implementation uses `ItemsConfig` under `[items]`, and the items module
  inserts an `InteractionRange` resource. The interactions module reads
  `InteractionRange` rather than owning the config. This is correct — the
  range is an items-layer concern, not an interactions-layer concern.

- **Template-owned physics and meshes.** The plan described item spawning
  in `world_setup.rs` with inline component insertion. The implementation
  moved mesh, collider, and physics setup into `ThingRegistry` templates
  so that both server and client spawn items with the same properties. The
  client uses template-inserted meshes from `EntitySpawned` handling
  rather than separate client-side spawn logic.

- **No `ConstantForce` removal in pickup.** The plan listed `ConstantForce`
  as one of the physics components removed during pickup. The
  implementation correctly omits it — `ConstantForce` is managed by
  atmospherics and may not be present on all items. The stash only
  preserves `Collider` and `GravityScale`, which are the item-owned
  physics data.

### Hurdles

1. **Item drop position race with physics.** Dropped items appeared at the
   hand offset instead of the clicked world position. The `Transform` was
   set correctly, but deparenting from `HandSlot` and restoring
   `RigidBody::Dynamic` caused Avian to read the stale parent-relative
   transform before the world-space position propagated. Fixed by ensuring
   the transform is set after deparenting and physics restoration happens
   in the correct order. **Lesson:** When deparenting and restoring physics
   in the same frame, set the world-space transform after removing the
   parent relationship, not before.

2. **Headless server missing physics plugin.** The server binary didn't
   include the physics plugin for items, so `RigidBody` and `Collider`
   components weren't being processed. Items existed but had no physics
   simulation. Fixed by ensuring `PhysicsPlugin` is included in the
   headless server plugin chain. **Lesson:** When adding physics-dependent
   entities, verify the headless server includes the physics plugin — it's
   easy to assume "server doesn't render" means "server doesn't need
   physics."

3. **System ordering between `dispatch_interaction` and item handlers.**
   `dispatch_interaction` fires `ItemPickupRequest` as a Bevy message, but
   `handle_item_interaction` needs to read it in the same frame. Without
   explicit ordering, the handler sometimes ran before the dispatcher. PR
   #200 introduced `SystemSet` enums to enforce ordering. **Lesson:**
   When a system fires a message that another system reads in the same
   frame, explicit set ordering is mandatory — don't rely on Bevy's
   default scheduling.

4. **Camera query safety in `resolve_world_hits`.** The system queried
   `Camera` with `.single()` which panics if there's no camera (e.g.,
   during headless server startup). Changed to use `let Ok(cam) =
   query.single() else { return; }`. **Lesson:** Always use fallible
   queries for components that may not exist in all app configurations
   (headless vs client).

5. **Context menu range gating.** The initial context menu showed actions
   for items at any distance. Added range checking using
   `InteractionRange` so the menu only shows "Pick up" when the player is
   within range. This was in the plan (server validates range) but the
   client-side UX of not showing impossible actions was an oversight.
   **Lesson:** Server-side validation is necessary but not sufficient —
   the UI should also reflect what's possible to avoid confusing
   request-then-reject interactions.

### What went well

- **The reparenting spike paid off immediately.** It confirmed that Avian
  handles dynamic `RigidBody`/`Collider` removal and re-insertion cleanly
  with no panics or warnings. Without the spike, the physics stashing
  pattern would have been a leap of faith.

- **Bottom-up sequencing continued to work.** PRs landed in dependency
  order: spike → hand components → WorldHit button field → items module →
  item event replication → item spawning → unified interactions stream →
  raycast resolution + context menu → system ordering. No PR needed
  rework due to missing infrastructure.

- **The "hand is a container" abstraction held.** All four operations
  (pickup, drop, store, take) are the same primitive: move item between
  containers. The `Container` component with `slots: Vec<Option<Entity>>`
  handled hand (capacity 1) and toolbox (capacity 6) uniformly. No
  special-case code for "held vs stored."

- **Stream-per-module isolation.** Moving `ItemEvent` to its own stream
  (stream 5) instead of piggybacking on things' stream 3 was a
  deviation from the plan but a clear improvement. The items module is
  fully self-contained — it registers, serialises, and deserialises its
  own events without any changes to the things module.

- **Previous post-mortem recommendations were followed.** The break-a-wall
  post-mortem said: "spike the actual API" and "verify force directions
  with a unit test." The reparenting spike tested the concrete Avian API,
  and the items module includes directional tests for physics restoration.

- **Config-driven range worked out of the box.** The `interaction_range`
  field in `config.toml` was used for both server validation and
  client-side menu gating without any additional plumbing.

### What to do differently next time

- **Plan for stream ownership per module, not shared streams.** The plan
  placed `ItemEvent` on things' stream 3. The implementation correctly
  gave items its own stream 5. Future plans should default to one stream
  per module — sharing streams couples modules at the wire protocol level.

- **Include client-side UX validation in the plan, not just server-side.**
  The plan thoroughly specified server-side range validation but didn't
  mention client-side range gating for the context menu. Future plans
  should address both "what does the server reject?" and "what does the
  client not show?"

- **Spike deparenting + transform ordering, not just reparenting.** The
  reparenting spike confirmed pickup works, but the drop-position bug
  (hurdle 1) was a deparenting ordering issue. Future spikes should test
  the reverse operation too.

- **Plan for binary separation early.** The split into client/server
  binaries (PR #192) was unplanned and touched many files. Future plans
  that add server-only or client-only systems should consider whether a
  binary split is warranted and plan for it explicitly.

- **Add system ordering to the spike checklist.** Two of the five hurdles
  (system ordering, camera query safety) were scheduling issues. Future
  plans should include a "scheduling spike" or at minimum a concrete
  `SystemSet` design in the layer participation table, not just prose
  ordering descriptions.
