# Plan: Character Models & Animation

> **Stage goal:** Player creatures are rigged GLTF models with idle and walk
> animation states driven by an animation state machine, and single-arm IK
> that positions the hand when holding an item. The orange capsule placeholder
> is replaced. Animation and hold state are server-computed and replicated so
> all clients see the correct pose on every character. The GLTF loading
> pattern established here becomes the standard for all future 3D assets.
> This is plan 2 of the Tangible Station arc.

## What "done" looks like

1. Creatures render as a rigged GLTF model instead of an orange capsule
2. The model plays an idle animation when the creature is stationary
3. The model plays a walk animation when the creature is moving
4. When a creature holds an item, single-arm IK positions the right hand
   at a hold target in front of the torso — the walk/idle animation
   continues playing on the rest of the body
5. Transitions between idle and walk are smooth (crossfade blending)
6. All clients see the correct animation state and hold pose on every
   creature, including late-joining clients
7. The server does not load GLTF scene assets or create `AnimationPlayer`
   components — it tracks animation state and hold flag as lightweight data
8. The creature's `HandSlot` child entity is attached to the model's hand
   bone so held items follow the hand through both animation and IK
9. The placeholder GLTF model and its animations exist in `assets/models/`
10. Items, balls, and toolboxes continue to use primitive meshes (unchanged)

## Strategy

Work bottom-up: first establish the animation module at L0 with a
game-agnostic animation state machine and single-arm IK solver, then
integrate GLTF loading into the creature visual builder, then wire state
computation and replication.

Holding items uses IK instead of a dedicated hold animation clip. This is
more durable: walk/idle animations play uninterrupted on the legs and torso
while IK overrides only the arm chain (upper_arm → forearm → hand) to
position the hand at a hold target. Future plans can vary hold positions per
item type by changing the IK target, without creating new animation clips.

Three spikes run first to validate runtime assumptions about Bevy 0.18's
GLTF scene hierarchy, bone reparenting, and IK solver feasibility.

**Lessons from map-authoring post-mortem:**

- Spike runtime assumptions before committing (Hurdle 1, 3)
- Test loading state in isolation (Hurdle 4) — validate GLTF scene readiness
  before driving animation
- Decide design choices definitively before coding (camera deviation)
- Scope "not in this plan" to distinguish "cannot do yet" from "choose not
  to do yet"

### Layer participation

| Layer | Module                         | Plan scope                                                                                                                                                                                                                                                                                                                                                                   |
| ----- | ------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| L0    | **`animation` (new)**          | `AnimState` enum (Idle, Walk), `AnimationController` (maps AnimState to graph nodes), `drive_animation` system. `IkChain` component and `solve_ik` system for single-arm two-bone IK. `HoldIk` component marks entities whose arm should IK to a hold target when holding an item. Game-agnostic — knows about states, transitions, and bone chains, not creatures or items. |
| L0    | `network`                      | Extend `EntityState` with `anim_state: u8` and `holding: bool` fields in the wire format. Keep the replication payload domain-neutral: animation state enum value plus holding flag.                                                                                                                                                                                         |
| L1    | `things`                       | Extend `ThingsStreamMessage::EntitySpawned` with `anim_state: u8` and `holding: bool`. Broadcast both fields in `StateUpdate` and apply them on the client during entity lifecycle handling.                                                                                                                                                                                 |
| L3    | `creatures`                    | New `compute_anim_state` system: reads `LinearVelocity` to derive `AnimState` (Idle or Walk). New `compute_hold_state` system: reads hand `Container` contents to set `HoldIk::active`. Both run on server.                                                                                                                                                                  |
| --    | `bins/shared/src/templates.rs` | Creature visual builder changes from `Mesh3d` + `MeshMaterial3d` to `SceneRoot` loaded from GLTF. Functional builder inserts `AnimState::Idle`, `HoldIk`.                                                                                                                                                                                                                    |

### Not in this plan

