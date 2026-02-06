# L4 - Mechanics

> **Horizon:** Scripted Canopy (lowest scripted layer)
> **Depends on:** L0-L3 (via scripting bridge), limited external libraries
> **Depended on by:** L5 and above

## Purpose

L4 is where the substrate meets the player. The compiled layers below provide
creatures, chemistry, construction, and electronics as raw simulation systems.
L4 takes those systems and defines the *specific* mechanics that players
actually engage with: what you can wear, what you can eat, what you can shoot,
what you can build with a machine, who is allowed through which door.

This is the largest layer by module count. That is by design - mechanics are
the broadest category of game logic, and placing them in the scripted canopy
means they can be iterated on, rebalanced, and extended without recompiling
the engine. Every module here talks to the substrate through the L3 API
surface, and each one represents a distinct, self-contained gameplay system.

## Responsibilities

- Define the specific gameplay mechanics that players interact with
- Specialise L3 simulation systems into concrete, content-driven behaviours
- Maintain clean separation between individual mechanics
- Leverage the scripted environment for rapid iteration and moddability

## Modules

| Module          | Description |
|-----------------|-------------|
| `souls`         | The player inhabits the world as a soul - an identity that can be bound to a creature to control it, and in theory unbound and rebound to another. Souls are the bridge between the human at the keyboard and the creature in the simulation. Also a powerful tool for admin operations: possessing NPCs, debugging creature state, or orchestrating events from the inside. |
| `clothes`       | Wearable items that, when placed in an appropriate equipment slot, alter the visual appearance of a creature. Clothes sit at the intersection of L2 `items` and L3 `creatures`, adding a presentation layer on top of the slot system. Primarily cosmetic, though specific clothing effects (armour, insulation) may be defined here or in adjacent modules. |
| `surgery`       | Medical procedures that maintain, repair, or modify creature health. Operates on the detailed body model from L3 `creatures` - individual limbs, organs, and parts. Surgery is procedural: it requires tools, conditions, skill, and follows defined steps. The module that turns the creature body simulation into something players can meaningfully interact with. |
| `objectives`    | Goals for players. Objectives give a round its shape - something to accomplish, something to prevent, something to survive. The module defines how objectives are assigned, tracked, and resolved, without prescribing specific objective content (which may live higher or be data-driven). |
| `genetics`      | A modification system for creatures that have genes. Genes define aspects of a creature's appearance and abilities, and can be read, altered, spliced, or activated. Builds on L3 `creatures` and L2 `abilities` to give certain creatures a mutable biological blueprint. |
| `weapons`       | Any item that deals damage. Laser rifles, blunt instruments, thrown bottles, improvised clubs - the weapon module does not distinguish by elegance. It defines how items translate player intent into damage application on the L3 `creatures` body model, including damage types, ranges, and attack patterns. |
| `machines`      | Structures that perform operations when activated. Crafting stations that combine materials into new items, dispensers, recyclers, fabricators - machines are the interactive face of L2 `structures` and L3 `electronics`. A machine takes input (materials, power, interaction) and produces output (items, effects, state changes). |
| `consumables`   | Food and drink. Consumables are the player-facing expression of L3 `chemistry` - the module that decides what happens when a creature ingests a substance. Nutrition, intoxication, poisoning, healing, and other status effects originate here, making chemistry's reaction system tangible to the player. |
| `implants`      | Electronics, but inside creatures. Implants are installed in the body (via `surgery`) and provide functionality analogous to what L3 `electronics` provides for structures: augmented abilities, internal tools, communication devices, tracking chips. Where electronics wire up structures, implants wire up bodies. |
| `cyborgs`       | A specific creature archetype: a playable robot housing a biological brain (or similar organic core). Cyborgs bridge the creature and electronics systems in a unique way - they are L3 `creatures` whose body is largely mechanical, governed by L3 `electronics`, but piloted by a `soul`. A specialisation that is distinct enough to warrant its own module. |
| `access`        | Authorisation and permissions across the station. Departments have areas, machines have clearance requirements, doors have locks. Access defines who is allowed where and what they are allowed to use, tying together L2 `locations`, L3 `station`, L3 `electronics`, and `souls` into a coherent security model. The bureaucratic backbone of station life. |

## Module Relationships

```
  Substrate dependencies (via L3 API):

  L3 creatures ────► souls, clothes, surgery, genetics, weapons,
                     consumables, implants, cyborgs
  L3 construction ─► machines
  L3 chemistry ────► consumables  (status effects from ingestion)
  L3 electronics ──► machines, implants, cyborgs, access
  L3 station ──────► access, objectives

  L2 items ────────► clothes, weapons, consumables
  L2 structures ───► machines
  L2 abilities ────► genetics
  L2 locations ────► access  (area permissions)

  Intra-layer relationships:

  souls ◄──────────► creatures (via L3)  (binding/unbinding)
  surgery ──────────► implants  (installation procedure)
  cyborgs ──────────► implants, souls  (mechanical body + organic pilot)
```

## Scripting Environment

L4 is the first scripted layer. All modules here are loaded at runtime and
interface with the compiled substrate through the L3 API boundary. The
specific runtime (interpreted, JIT, or hot-reloaded) is TBD, but the key
properties are:

- **No direct Rust calls.** All substrate interaction goes through the
  scripting bridge defined at the compile horizon.
- **Hot-reloadable.** Mechanics can be modified and reloaded without
  restarting the engine - critical for iteration speed.
- **Sandboxed from each other.** Individual mechanics should not reach into
  each other's internals; cross-mechanic interaction flows through the
  substrate's shared state and event systems.

## Design Notes

**Souls as indirection.** The soul concept decouples player identity from
creature identity. This is more than a convenience - it means creature death
does not necessarily end a player's round, admin tools can freely reassign
control, and the system naturally supports observer modes and body-swapping
mechanics.

**Largest module count is intentional.** Mechanics are the widest category
of game content. Keeping each mechanic as a separate, self-contained scripted
module means any one of them can be modified, disabled, or replaced without
touching the others. The scripted canopy pays for this flexibility with some
runtime overhead, but mechanics are not the performance bottleneck - the
substrate handles that.

**Consumables as chemistry's front door.** Chemistry at L3 defines what
reactions produce and what effects substances have. Consumables at L4 define
*how players experience those effects* - the presentation, the feedback, the
gameplay consequence of drinking something ill-advised.
