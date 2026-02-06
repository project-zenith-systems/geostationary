# L5 - Meta

> **Horizon:** Scripted Canopy
> **Depends on:** L0-L4
> **Depended on by:** L6 and above

## Purpose

L5 steps outside the simulation and looks at the game from above. Where L4
defines what happens *in* the world, L5 defines the structures that organise
the *experience* of playing: who the human is as a participant, how they
communicate, and what role they have been cast in. These are the systems that
shape a round before a single tile is built or a single creature is harmed.

Three modules. Clean, distinct, and foundational to the social and narrative
structure of the game.

## Responsibilities

- Manage the human player as a participant distinct from their in-world creature
- Provide communication channels tied to both in-world and out-of-world contexts
- Define the role system that assigns jobs, access, objectives, and allegiances

## Modules

| Module    | Description |
|-----------|-------------|
| `player`  | The human behind the screen, as a game participant. Distinct from the L4 `soul` (which binds to a creature) and from the creature itself - the player exists whether they are in a round, spectating, or sitting in chat. This module tracks the player as a person: their session, their identity, their participation state. The player is present in chat before they are ever present in the world. |
| `comms`   | The communication system, woven into the world but reaching beyond it. Channels are tied to in-world systems: radio frequencies carried by L3 `electronics`, proximity-based local chat, department channels gated by L4 `access`. But comms also spans the out-of-world: OOC (out of character) chat, LOOC (local out of character), and admin channels. A single system that handles both the diegetic and the meta. |
| `roles`   | What part you play. Roles define a player's job on the station - engineer, doctor, security officer - which in turn determines their L4 `access` permissions and `objectives`. But roles also encompass the hidden layer: antagonist assignments like vampires, secret agents, and impostors. A role is the narrative seed that shapes a player's entire round, whether they are crew or threat. |

## Module Relationships

```
  L4 souls ────────► player   (the soul is the player's presence in-world;
                               the player exists beyond it)
  L4 access ◄────── roles    (roles configure access levels)
  L4 objectives ◄── roles    (roles assign objectives)
  L3 electronics ──► comms    (radio channels via in-world devices)
  L3 station ──────► roles    (station departments map to job roles)

  Intra-layer:

  player ──────────► comms    (players are participants in chat)
  roles ───────────► comms    (role-specific channels, e.g. department radio)
```

## Design Notes

**Player vs. soul vs. creature.** Three distinct concepts, three different
layers:

```
  L5  player      The human participant. Exists outside the simulation.
  L4  soul        The player's agency in-world. Can bind to creatures.
  L3  creature    A simulated living entity. Can exist without a soul.
```

A player can be in chat without a soul. A soul can be unbound between
creatures. A creature can exist without either. Each layer owns its piece
of the identity stack.

**Comms as a diegetic bridge.** Most communication systems in games are
purely meta - chat is chat. Here, comms straddles both worlds: radio
channels are physical objects with frequencies, local chat has range, but
OOC and admin channels transcend the simulation. This dual nature places
comms naturally at L5, where it can reference both in-world systems (L3-L4)
and out-of-world player state.

**Roles as narrative architecture.** The role system is deceptively powerful.
It does not just assign a job title - it cascades through the layers below,
setting access permissions, defining objectives, and potentially marking
a player as an antagonist whose goals oppose the rest of the crew. A single
role assignment at L5 shapes the entire downstream experience.
