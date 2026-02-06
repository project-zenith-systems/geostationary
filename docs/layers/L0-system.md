# L0 - System

> **Horizon:** Compiled Substrate
> **Depends on:** External libraries only
> **Depended on by:** L1 and above

## Purpose

L0 is the ground floor - the raw system-level backends that interface directly
with hardware, operating system services, and third-party engines. Nothing at
this layer knows what a station, creature, or round is. These are
domain-agnostic capabilities: the ability to move packets, solve constraints,
read devices, interpolate keyframes, and draw rectangles. They are selected,
configured, and wrapped here so that every layer above can treat them as
stable, engine-owned primitives.

This is where the heaviest external dependency surface lives. L0 absorbs the
complexity of third-party integrations so the rest of the stack doesn't have to.

## Responsibilities

- Provide low-level system backends as self-contained modules
- Own the direct interface to external engines and platform APIs
- Expose clean, engine-flavoured APIs upward - never leak vendor types
- Remain entirely game-agnostic; no gameplay concepts exist here

## Modules

| Module      | Description |
|-------------|-------------|
| `network`   | Transport layer: connection management, packet framing, protocol abstraction. Provides reliable and unreliable channels without any knowledge of what is being sent. Must support the demands of a server-authoritative multiplayer simulation: many concurrent connections, frequent state updates, and both reliable (chat, interactions) and unreliable (movement, atmospherics sync) delivery. |
| `physics`   | Rigid-body dynamics, collision detection, spatial queries. A simulation engine that knows about shapes and forces, but not about what they represent. The upper layers use this for everything from thrown objects and projectiles to creature movement and shuttle docking - but at L0 these are just bodies and colliders. |
| `input`     | Device abstraction for keyboard, mouse, gamepad, touch. Captures raw device state and produces normalised input streams. No bindings, no actions - just signals. The interaction and camera systems at L6 translate these signals into meaningful player intent. |
| `animation` | Keyframe interpolation, skeletal blending, animation graph evaluation. Knows how to move bones and blend curves, but has no opinion on what is being animated or why. The creature body model at L3 - with its per-limb tracking and damage states - will be the primary consumer, driving skeletal animation for humanoids, animals, and robots alike. |
| `ui`        | Layout engine and rendering backend for 2D interface elements. Handles positioning, styling, and draw calls for panels, text, and images. Knows nothing about menus or HUDs. The menu system at L6 will drive this heavily, surfacing information from nearly every layer of the stack - inventory, chat, station alerts, surgery, chemistry, and more. |

## External Dependencies

| Crate | Purpose |
|-------|---------|
| *TBD* | *To be selected during implementation* |

## Design Notes

Each module at L0 acts as an **isolation boundary** around its corresponding
external dependency. If the physics backend is swapped from one library to
another, the blast radius is confined to `l0::physics` - nothing above should
need to change.

The APIs exposed by L0 modules should be:
- **Thin but opinionated** - not a 1:1 re-export of the vendor API, but a
  curated surface that reflects how the engine wants to use the capability.
- **Type-safe** - engine-owned types at the boundary, no raw pointers or
  vendor enums leaking upward.
- **Minimal surface** - expose what the upper layers need, not everything the
  vendor provides. The station sim has specific demands (many-body spatial
  queries, skeletal creatures, high-frequency network updates, dense 2D UI);
  the L0 API should be shaped around those realities.
