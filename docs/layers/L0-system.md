# L0 - System

> **Horizon:** Compiled Substrate
> **Depends on:** Bevy
> **Depended on by:** L1 and above

## Purpose

L0 is the ground floor - the system-level capabilities that the game needs
but which have no concept of a station, creature, or round. These are
domain-agnostic concerns: the ability to move packets, solve constraints,
read devices, interpolate keyframes, draw rectangles, and play sounds. L0
modules build on Bevy and its ecosystem to provide game-flavoured APIs that
the upper layers can use without caring about the underlying implementation.

## Responsibilities

- Provide system-level capabilities as self-contained modules
- Build on Bevy's infrastructure where appropriate
- Expose clean, game-flavoured APIs upward
- Remain entirely game-agnostic; no gameplay concepts exist here

## Modules

| Module      | Description |
|-------------|-------------|
| `network`   | Transport layer: connection management, packet framing, protocol abstraction. Provides reliable and unreliable channels without any knowledge of what is being sent. Must support the demands of a server-authoritative multiplayer simulation: many concurrent connections, frequent state updates, and both reliable (chat, interactions) and unreliable (movement, atmospherics sync) delivery. |
| `physics`   | Rigid-body dynamics, collision detection, spatial queries. A simulation engine that knows about shapes and forces, but not about what they represent. The upper layers use this for everything from thrown objects and projectiles to creature movement and shuttle docking - but at L0 these are just bodies and colliders. |
| `input`     | Device abstraction for keyboard, mouse, gamepad, touch. Captures raw device state and produces normalised input streams. No bindings, no actions - just signals. The interaction and camera systems at L6 translate these signals into meaningful player intent. |
| `animation` | Keyframe interpolation, skeletal blending, animation graph evaluation. Knows how to move bones and blend curves, but has no opinion on what is being animated or why. The creature body model at L3 - with its per-limb tracking and damage states - will be the primary consumer, driving skeletal animation for humanoids, animals, and robots alike. |
| `ui`        | Layout engine and rendering backend for 2D interface elements. Handles positioning, styling, and draw calls for panels, text, and images. Knows nothing about menus or HUDs. The menu system at L6 will drive this heavily, surfacing information from nearly every layer of the stack - inventory, chat, station alerts, surgery, chemistry, and more. |
| `audio`     | Sound playback, spatial audio, and mixing. Plays sounds at positions in the world, manages channels and volume. Does not know what an explosion or a radio is - just that something at a position wants to make a noise. |

## Design Notes

L0 modules should expose APIs shaped around what the game actually needs,
not a 1:1 mirror of whatever Bevy provides. The goal is a clean, purposeful
surface that upper layers can use without thinking about the underlying
engine plumbing.
