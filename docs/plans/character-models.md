# Plan: Character Models & Animation

> **Stage goal:** Player creatures are rigged GLTF models with idle, walk,
> and hold-item animation states driven by an animation state machine. The
> orange capsule placeholder is replaced. Animation state is server-computed
> and replicated so all clients see the correct animation on every character.
> The GLTF loading pattern established here becomes the standard for all
> future 3D assets. This is plan 2 of the Tangible Station arc.

## What "done" looks like

1. Creatures render as a rigged GLTF model instead of an orange capsule
2. The model plays an idle animation when the creature is stationary
3. The model plays a walk animation when the creature is moving
4. The model plays a hold animation when the creature has an item in hand
5. Transitions between animation states are smooth (crossfade blending)
6. All clients see the correct animation state on every creature, including
   late-joining clients
7. The server does not load GLTF scene assets or create `AnimationPlayer`
   components — it tracks animation state as a lightweight enum
8. The creature's `HandSlot` child entity is attached to the model's hand
   bone so held items follow the hand during animations
9. The placeholder GLTF model and its animations exist in `assets/models/`
10. Items, balls, and toolboxes continue to use primitive meshes (unchanged)

## Strategy

Work bottom-up: first establish the animation module at L0 with a
game-agnostic animation state machine, then integrate GLTF loading into the
creature visual builder, then wire animation state computation and
replication.

Two spikes run first to validate runtime assumptions about Bevy 0.18's GLTF
scene hierarchy and animation graph API — specifically how to find bone
entities by name and how to drive `AnimationPlayer` programmatically through
an `AnimationGraph`.

**Lessons from map-authoring post-mortem:**
- Spike runtime assumptions before committing (Hurdle 1, 3)
- Test loading state in isolation (Hurdle 4) — validate GLTF scene readiness
  before driving animation
- Decide design choices definitively before coding (camera deviation)
- Scope "not in this plan" to distinguish "cannot do yet" from "choose not
  to do yet"

### Layer participation

| Layer | Module | Plan scope |
|-------|--------|------------|
| L0 | **`animation` (new)** | `AnimState` enum (Idle, Walk, Hold), `AnimationController` component (maps AnimState to animation clips), system that drives `AnimationPlayer` transitions based on `AnimState` changes. Game-agnostic — knows about states and transitions, not creatures or items. |
| L1 | `things` | Extend `EntityState` to include an `anim_state: u8` field. Extend `ThingsStreamMessage::EntitySpawned` to include initial animation state. Broadcast and apply animation state in `StateUpdate`. |
| L3 | `creatures` | New `compute_anim_state` system: reads `LinearVelocity` and hand contents (`Children` + `HandSlot` + `Container`) to derive `AnimState`, writes it to the entity. Runs on server and client (server is authoritative, client is for local prediction). |
| -- | `bins/shared/src/templates.rs` | Creature visual builder changes from `Mesh3d` + `MeshMaterial3d` to `SceneRoot` loaded from GLTF. Inserts `AnimState::Idle` and `AnimationController` with clip handles. Functional builder unchanged except `HandSlot` spawn deferred to a scene-ready hook. |

### Not in this plan

- **Character customisation** (clothing, hair, skin colour) — all creatures
  use the same model. Cannot do yet: requires a customisation system and
  multiple model variants.
- **Item-specific hold animations** — single generic hold pose. Choose not
  to do yet: pose variety is low value without item model variety.
- **Facial animation or blend shapes** — not needed at this stage.
- **Animation events** (footstep sounds on walk frames) — deferred to the
  Sound & Ambience plan (plan 4).
- **Item GLTF models** — items remain primitive meshes. Choose not to do
  yet: item models are independent of the character animation pipeline.
- **Client-side animation prediction** — clients apply server-broadcast
  `AnimState`. Local prediction of animation state (computing it from local
  velocity before server confirms) is a polish optimisation, not essential.
  Choose not to do yet.
- **Root motion** — animation does not drive entity translation. Movement
  remains velocity-based.

### Module placement

```
assets/models/
  creature.glb               # Placeholder rigged model with idle/walk/hold clips

modules/animation/            # NEW — L0
  Cargo.toml
  src/
    lib.rs                    # AnimationPlugin, AnimState, AnimationController,
                              # drive_animation system

modules/creatures/src/lib.rs  # compute_anim_state system added

modules/things/src/lib.rs     # EntityState gains anim_state field
modules/network/src/protocol.rs  # EntityState wire format updated

bins/shared/src/templates.rs  # Creature visual builder rewritten for GLTF
```

### Animation module design (L0)

The `animation` module is game-agnostic. It provides:

- **`AnimState`** — a `#[derive(Component)]` enum: `Idle`, `Walk`, `Hold`.
  Serializes to `u8` for wire efficiency. This enum is extensible — future
  plans add variants (e.g., `UseItem`, `Stagger`) without structural changes.

- **`AnimationController`** — a component that maps each `AnimState` variant
  to a node index in a Bevy `AnimationGraph`. Stores the graph handle and
  the clip-to-node mapping. Created at scene spawn time from the GLTF's
  named animations.

- **`drive_animation` system** — runs in `PostUpdate`. When `AnimState`
  changes (detected via `Changed<AnimState>`), it looks up the corresponding
  graph node in `AnimationController` and initiates a crossfade transition on
  the entity's `AnimationPlayer`. The system finds the `AnimationPlayer` by
  walking the entity's descendants (GLTF scenes nest it on a child).

