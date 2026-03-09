## Spike: GLTF scene hierarchy and AnimationPlayer access

Time-box: 30 minutes. Answer the questions in the plan's spike 1 section
before committing to the AnimationController design or scene-ready approach.

1. After spawning a `SceneRoot` from a `.glb`, what does the entity hierarchy
   look like? Where is `AnimationPlayer` in the tree — on the root, a child,
   or deeper?
2. Can we find a named bone entity (e.g., `Name("hand.R")`) by walking
   descendants of the scene root?
3. How does `AnimationGraph` work in Bevy 0.18? Can we create a graph with
   two clip nodes (idle, walk), insert it as an asset, and switch between
   them by setting the active node on `AnimationPlayer`?
4. How does scene readiness detection work — `SceneInstanceReady` trigger,
   polling for `AnimationPlayer` in descendants, or something else?

Create a minimal Bevy app in `spikes/` that loads a test `.glb`, spawns it,
logs the entity hierarchy, and attempts to drive animation transitions.
Keep the spike code — its assertions become regression tests.

If any answer invalidates the plan, update `docs/plans/character-models.md`
before continuing.

**Plan:** `plan/character-models` · [docs/plans/character-models.md](docs/plans/character-models.md)

## Spike: Bone reparenting for HandSlot

Time-box: 30 minutes. Answer the questions in the plan's spike 2 section.

1. Can we reparent an existing entity (spawned as a child of the creature
   root) to become a child of a bone entity inside the GLTF scene hierarchy?
2. Does the reparented entity inherit the bone's world transform correctly
   during animation playback?
3. Does reparenting interfere with Bevy's scene management or cause the
   entity to be despawned on scene reload?

Using the same test `.glb` from Spike 1, spawn a child entity on the scene
root, then after scene readiness, reparent it to a bone. Log world
transforms each frame to verify the child follows the bone through animation.

If reparenting does not work, the fallback is to spawn `HandSlot` directly
on the bone entity during scene-ready initialisation instead of reparenting.
Update the plan if needed.

Depends on: Spike: GLTF scene hierarchy and AnimationPlayer access.

**Plan:** `plan/character-models` · [docs/plans/character-models.md](docs/plans/character-models.md)

## Spike: Two-bone IK on animated skeleton

Time-box: 30 minutes. Answer the questions in the plan's spike 3 section.

1. Can we write to bone `Transform` components after `AnimationPlayer` has
   applied the clip pose, without the animation overwriting the IK result
   next frame? What system ordering is required?
2. Does a standard two-bone IK geometric solve produce correct joint
   rotations when the bones have non-identity rest poses (as exported from
   Blender)?
3. Can IK and clip animation coexist on the same skeleton — e.g., walk
   animation plays on legs while IK overrides the arm — without visual
   artefacts or transform conflicts?

Using the same test `.glb` from Spike 1, play the walk animation and
simultaneously apply a two-bone IK solve on the right arm to a fixed
world-space target. Visually verify legs walk while the arm reaches the
target. Log bone rotations to confirm IK output persists across frames.

If IK conflicts with AnimationPlayer (animation overwrites IK every frame
with no viable ordering), the fallback is masking specific bone tracks from
the animation clip or using an additive animation layer. Update the plan
if needed.

Depends on: Spike: GLTF scene hierarchy and AnimationPlayer access.

**Plan:** `plan/character-models` · [docs/plans/character-models.md](docs/plans/character-models.md)

## Create placeholder creature model

Create a placeholder rigged GLTF model with idle and walk animations. This
is a minimal humanoid mesh — visual quality is not the goal; correct
skeleton structure and named bones are.

Files created:

- `assets/models/creature.glb` — rigged humanoid with 2 animation clips

Concrete changes:

- Model has a skeleton with at minimum: root, spine, head, upper_arm.L,
  upper_arm.R, forearm.L, forearm.R, hand.L, hand.R, upper_leg.L,
  upper_leg.R, lower_leg.L, lower_leg.R, foot.L, foot.R
- Bone naming follows a standard convention (Mixamo-compatible) for future
  IK chain discovery — the arm chain is upper_arm.R → forearm.R → hand.R
- Two baked animation clips named: `idle` (breathing/sway loop), `walk`
  (walk cycle loop)
