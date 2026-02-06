# Geostationary - Systems Architecture

## What is Geostationary?

A round-based multiplayer space station simulation. Players are assigned roles
on a station - engineers, doctors, security, command - and must keep it
running while hidden antagonists work to undermine, subvert, or destroy it.
Rounds are ephemeral: each one builds a station, plays out, and ends. The
station is not saved; the stories are.

The simulation runs deep. Atmospherics models gas pressure across every tile.
Creatures have individual limbs and organs. Chemistry is combinatorial.
Electronics can be hacked. The complexity lives in the systems, and the
emergent gameplay lives in how players and antagonists exploit them.

## Philosophy

Geostationary is built on the principle of **stratified isolation** - a layered
architecture where each stratum serves a singular, well-bounded purpose. The
raw capability to move packets and solve physics constraints sits at the
bottom. The concept of a player logging into an account sits at the top.
Between them, the simulation builds upward in strict order: world primitives,
structural categories, core simulation, player-facing mechanics, social
systems, session governance, and persistence.

The architecture is divided into **eight layers**, numbered L0 through L7. This
numbering is deliberate: layers are referenced by number, not by name, to
reinforce the hierarchy as a first-class concept in every design conversation.

## The Two Horizons

The eight layers are divided across a fundamental boundary - the **compile
horizon** - which separates the engine into two distinct halves:

### The Compiled Substrate (L0 - L3)

The bottom four layers are native Rust, compiled directly into the engine
binary. They form the **substrate**: the bedrock upon which everything else
stands. These layers prioritise performance, safety, and determinism. They
change infrequently and are validated at compile time.

| Layer | Name         | Role |
|-------|--------------|------|
| L0    | System       | System backends: network, physics, input, animation, UI |
| L1    | Foundation   | World primitives: things, tiles, bootstrapping |
| L2    | Structural   | World structure: items, structures, atmos, gravity, abilities |
| L3    | Core         | Simulation: creatures, construction, chemistry, electronics, magic |

### The Scripted Canopy (L4 - L7)

The top four layers are **scripted** - loaded at runtime through interpretation,
JIT compilation, or hot-reloaded modules. They form the **canopy**: the
player-facing surface where mechanics are defined, social structures are
organised, sessions are governed, and persistence is managed. These layers
prioritise expressiveness, iteration speed, and moddability.

| Layer | Name         | Role |
|-------|--------------|------|
| L4    | Mechanics    | Player mechanics: souls, surgery, weapons, machines, access |
| L5    | Meta         | Players, comms, roles (jobs and antagonists) |
| L6    | Interface    | Admin, rounds, menus, interactions, camera, FOV |
| L7    | Cloud        | Persistence: saves, auth, preferences (optional layer) |

## Dependency Rule

The architecture enforces a strict **downward-only** dependency rule:

> A module may depend on any module from a **lower-numbered layer**, or on
> **external libraries** where appropriate. It may **never** depend on a module
> from a higher-numbered layer.

This rule is absolute. There are no exceptions, no "just this once" escape
hatches. Upward communication is achieved exclusively through events, trait
objects, or callback registration - never through direct dependency.

### External Library Policy

External crate and library dependencies should concentrate at the **lower
layers** of the stack. As you ascend the layer hierarchy, the code should
increasingly depend on the engine's own abstractions rather than reaching
out to third-party implementations directly. The substrate insulates the
canopy from the outside world.

## Layer Index

Detailed documentation for each layer:

- [L0 - System](layers/L0-system.md)
- [L1 - Foundation](layers/L1-foundation.md)
- [L2 - Structural](layers/L2-structural.md)
- [L3 - Core](layers/L3-core.md)
- [L4 - Mechanics](layers/L4-mechanics.md)
- [L5 - Meta](layers/L5-meta.md)
- [L6 - Interface](layers/L6-interface.md)
- [L7 - Cloud](layers/L7-cloud.md)

## Visual Overview

```
  L7  Cloud        ╌╌╌  Saves, auth, preferences (optional)
  L6  Interface    ╌╌╌  Admin, rounds, menus, interactions, camera, FOV
  L5  Meta         ╌╌╌  Players, comms, roles
  L4  Mechanics    ╌╌╌  Souls, surgery, weapons, machines, access, ...
 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━  compile horizon  ━━━
  L3  Core         ───  Creatures, construction, chemistry, electronics, magic
  L2  Structural   ───  Items, structures, atmos, gravity, abilities
  L1  Foundation   ───  Things, tiles, main menu bootstrap
  L0  System       ───  Network, physics, input, animation, UI
```

*Dashed lines (╌) denote scripted layers. Solid lines (─) denote compiled layers.*
