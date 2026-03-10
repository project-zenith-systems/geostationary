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
| L1    | `things`                       | Extend `ThingsStreamMessage::EntitySpawned` with `anim_state: u8` and `holding: bool`. Broadcast both fields in `StateUpdate` and apply them on the client during entity lifecycle handling. New dependency on `animation` (L0) for `AnimState` and `HoldIk` types.                                                                                                          |
| L3    | `creatures`                    | New `compute_anim_state` system: reads `LinearVelocity` to derive `AnimState` (Idle or Walk). New `compute_hold_state` system: reads hand `Container` contents to set `HoldIk::active`. Both run on server. New dependency on `items` (L2) for `Container` type.                                                                                                             |
| —     | `bins/shared/src/templates.rs` | Creature visual builder changes from `Mesh3d` + `MeshMaterial3d` to `SceneRoot` loaded from GLTF. Functional builder inserts `AnimState::Idle`, `HoldIk`.                                                                                                                                                                                                                    |

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

modules/items/src/lib.rs      # find_hand_slot_with_space, find_hand_slot_containing
                              # updated to descendant traversal post-reparenting
modules/network/src/protocol.rs  # EntityState wire format gains anim_state + holding
modules/things/src/lib.rs     # EntitySpawned, StateUpdate broadcast/apply updated