- No hold animation clip — holding is handled by IK
- Model scale and proportions match the existing capsule dimensions
  (roughly 0.3 radius, 1.0 height cylinder + hemispheres = ~1.6 total)
- Model origin at feet (y=0 is ground contact) to match the project's
  floor reference convention

Does not include: textures, facial features, clothing, multiple model
variants, or hold animation clips.

Depends on: All spikes completed (bone naming and IK chain confirmed).

**Plan:** `plan/character-models` · [docs/plans/character-models.md](docs/plans/character-models.md)

## Create animation module (L0)

New crate at `modules/animation/`. Game-agnostic animation state machine
and single-arm IK solver that drives Bevy's `AnimationPlayer` and bone
transforms.

Files created:

- `modules/animation/Cargo.toml` — depends only on `bevy`
- `modules/animation/src/lib.rs` — `AnimationPlugin`, `AnimState`,
  `AnimationController`, `drive_animation`, `IkChain`, `HoldIk`, `solve_ik`

Concrete changes:

- `AnimState` — `#[derive(Component, Clone, Copy, PartialEq, Eq)]` enum
  with variants `Idle`, `Walk`. Implements `From<u8>` and `Into<u8>` for
  wire encoding
- `AnimationController` — component storing a `Handle<AnimationGraph>` and
  a mapping from `AnimState` variant to `AnimationNodeIndex`
- `drive_animation` system — runs in `PostUpdate`, queries entities with
  `Changed<AnimState>` + `AnimationController`, finds the `AnimationPlayer`
  in descendants, initiates a crossfade transition to the target graph node
- `IkChain` — component storing root/mid/tip bone entity references and
  total chain length, for a two-bone IK chain
- `HoldIk` — component with `active: bool` and `target: Vec3` (local-space
  hold position relative to creature root)
- `solve_ik` system — runs in `PostUpdate` after `drive_animation` and
  before `TransformPropagate`. When `HoldIk::active`, solves two-bone IK
  for the arm chain and writes rotations to upper_arm and forearm bones
- `AnimationPlugin` stays Bevy-only and registers both systems without a
  `Headless` dependency; on the headless server the queries are inert
  because no `AnimationController`, `IkChain`, or `AnimationPlayer`
  components are spawned
- Add `animation` to workspace members in root `Cargo.toml`

Does not include: creature-specific logic, GLTF loading, template changes,
or network replication. This module knows about states, transitions, and
bone chains, not about creatures or items.

Depends on: All spikes completed (AnimationGraph API and IK ordering
confirmed).

**Plan:** `plan/character-models` · [docs/plans/character-models.md](docs/plans/character-models.md)

## Extend EntityState with animation and hold state

Add `anim_state` and `holding` fields to the network wire format so
animation state and hold pose can be replicated alongside position and
velocity.

Files touched:

- `modules/network/src/protocol.rs` — `EntityState` struct gains
  `anim_state: u8` and `holding: bool` fields
- `modules/things/src/lib.rs` — `ThingsStreamMessage::EntitySpawned` gains
  the same fields; `broadcast_state` and client lifecycle handling encode
  and apply them

Concrete changes:

- `EntityState { net_id, position, velocity, anim_state, holding }` — new
  fields, default to 0 / false
- `EntitySpawned` gains `anim_state: u8` and `holding: bool` for late-joiner
  initial state
- `broadcast_state` system reads `AnimState` and `HoldIk` from creature
  entities (if present) and encodes into `EntityState`; non-creature
  entities default to Idle / not holding
- `handle_entity_lifecycle` on client reads both fields from `StateUpdate`
  and `EntitySpawned`, inserts/updates `AnimState` and `HoldIk::active` on
  the local entity

Does not include: state computation (separate task) or animation/IK
playback (handled by animation module).

Depends on: Create animation module (L0) (needs `AnimState` and `HoldIk`
types).

**Plan:** `plan/character-models` · [docs/plans/character-models.md](docs/plans/character-models.md)

## Compute animation state and hold state

Add `compute_anim_state` and `compute_hold_state` systems to the creatures
module. Derives the correct `AnimState` from velocity and `HoldIk::active`
from hand contents.

Files touched:

