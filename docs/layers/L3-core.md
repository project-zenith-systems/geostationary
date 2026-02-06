# L3 - Core

> **Horizon:** Compiled Substrate (topmost compiled layer)
> **Depends on:** L0, L1, L2, external libraries
> **Depended on by:** L4 and above (across the compile horizon)

## Purpose

L3 is where the game comes alive. The layers below define what the world is
made of; L3 defines what happens in it. These are the systems that players
talk about, that fill wiki pages, that make the simulation feel real. Creatures
breathe the atmosphere that L2 simulates. Construction turns L2 structures
into something players deliberately build. Chemistry makes liquids dangerous
and interesting. Electronics give structures purpose.

This is also the last compiled layer - the top of the substrate. Everything
here is performance-critical and stable enough to justify native compilation.
These systems are complex, deeply interconnected with the structural layer
below, and important enough that they should not be subject to the runtime
overhead or instability of scripting. The compile horizon sits directly above
L3, and the APIs exposed here form the surface that the entire scripted canopy
binds against.

## Responsibilities

- Implement the core simulation systems that define the player experience
- Provide the richest and most wiki-documented gameplay systems
- Expose a stable API surface for the scripted canopy above
- Maintain native performance for systems too complex or critical to script

## Modules

| Module          | Description |
|-----------------|-------------|
| `creatures`     | The most complex system after atmospherics. A creature is any living entity - humanoid, animal, or robot - capable of interacting with the world. Creatures are modelled in detail: individual limbs are tracked for damage and function, internal organs (or mechanical parts for synthetics) are simulated, and the body as a whole mediates the creature's ability to act. Builds heavily on L1 `things` and L2 `abilities`. |
| `construction`  | The procedural layer on top of L2 `structures`. Where structures define what *can* exist on a tile, construction defines *how* it gets built: what tools are required, what materials are consumed, what steps must be followed, and what skill or role is needed. Turns raw structural definitions into player-driven building. |
| `chemistry`     | A reaction system for liquids and compounds. Chemicals can react with each other to produce new substances, and can apply status effects to their containers or anything they contact - intoxication, combustion, healing, corrosion, explosions. The system is combinatorial: the interesting behaviour emerges from mixing, not from individual substances in isolation. |
| `electronics`   | The functional nervous system of structures and items. Radios, door controls, power distribution, airlocks, machinery - electronics give built objects their behaviour. Also encompasses hacking: the subversion of electronic systems by tampering with their logic. A module that bridges the gap between passive structures and active, interactive objects. |
| `magic`         | Ritual-based supernatural effects. Magic operates through conditions and consequences: a ritual requires specific ingredients, locations, timing, or states of the world, and when satisfied, produces effects that can reach across many other systems. Deliberately broad in what it can touch - magic is the escape hatch for effects that don't fit neatly into physical simulation. |
| `station`       | The station as a cohesive whole. While L2 `structures` and `locations` define the physical layout, this module manages the station as a gameplay entity: power grids, alert levels, departmental operations, overall station state. The organisational layer that makes a collection of rooms into a functioning station. |
| `shuttles`      | Spacecraft that can move between locations. Shuttles are mobile collections of tiles - a small station that detaches, travels, and docks. They bridge the gap between the static tile grid and dynamic spatial movement, and are the primary means of transit between the station and the wider world. |

## Module Relationships

```
  L2 items ──────────► construction  (materials consumed)
  L2 structures ─────► construction  (what gets built)
  L2 structures ─────► electronics   (functionality for built objects)
  L2 abilities ──────► creatures     (what creatures can do)
  L2 atmospherics ───► creatures     (breathing, pressure damage)
  L2 locations ──────► station       (spatial organisation)
  L2 locations ──────► shuttles      (transit between locations)

  L1 things ─────────► creatures     (creatures are things)
  L1 tiles ──────────► shuttles      (shuttles are mobile tile grids)

  chemistry, magic ───► (broadly cross-cutting, touch many systems)
```

## External Dependencies

| Crate | Purpose |
|-------|---------|
| *TBD* | *To be selected during implementation* |

## Compile Horizon Interface

L3 is the last compiled layer. The scripted canopy (L4-L7) cannot directly
call Rust functions - it binds against the API surface that L3 exposes. This
makes the L3 API the most architecturally significant interface in the entire
stack.

The four scripted layers place specific demands on this surface:

- **L4 (Mechanics)** needs to manipulate creatures (equip clothes, perform
  surgery, install implants, apply weapon damage), operate machines, query
  chemistry reactions, and check electronics state. This is the heaviest
  consumer - 11 modules all reaching down through this boundary.
- **L5 (Meta)** needs to read station department structure for role assignment,
  access electronics for radio channels, and query creature state for soul
  binding.
- **L6 (Interface)** needs broad read access to present game state in menus,
  plus the ability to invoke interactions that route to L3/L4 systems.
- **L7 (Cloud)** needs to serialise creature state for character saves.

Guidelines for this boundary:

- **Stability over expressiveness.** The scripted canopy iterates fast; the
  API it sits on must not. Changes here ripple through every scripted module.
- **Coarse-grained operations.** Expose meaningful actions ("build wall at
  position", "apply reagent to container", "deal damage to limb"), not
  fine-grained internal state manipulation.
- **Event-driven upward communication.** L3 systems notify the canopy through
  events, never through direct calls to scripted code.
- **Query-friendly.** The canopy needs to read world state extensively.
  Provide efficient, well-typed query surfaces for creatures, station state,
  chemistry, etc.

## Design Notes

**Creatures as the complexity centre.** Individual limb tracking, organ
simulation, body-mediated interaction - creatures are where the simulation
depth is most visible to the player. This module will likely be the largest
single codebase in the substrate and warrants careful internal decomposition.

**Chemistry is combinatorial.** The value of the chemistry system is in the
emergent behaviour of mixing. The module must be designed around reaction
rules and substance properties, not hard-coded recipes, so that the
possibility space grows with the number of substances rather than linearly.

**Magic as a cross-cutting wildcard.** Magic rituals can produce effects that
touch creatures, chemistry, atmospherics, station state, and more. The ritual
system needs to be able to reach broadly across L2 and L3 without creating
tangled dependencies - likely through an effect/event system rather than
direct module coupling.

**Shuttles as mobile tile grids.** A shuttle is conceptually a small,
self-contained station that can move. This has deep implications: the tile
grid must support detachable, relocatable sections, and systems like
atmospherics and electronics need to function within a shuttle's local grid
while it is in transit.
