# L7 - Cloud

> **Horizon:** Scripted Canopy (topmost layer)
> **Depends on:** L0-L6
> **Depended on by:** Nothing - this is the outermost layer

## Purpose

L7 is the outermost shell - the layer that connects the game to the world
beyond a single session. Saves, accounts, and preferences: the persistent
state that follows a player across rounds and servers. Everything below L7
functions without it. The station runs, creatures live and die, rounds begin
and end - all without L7. But L7 is what turns a series of anonymous sessions
into a persistent, accountable experience.

Technically optional. Practically essential.

## Responsibilities

- Persist player data across sessions and servers
- Authenticate players and tie identity to accountability
- Store and retrieve player preferences and settings

## Modules

| Module          | Description |
|-----------------|-------------|
| `saves`         | Character persistence across rounds. When a round ends, a player's character can be saved and carried into a future session. This is not world-state serialisation - the station is ephemeral - but the preservation of a character's identity, progression, or state for later use. |
| `auth`          | Player authentication. Ties a human to an account so the game knows who they are across sessions. The foundation of accountability: you can only ban someone if you can identify them. Interfaces with L5 `player` to associate a session participant with a persistent identity. |
| `preferences`   | Game settings, key bindings, visual options, accessibility configuration - anything the player wants to persist between sessions that is not character data. Preferences are per-account, restored on login, and entirely cosmetic to the simulation below. |

## Design Notes

**Optional by design.** L7 can be entirely absent and the game still works.
A local or anonymous server runs the full L0-L6 stack without authentication,
saves, or persistent preferences. This is a deliberate architectural property:
the cloud layer is additive, never load-bearing. No system below should
ever check "is L7 available?" to decide how to behave.

**Saves are character-scoped, not world-scoped.** The station is not saved.
Each round builds a new station, plays out, and ends. What persists is the
player's character - carried forward into a new round on a new station. This
keeps the save system focused and avoids the complexity of full world-state
serialisation.

**Auth as the root of trust.** Authentication is simple in scope but critical
in function. Without it, admin actions like bans have no teeth - there is no
persistent identity to act against. Auth provides the anchor that makes
moderation, reputation, and accountability possible.