The module depends only on `bevy`. It does not depend on `creatures`,
`things`, or `network`.

### GLTF loading and scene spawn

The creature visual builder in `templates.rs` changes from:

```
Mesh3d(capsule) + MeshMaterial3d(orange)
```

to:

```
SceneRoot(asset_server.load("models/creature.glb#Scene0"))
```

**Scene readiness problem:** When a GLTF scene spawns, the child entities
(skeleton bones, `AnimationPlayer`, mesh nodes) are not available
immediately — they arrive after Bevy's scene spawning system runs. Any
system that needs to find the `AnimationPlayer` or a bone entity must wait.

**Solution:** Use a `SceneInstanceReady` trigger or poll for
`AnimationPlayer` in descendants via a one-shot initialisation system that
runs each frame until the player is found, then inserts a `SceneReady`
marker. The spike will determine which approach is more reliable.

### HandSlot bone attachment

Currently `HandSlot` spawns as a direct child of the creature entity at a
fixed offset. With a GLTF model, it needs to follow the hand bone.

**Approach:** After the GLTF scene is ready (scene readiness confirmed),
find the bone entity with `Name("hand.R")` (or whatever the placeholder
model names it) in the creature's descendants. Reparent the `HandSlot`
entity to be a child of that bone entity with a local transform offset of
`Vec3::ZERO` (or a small adjustment if the bone's origin is not at the
grip point).

This replaces the hardcoded `HAND_OFFSET` constant. The constant can be
removed or kept as fallback for non-GLTF creatures.

### Animation state replication

Animation state rides on the existing stream 3 `StateUpdate` messages
alongside position and velocity. This avoids ordering issues between
separate streams and requires minimal wire overhead (1 byte per entity per
tick).

**Wire format change:** `EntityState` gains an `anim_state: u8` field.
The `u8` encoding is: 0 = Idle, 1 = Walk, 2 = Hold. Unknown values fall
back to Idle.

**Server-side:** The `compute_anim_state` system in `creatures` runs in
`Update`. It reads `LinearVelocity` magnitude and checks whether the
creature's hand `Container` holds an item. It writes `AnimState` to the
creature entity. The `broadcast_state` system in `things` reads `AnimState`
and includes it in `EntityState`.

**Client-side:** `handle_entity_lifecycle` reads `anim_state` from
`StateUpdate` and writes it to the local entity's `AnimState` component.
The `drive_animation` system in the animation module reacts to the change.

**Late joiners:** `EntitySpawned` includes initial `anim_state` so clients
joining mid-round see the correct animation from the first frame.

### Headless server considerations

The server inserts `Headless` and does not have a renderer, but it does have
`AssetPlugin`, `MeshPlugin`, and `ScenePlugin`. The creature visual builder
currently runs on the server (visual builders are called unconditionally in
`on_spawn_thing`). For primitive meshes this is harmless — handles are
created but never rendered.

For GLTF scenes, running the visual builder on the server would trigger
asset loading of the `.glb` file and scene instantiation, which is wasteful.
The visual builder already exists as a separate callback from the functional
builder. The fix is to skip visual builder execution when `Headless` is
present. This is a one-line change in `on_spawn_thing`: gate the
`visual_builders` call on `!world.contains_resource::<Headless>()`.

The server still needs `AnimState` (for replication) and `AnimationController`
is not needed (no `AnimationPlayer` to drive). The `compute_anim_state`
system reads only velocity and hand contents — no rendering components.

## Spike 1: GLTF scene hierarchy and AnimationPlayer access (30 min)

**Questions:**

1. After spawning a `SceneRoot` from a `.glb`, what does the entity
   hierarchy look like? Where is `AnimationPlayer` in the tree?
2. Can we find a named bone entity (e.g., `Name("hand.R")`) by walking
   descendants?
3. How does `AnimationGraph` work in Bevy 0.18? Can we create a graph with
   three clip nodes (idle, walk, hold), insert it as an asset, and switch
   between them by setting the active node on `AnimationPlayer`?
4. How does `SceneInstanceReady` or `SceneInstance` work for detecting when
   the scene's children are fully spawned?

**Method:** Create a minimal Bevy app that loads a test `.glb` file (a
simple rigged mesh with 2-3 animations exported from Blender), spawns it,
and logs the entity hierarchy. Attempt to find `AnimationPlayer`, find a
bone by name, create an `AnimationGraph`, and drive transitions.

**Blocking:** All implementation tasks. The `AnimationController` design and
scene-ready approach depend on these answers.

## Spike 2: Bone reparenting for HandSlot (30 min)

**Questions:**

1. Can we reparent an existing entity (spawned as a child of the creature
   root) to become a child of a bone entity inside the GLTF scene hierarchy?
2. Does the reparented entity inherit the bone's world transform correctly
   during animation playback?
3. Does reparenting interfere with Bevy's scene management or cause the
   entity to be despawned on scene reload?

**Method:** Using the same test `.glb` from Spike 1, spawn a child entity
on the creature root, then after scene readiness, reparent it to a bone.
Verify that the child follows the bone through an animation cycle by
logging world transforms each frame.

**Blocking:** HandSlot bone attachment implementation. If reparenting does
not work, the fallback is to spawn `HandSlot` directly on the bone entity
during scene-ready initialisation instead of reparenting.

## Post-mortem

*To be filled in after the plan ships.*
