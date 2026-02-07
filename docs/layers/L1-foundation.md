# L1 - Foundation

> **Horizon:** Compiled Substrate
> **Depends on:** L0
> **Depended on by:** L2 and above

## Purpose

L1 defines the foundational primitives of the game world - the two basic
categories of "stuff that exists" and the entry point that brings it all to
life. Where L0 provides raw system capabilities with no concept of a game,
L1 introduces the first game-aware abstractions: the distinction between
things that live on a grid and things that don't, and the bootstrapping
sequence that gets the player from launch to gameplay.

These primitives are deliberately minimal. L1 does not define what kinds of
things or tiles exist - it defines *that* they exist, and provides the
foundational data structures and lifecycle for working with them. Specific
categories (creatures, items, terrain types) are the concern of higher layers.

## Responsibilities

- Establish the two fundamental world-object primitives: things and tiles
- Provide the bootstrapping and main menu flow that initialises the game
- Define the foundational data structures that higher layers build upon
- Keep the vocabulary small - specific game content belongs above

## Modules

| Module      | Description |
|-------------|-------------|
| `things`    | The base primitive for any game-world object that is **not** grid-bound. Items, creatures, projectiles, effects - anything that exists freely in the world is, at its lowest level, a Thing. The name is deliberately chosen to avoid collision with the ECS concept of "entity". Higher layers define specific categories; L1 defines the shared foundation they all stand on. |
| `tiles`     | The base primitive for anything that **is** grid-bound. This covers both individual tile entities and the tilemap structure that organises them. A tile is defined by its position on the grid; the tilemap is the grid itself. Together they form the spatial backbone of the world. |
| `main_menu` | The bootstrapping module. Responsible for the initial game state after launch: main menu presentation, session initialisation, and the transition into gameplay. Exists at L1 because it stands apart from the runtime game systems above - it is the doorway through which the player enters, and it needs to be available before anything else is ready. |

## Naming: Why "Things"

Bevy's ECS claims the word "entity" as a core concept with precise technical
meaning. Using the same word for a game-world object would create constant
ambiguity: does "entity" mean the ECS row, or the in-game creature standing
in front of you?

**Thing** sidesteps this entirely. It is informal, unambiguous, and carries no
ECS baggage. A Thing is a game-world object. An entity is an ECS identifier.
They are related - every Thing has an entity - but the words never collide.

## Open Questions

- **`metadata`** - Prior design notes reference a metadata module at this
  layer. Its original purpose is unclear. Candidates worth considering: asset
  metadata indexing, type registration for things/tiles, or a tag/property
  system for the foundational primitives. To be revisited once the upper
  layers clarify what metadata they need to flow downward.

## Design Notes

The things/tiles split is the most fundamental spatial distinction in the
game world. Everything that exists in the world is one or the other. These
two primitives carry the entire game world - every module from L2 through L4
ultimately traces back to one of them. Some modules (like magic at L4) are
cross-cutting and don't root cleanly in either primitive.

**Main menu and the session lifecycle.** `main_menu` bootstraps the game
before any simulation systems are running. With the full picture visible,
its role is clearer: it is the entry point that precedes L6 `rounds`. The
main menu gets the player from launch to the point where a round can begin,
at which point L6 takes over the session lifecycle. If L7 `auth` is present,
the main menu is also where login occurs - but it must function without it.