- **Character customisation** (clothing, hair, skin colour) — all creatures
  use the same model. Cannot do yet: requires a customisation system and
  multiple model variants.
- **Item-specific hold positions** — single generic hold target in front of
  the torso. Choose not to do yet: per-item IK targets are a data change,
  not an architectural one, once the IK solver exists.
- **Multi-bone IK chains** (spine look-at, foot placement) — only single-arm
  IK for holding. Choose not to do yet: the `IkChain` component supports
  arbitrary chains but only the arm is wired up.
- **Facial animation or blend shapes** — not needed at this stage.
- **Animation events** (footstep sounds on walk frames) — deferred to the
  Sound & Ambience plan (plan 4).
- **Item GLTF models** — items remain primitive meshes. Choose not to do
  yet: item models are independent of the character animation pipeline.
- **Client-side animation prediction** — clients apply server-broadcast
  state. Choose not to do yet.
- **Root motion** — animation does not drive entity translation. Movement
  remains velocity-based.
- **Full IK framework** (multiple simultaneous chains, priority blending,
  constraints) — this plan implements a minimal two-bone IK solver for one
  arm. A general-purpose IK framework is future work.

### Module placement

```
assets/models/
  creature.glb               # Placeholder rigged model with idle/walk clips

modules/animation/            # NEW — L0
  Cargo.toml
  src/
    lib.rs                    # AnimationPlugin, AnimState, AnimationController,
                              # drive_animation, IkChain, HoldIk, solve_ik

modules/creatures/src/lib.rs  # compute_anim_state, compute_hold_state

modules/network/src/protocol.rs  # EntityState wire format gains anim_state + holding
modules/things/src/lib.rs     # EntitySpawned, StateUpdate broadcast/apply updated

bins/shared/src/templates.rs  # Creature visual builder rewritten for GLTF
```

### Animation module design (L0)

The `animation` module is game-agnostic. It provides:

**Clip-driven animation:**

- **`AnimState`** — a `#[derive(Component)]` enum: `Idle`, `Walk`.
  Serializes to `u8` for wire efficiency. Extensible — future plans add
  variants (e.g., `UseItem`, `Stagger`) without structural changes.

- **`AnimationController`** — a component that maps each `AnimState` variant
  to a node index in a Bevy `AnimationGraph`. Stores the graph handle and
  the clip-to-node mapping. Created at scene spawn time from the GLTF's
  named animations.

- **`drive_animation` system** — runs in `PostUpdate`. When `AnimState`
  changes (detected via `Changed<AnimState>`), it looks up the corresponding
  graph node in `AnimationController` and initiates a crossfade transition on
  the entity's `AnimationPlayer`. The system finds the `AnimationPlayer` by
  walking the entity's descendants (GLTF scenes nest it on a child).

**Single-arm IK:**

- **`IkChain`** — a component storing references to three bone entities
  (root, mid, tip) forming a two-bone IK chain. Also stores the chain's
  total length for reach validation. Populated during scene-ready
  initialisation by finding bones by name.

- **`HoldIk`** — a component on creature entities that stores: `active: bool`
  (whether to apply IK), `target: Vec3` (local-space hold position relative
  to creature root, e.g., `Vec3::new(0.3, 0.7, -0.3)` — in front of torso).

- **`solve_ik` system** — runs in `PostUpdate`, after `drive_animation` and
  before Bevy's `TransformPropagate`. When `HoldIk::active` is true, reads
  the IK target in world space, solves the two-bone IK for the arm chain,
  and writes the resulting rotations to the upper_arm and forearm bone
  entities. This overrides the animation clip's arm pose while leaving the
  rest of the body untouched.

The two-bone IK solver is a standard geometric solution: given shoulder
position, elbow-to-hand length, shoulder-to-elbow length, and target
position, compute the two joint angles. The pole vector (elbow direction)
defaults to pointing backward-down to produce a natural arm bend.

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

