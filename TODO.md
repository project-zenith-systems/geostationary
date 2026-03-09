## Spike: GLTF scene hierarchy and AnimationPlayer access

Time-box: 30 minutes. Answer the questions in the plan's spike 1 section
before committing to the AnimationController design or scene-ready approach.

1. After spawning a `SceneRoot` from a `.glb`, what does the entity hierarchy
   look like? Where is `AnimationPlayer` in the tree — on the root, a child,
   or deeper?
2. Can we find a named bone entity (e.g., `Name("hand.R")`) by walking
   descendants of the scene root?
3. How does `AnimationGraph` work in Bevy 0.18? Can we create a graph with
   three clip nodes (idle, walk, hold), insert it as an asset, and switch
   between them by setting the active node on `AnimationPlayer`?
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

## Create placeholder creature model

Create a placeholder rigged GLTF model with idle, walk, and hold-item
animations. This is a minimal humanoid mesh — visual quality is not the
goal; correct skeleton structure and named bones are.

Files created:
- `assets/models/creature.glb` — rigged humanoid with 3 animation clips

Concrete changes:
- Model has a skeleton with at minimum: root, spine, head, arm.L, arm.R,
  hand.L, hand.R, leg.L, leg.R bones
- Hand bone named `hand.R` (or as determined by spike 1) for HandSlot
  attachment
- Three baked animation clips named: `idle` (breathing/sway loop), `walk`
  (walk cycle loop), `hold` (arm raised with open hand, loop)
- Model scale and proportions match the existing capsule dimensions
  (roughly 0.3 radius, 1.0 height cylinder + hemispheres = ~1.6 total)
- Model origin at feet (y=0 is ground contact) to match the project's
  floor reference convention

Does not include: textures, facial features, clothing, multiple model
variants, or item-specific hold poses.

Depends on: Both spikes completed (bone naming convention confirmed).

**Plan:** `plan/character-models` · [docs/plans/character-models.md](docs/plans/character-models.md)

## Create animation module (L0)

New crate at `modules/animation/`. Game-agnostic animation state machine
that drives Bevy's `AnimationPlayer` based on a state enum.

Files created:
- `modules/animation/Cargo.toml` — depends only on `bevy`
- `modules/animation/src/lib.rs` — `AnimationPlugin`, `AnimState`,
  `AnimationController`, `drive_animation` system

Concrete changes:
- `AnimState` — `#[derive(Component, Clone, Copy, PartialEq, Eq)]` enum
  with variants `Idle`, `Walk`, `Hold`. Implements `From<u8>` and
  `Into<u8>` for wire encoding
- `AnimationController` — component storing a `Handle<AnimationGraph>` and
  a mapping from `AnimState` variant to `AnimationNodeIndex`
- `drive_animation` system — runs in `PostUpdate`, queries entities with
  `Changed<AnimState>` + `AnimationController`, finds the `AnimationPlayer`
  in descendants, initiates a crossfade transition to the target graph node
- `AnimationPlugin` registers the system with run condition
  `not(resource_exists::<Headless>)` — server does not drive animation
  playback
- Add `animation` to workspace members in root `Cargo.toml`

Does not include: creature-specific logic, GLTF loading, template changes,
or network replication. This module knows about states and transitions, not
about creatures or items.

Depends on: Both spikes completed (AnimationGraph API confirmed).

**Plan:** `plan/character-models` · [docs/plans/character-models.md](docs/plans/character-models.md)

## Extend EntityState with animation state

Add an `anim_state` field to the network wire format so animation state
can be replicated alongside position and velocity.

Files touched:
- `modules/things/src/lib.rs` — `EntityState` struct gains `anim_state: u8`
  field; `ThingsStreamMessage::EntitySpawned` gains `anim_state: u8`
- `modules/network/src/protocol.rs` — if `EntityState` is defined here,
  update accordingly

Concrete changes:
- `EntityState { net_id, position, velocity, anim_state }` — new `u8` field,
  defaults to 0 (Idle)
- `EntitySpawned` gains `anim_state: u8` for late-joiner initial state
- `broadcast_state` system reads `AnimState` from creature entities (if
  present) and encodes as `u8` into `EntityState`; non-creature entities
  default to 0
- `handle_entity_lifecycle` on client reads `anim_state` from `StateUpdate`
  and `EntitySpawned`, inserts/updates `AnimState` component on the local
  entity

Does not include: `AnimState` computation (separate task) or animation
playback (handled by animation module).

Depends on: Create animation module (L0) (needs `AnimState` type).

**Plan:** `plan/character-models` · [docs/plans/character-models.md](docs/plans/character-models.md)

## Compute animation state from velocity and hand contents

Add `compute_anim_state` system to the creatures module. Derives the
correct `AnimState` from movement velocity and whether the creature is
holding an item.

Files touched:
- `modules/creatures/src/lib.rs` — new `compute_anim_state` system
- `modules/creatures/Cargo.toml` — add dependency on `animation` module
  (for `AnimState`), add dependency on `items` module (for `Container`)

Concrete changes:
- `compute_anim_state` system runs in `Update`, queries entities with
  `(Creature, LinearVelocity, AnimState, Children)`
- Logic: if hand `Container` has an item → `Hold`; else if velocity
  magnitude > threshold → `Walk`; else → `Idle`
- `Hold` takes priority over `Walk` (holding while moving still shows hold)
- System runs on both server (authoritative) and client (for potential
  future local prediction, gated behind a feature flag or run condition)
- Velocity threshold is a const (e.g., `0.1`) to avoid flicker at rest

Does not include: network replication (EntityState task), visual builder
changes, or HandSlot attachment.

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
  looked up from the GLTF asset's named animations (idle, walk, hold)
- Scene-ready initialisation: after the GLTF scene spawns its children,
  a system detects scene readiness (method from spike 1) and inserts a
  `SceneReady` marker component
- `on_spawn_thing` in things module: gate `visual_builders.get(kind)`
  call on `!world.contains_resource::<Headless>()` — server skips GLTF
  loading entirely
- Creature functional builder unchanged: still inserts `Creature`,
  `MovementSpeed`, `InputDirection`, `RigidBody`, `Collider`, etc.
- `AnimState::Idle` inserted by the functional builder (needed on both
  server and client)
- Capsule mesh and material assets can be removed from `TemplatesPlugin`

Does not include: HandSlot bone attachment (separate task), animation
state computation, or network replication.

Depends on: Create placeholder creature model, Create animation module
(L0).

**Plan:** `plan/character-models` · [docs/plans/character-models.md](docs/plans/character-models.md)

## HandSlot bone attachment

After the GLTF scene is ready, reparent the creature's `HandSlot` child
entity to the model's hand bone so held items follow the hand during
animations.

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
- Held items now follow the hand through walk/idle/hold animations

Does not include: item-specific hold positions or per-hand animation.

Depends on: GLTF creature visual builder and headless skip (needs
SceneReady marker and GLTF scene hierarchy).

**Plan:** `plan/character-models` · [docs/plans/character-models.md](docs/plans/character-models.md)
