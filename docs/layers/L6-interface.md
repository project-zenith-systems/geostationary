# L6 - Interface

> **Horizon:** Scripted Canopy
> **Depends on:** L0-L5
> **Depended on by:** L7 only

## Purpose

L6 is the layer between the game and the human. Not in the visual sense alone
- this is not just "the UI layer" - but in the broader sense of everything
that mediates how a person *experiences and controls* the game. What the
player sees (camera, field of view), what they can do (interactions), what
the screen shows them (menus), how a session is structured (rounds), and
how admins shape the experience from above (admin tools).

These modules are abstract in nature. They do not add simulation content -
they define how the simulation is *presented, controlled, and governed*.

## Responsibilities

- Mediate between the simulation and the human experience of it
- Control what the player sees, how they act, and what the UI presents
- Structure gameplay sessions into rounds with defined lifecycles
- Provide admin tools for server governance and event orchestration

## Modules

| Module           | Description |
|------------------|-------------|
| `admin`          | The toolbox for server operators and game masters. Admin actions span the entire stack: spawning creatures, modifying station state, adjusting roles, ending rounds, banning players. This module does not implement those capabilities - it provides the admin-facing interface to invoke them, gated by appropriate permissions. The control room above the simulation. |
| `rounds`         | The session lifecycle. A round begins - typically by arriving at the station - and ends when the station is destroyed, evacuated, or an admin calls it. Rounds orchestrate the arc of a play session: setup, role assignment, gameplay, and conclusion. The module that gives the sandbox a beginning and an end. |
| `menu`           | The player-facing UI that hooks into systems across the stack. Inventory screens, crafting interfaces, communication panels, status displays, settings - the menu module is the connective tissue between the L0 `ui` rendering backend and the game state it needs to present. Broad by necessity, as it must surface information from nearly every lower layer. |
| `interactions`   | The discrete actions that creatures perform in the world. Picking up an item, throwing it, activating a machine, opening a door, attacking - interactions are the atomic verbs of gameplay. This module defines the interaction system: how available actions are discovered, presented, selected, and executed. The bridge between player input and simulation consequence. |
| `camera`         | Controls what the player's viewport shows. Camera positioning, following, panning, zoom - the spatial framing of the player's view of the world. Works with L0 `input` for player-driven camera control and with other L6 modules (like `fov`) to determine the final presented view. |
| `fov`            | Field of view - the system that limits what the player can see. Not everything on the station is visible at once; fog of war, line of sight, darkness, and obstruction all constrain perception. FOV determines which tiles, things, and creatures are revealed to a given viewer, shaping information asymmetry and tension. |

## Design Notes

**Interface is not just UI.** The name is deliberate. This layer handles
the full interface between human and game: visual framing (camera, fov),
available actions (interactions), presented information (menu), session
structure (rounds), and governance (admin). The L0 `ui` module handles
rendering rectangles; L6 decides what those rectangles mean.

**Interactions as the verb system.** Every meaningful thing a player does
in the world is an interaction. The module acts as a unified dispatch layer:
given a player's context (what they're holding, what they're near, what
they're looking at), it determines what actions are available and routes
the selected action to the appropriate system below. This keeps the "what
can I do here?" logic in one place rather than scattered across mechanics.

**FOV as information control.** In a game with antagonists, secrets, and
hidden roles, controlling what each player can see is critical. FOV is
not just a rendering optimisation - it is a gameplay system that creates
tension through limited information. What you can't see matters as much
as what you can.