bins/shared/src/templates.rs  # Creature visual builder rewritten for GLTF
```

### Animation module design (L0)

The `animation` module is game-agnostic. It provides two capabilities:

**Clip-driven animation:** `AnimState` enum (`Idle`, `Walk`) serializes to
`u8` for wire efficiency. `AnimationController` maps each variant to a node
in a Bevy `AnimationGraph` — populated at scene-ready time (not spawn time,
since the GLTF may not be loaded yet). `drive_animation` runs in
`PostUpdate`, reacting to `Changed<AnimState>` or
`Added<AnimationController>` to drive crossfade transitions on the entity's
`AnimationPlayer`. The `Added` trigger ensures the initial idle animation
starts as soon as the scene is ready, avoiding a T-pose.

**Single-arm IK:** `IkChain` stores three bone entity references (root, mid,
tip) for a two-bone chain, populated at scene-ready time. `HoldIk` stores
`active: bool` and a local-space `target: Vec3`. `solve_ik` runs in
`PostUpdate` after `drive_animation` and before `TransformPropagate` —
when active, it solves two-bone IK for the arm and writes rotations to the
upper_arm and forearm bones, overriding the clip pose on those bones only.
Standard geometric solver with a backward-down pole vector for natural arm
bend.

The module depends only on `bevy` (with `bevy_animation` feature). It does
not depend on `creatures`, `things`, or `network`.

### Dependencies

The workspace `Cargo.toml` uses `default-features = false` for Bevy. This
plan requires adding `bevy_animation` to the feature list (for
`AnimationGraph`, `AnimationPlayer`, `AnimationNodeIndex`). `bevy_gltf` may
also be needed — the spike will confirm.

### GLTF loading and scene readiness

The creature visual builder changes from `Mesh3d` + `MeshMaterial3d` to
`SceneRoot(asset_server.load("models/creature.glb#Scene0"))`. GLTF scene
children (bones, `AnimationPlayer`, mesh nodes) are not available
immediately — a scene-ready detection mechanism (spike will determine:
`SceneInstanceReady` trigger vs polling) inserts a `SceneReady` marker.
Scene-ready initialisation then populates `AnimationController` (graph node
mapping from the GLTF's named clips) and `IkChain` (arm bone references by
name). Both require the GLTF to be fully loaded.

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

**Items module impact:** The existing `find_hand_slot_with_space` and
`find_hand_slot_containing` functions in `modules/items/src/lib.rs` use
single-level `Children` traversal to locate `HandSlot`. After reparenting,
`HandSlot` is no longer a direct child of the creature root — these
functions must be updated to use descendant traversal so item pickup, drop,
and hold interactions continue to work.

### State replication

Animation state and hold flag piggyback on stream 3 `StateUpdate` messages.
`EntityState` gains `anim_state: u8` (0 = Idle, 1 = Walk) and
`holding: bool`. `EntitySpawned` gains both fields for late joiners.

**Server:** `compute_anim_state` derives `AnimState` from velocity;
`compute_hold_state` derives `HoldIk::active` from hand `Container`
contents (using descendant traversal to find `HandSlot` post-reparenting).
`broadcast_state` reads both. `LastBroadcast` must also track `anim_state`
and `holding` so stationary creatures that change state still trigger a
broadcast.

**Client:** `handle_entity_lifecycle` writes both fields to the local
entity's `AnimState` and `HoldIk`. `drive_animation` and `solve_ik` react.

### Headless server considerations

The server needs `AnimState` and `HoldIk` for replication but must not load
GLTF assets or create visual components. Two changes achieve this:
(1) skip visual builder *registration* in `TemplatesPlugin::build()` when
`Headless` is present (prevents `asset_server.load()` entirely), and
(2) gate visual builder *execution* in `on_spawn_thing` as defence-in-depth.
`drive_animation` and `solve_ik` are inert on headless by construction —
their queries require `AnimationController`/`IkChain` components that only
exist on client-side entities.


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

**Status: Complete.**

### Spike 1 answers

**Q1 — Entity hierarchy and AnimationPlayer location:**
Spawning `SceneRoot(asset_server.load("model.glb#Scene0"))` produces a
hierarchy mirroring the GLTF node tree. The `SceneRoot` entity sits at the
top. Beneath it, Bevy creates one entity per GLTF node with `Name`,
`Transform`, and `GlobalTransform`. `AnimationPlayer` is placed on a
**descendant** entity (the skeleton root), **not** on the `SceneRoot`
entity. To find it, walk `Children::iter_descendants(scene_root)` and query
for `AnimationPlayer`. Confirmed by the official Bevy 0.18
`animated_mesh.rs` example.

**Q2 — Named bone entities:**
Yes. Every GLTF node becomes an entity with a `Name` component. Bone nodes
are included. Walking `Children::iter_descendants(scene_root)` and matching
`Name("hand.R")` finds the bone entity. The plan's descendant-walk approach
is valid.

**Q3 — AnimationGraph in Bevy 0.18:**
`AnimationGraph` is an `Asset`. Construction: `AnimationGraph::from_clips(
[handle_idle, handle_walk])` returns `(AnimationGraph, Vec<AnimationNodeIndex>)`.
Store the graph as an asset (`graphs.add(graph)`) and insert
`AnimationGraphHandle(handle)` on the entity with `AnimationPlayer`. Use
`AnimationTransitions` for crossfade: `transitions.play(&mut player,
walk_index, Duration::from_millis(250)).repeat()`. Transitions manage blend
weights internally. Clip handles are obtained from
`GltfAssetLabel::Animation(n).from_asset("model.glb")`.

**Q4 — Scene readiness detection:**
Bevy 0.18 fires `SceneInstanceReady` as a **trigger** on the `SceneRoot`
entity. The recommended pattern is an **observer**:
`commands.spawn(SceneRoot(handle)).observe(on_scene_ready)`. The alternative
(`Added<AnimationPlayer>` polling in Update) also works but the observer is
more targeted. Recommendation: use the `SceneInstanceReady` observer for
initialising `AnimationController` and `IkChain`.

### Spike 1 plan impact

No findings invalidate the plan. Confirmed: (a) AnimationPlayer is on a
descendant — the plan's descendant-walk is correct; (b) named bone lookup
works; (c) `AnimationGraph::from_clips` + `AnimationTransitions` supports
the idle/walk crossfade design; (d) `SceneInstanceReady` observer is the
preferred readiness mechanism; (e) both `bevy_animation` and `bevy_gltf`
workspace features are required.

One refinement: `AnimationController` should store `AnimationNodeIndex`
values (not clip handles) and the entity that holds `AnimationPlayer` (which
differs from the `SceneRoot` entity).

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