- `modules/creatures/src/lib.rs` — new `compute_anim_state` and
  `compute_hold_state` systems
- `modules/creatures/Cargo.toml` — add dependency on `animation` module
  (for `AnimState`, `HoldIk`), add dependency on `items` module (for
  `Container`)

Concrete changes:

- `compute_anim_state` runs in `Update`, queries `(Creature,
LinearVelocity, &mut AnimState)`. If velocity magnitude > threshold
  (e.g., `0.1`) → `Walk`, else → `Idle`
- `compute_hold_state` runs in `Update`, queries `(Creature, Children,
&mut HoldIk)`. Finds child `HandSlot` entity, checks its `Container` —
  if any item present → `active = true`, else → `active = false`
- Both systems run on server (authoritative for replication)
- Velocity threshold is a const to avoid flicker at rest

Does not include: network replication (EntityState task), visual builder
changes, or HandSlot bone attachment.

Depends on: Create animation module (L0).

**Plan:** `plan/character-models` · [docs/plans/character-models.md](docs/plans/character-models.md)

## GLTF creature visual builder and headless skip

Replace the creature's primitive mesh visual builder with GLTF scene
loading. Skip visual builders on the headless server.

Files touched:

- `bins/shared/src/templates.rs` — creature visual builder rewritten
- `modules/things/src/lib.rs` — gate visual builder execution on
  `!Headless`

Concrete changes:

- Creature visual builder changes from `Mesh3d(capsule) +
MeshMaterial3d(orange)` to `SceneRoot(asset_server.load(
"models/creature.glb#Scene0"))`
- Visual builder also inserts `AnimationController` with clip handles
  looked up from the GLTF asset's named animations (idle, walk)
- Scene-ready initialisation: after the GLTF scene spawns its children,
  a system detects scene readiness (method from spike 1) and inserts a
  `SceneReady` marker component
- Scene-ready also populates `IkChain` by finding upper_arm.R, forearm.R,
  hand.R bone entities by name in the creature's descendants
- `on_spawn_thing` in things module: gate `visual_builders.get(kind)`
  call on `!world.contains_resource::<Headless>()` — server skips GLTF
  loading entirely
- Creature functional builder unchanged: still inserts `Creature`,
  `MovementSpeed`, `InputDirection`, `RigidBody`, `Collider`, etc.
- `AnimState::Idle` and `HoldIk { active: false, target: Vec3::new(0.3,
0.7, -0.3) }` inserted by the functional builder (needed on both server
  and client)
- Capsule mesh and material assets can be removed from `TemplatesPlugin`

Does not include: HandSlot bone attachment (separate task), animation/hold
state computation, or network replication.

Depends on: Create placeholder creature model, Create animation module
(L0).

**Plan:** `plan/character-models` · [docs/plans/character-models.md](docs/plans/character-models.md)

## HandSlot bone attachment

After the GLTF scene is ready, reparent the creature's `HandSlot` child
entity to the model's hand bone so held items follow the hand during
both animation and IK.

Files touched:

- `bins/shared/src/templates.rs` — `HandSlot` spawn adjusted (may keep
  initial spawn as child of root, or defer to scene-ready hook)
- `modules/things/src/lib.rs` or new system in `bins/shared/` — scene-ready
  system that finds the hand bone and reparents `HandSlot`

Concrete changes:

- After `SceneReady` marker is inserted on a creature entity, a system
  queries the entity's descendants for `Name("hand.R")` (bone name from
  spike 2)
- The `HandSlot` entity is reparented from the creature root to the hand
  bone entity using `commands.entity(hand_slot).set_parent(bone_entity)`
- `HandSlot` local transform set to `Vec3::ZERO` (or small offset if
  bone origin is not at grip point, determined by spike 2)
- `HAND_OFFSET` constant removed or marked as legacy fallback
- Held items now follow the hand through walk/idle animation and IK hold
  pose — when IK positions the hand, HandSlot (as a child of the hand
  bone) follows automatically

Does not include: item-specific hold positions or per-hand animation.

Depends on: GLTF creature visual builder and headless skip (needs
SceneReady marker, IkChain populated, and GLTF scene hierarchy).

**Plan:** `plan/character-models` · [docs/plans/character-models.md](docs/plans/character-models.md)