**Scene-ready initialisation** also populates the `IkChain` component by
finding the arm bone entities (upper_arm.R, forearm.R, hand.R) by name.

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
removed or kept as fallback for non-GLTF creatures. When IK is active, the
hand bone moves to the hold target and HandSlot (as a child) follows
automatically.

### State replication

Animation state and hold flag ride on the existing stream 3 `StateUpdate`
messages alongside position and velocity. This avoids ordering issues
between separate streams and requires minimal wire overhead.

**Wire format change:** `EntityState` gains `anim_state: u8` and
`holding: bool` fields. The `u8` encoding is: 0 = Idle, 1 = Walk. Unknown
values fall back to Idle. `holding` maps directly to `HoldIk::active`.

**Server-side:** `compute_anim_state` in `creatures` derives `AnimState`
from `LinearVelocity` magnitude. `compute_hold_state` derives
`HoldIk::active` from hand `Container` contents. Both run in `Update`.
The `broadcast_state` system in `things` reads both and includes them in
`EntityState`.

**Client-side:** `handle_entity_lifecycle` reads `anim_state` and `holding`
from `StateUpdate` and writes them to the local entity's `AnimState` and
`HoldIk` components. `drive_animation` and `solve_ik` react to the changes.

**Late joiners:** `EntitySpawned` includes initial `anim_state` and
`holding` so clients joining mid-round see the correct pose from the first
frame.

### Headless server considerations

The server inserts `Headless` and does not have a renderer. The fix is to
skip visual builder execution when `Headless` is present — gate the
`visual_builders` call in `on_spawn_thing` on
`!world.contains_resource::<Headless>()`.

The server still needs `AnimState` and `HoldIk` for replication, but it
does not create `AnimationController`, `IkChain`, GLTF scene entities, or
`AnimationPlayer` components. That keeps the `animation` module Bevy-only:
`drive_animation` and `solve_ik` can be registered without a `Headless`
dependency because their queries only match fully visualized client-side
entities. On the headless server they are inert by construction.

## Spike 1: GLTF scene hierarchy and AnimationPlayer access (30 min)

**Questions:**

1. After spawning a `SceneRoot` from a `.glb`, what does the entity
   hierarchy look like? Where is `AnimationPlayer` in the tree?
2. Can we find a named bone entity (e.g., `Name("hand.R")`) by walking
   descendants?
3. How does `AnimationGraph` work in Bevy 0.18? Can we create a graph with
   two clip nodes (idle, walk), insert it as an asset, and switch between
   them by setting the active node on `AnimationPlayer`?
4. How does `SceneInstanceReady` or `SceneInstance` work for detecting when
   the scene's children are fully spawned?

**Method:** Create a minimal Bevy app that loads a test `.glb` file (a
simple rigged mesh with 2+ animations exported from Blender), spawns it,
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

## Spike 3: Two-bone IK on animated skeleton (30 min)

**Questions:**

1. Can we write to bone `Transform` components after `AnimationPlayer` has
   applied the clip pose, without the animation overwriting the IK result
   next frame? What system ordering is required?
2. Does a standard two-bone IK geometric solve produce correct joint
   rotations when the bones have non-identity rest poses (as exported from
   Blender)?
3. Can IK and clip animation coexist on the same skeleton — e.g., walk
   animation plays on legs while IK overrides the arm — without visual
   artefacts or transform conflicts?

**Method:** Using the same test `.glb` from Spike 1, play the walk
animation and simultaneously apply a two-bone IK solve on the right arm
chain to a fixed world-space target. Visually verify that the legs walk
while the arm reaches the target. Log bone rotations to confirm IK output
persists across frames.

**Blocking:** IK implementation in the animation module. If IK conflicts
with AnimationPlayer (e.g., animation overwrites IK every frame with no
viable ordering), the fallback is masking specific bone tracks from the
animation clip or using an additive animation layer.

## Post-mortem

_To be filled in after the plan ships._
