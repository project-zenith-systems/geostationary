# Arc: Networked World State

> **Goal:** Two players connect to a dedicated server, see each other's
> names, pick up and drop items, break walls to cause decompression that
> pushes entities through the breach — and every player sees the same
> thing. The server is the single source of truth for all world state.

## Plans

1. **Dedicated server with player souls** — A headless server hosts the
   world. Clients connect, choose a name, and get bound to a creature via
   the souls module (player identity decoupled from creature body). Names
   render as billboard text above each character. Disconnecting leaves the
   creature in the world; the soul unbinds. The bouncing ball is
   server-spawned and visible on all clients. Requires: extending entity
   replication beyond position/velocity to cover additional data (e.g. player
   display names) and initial world state (tilemap and atmos grid) sent to
   clients on connect, a souls module (L4), nameplate rendering, headless
   server mode as default host.

2. **Break a wall, watch gas rush out** — Tilemap mutations and atmos grid
   changes replicate in real time. Player A toggles a wall, player B sees
   the tile change and the pressure overlay update. Atmos pressure gradients
   apply forces to physics bodies — opening a wall next to vacuum pushes
   nearby entities toward the breach. Requires: replicating grid and
   tilemap mutations to clients, client-side atmos rendering without local
   simulation, a pressure-force system coupling GasGrid to Avian.

3. **Pick up and drop networked items** — Item entities spawn in the world,
   can be picked up and put down by players, and placed into containers.
   All interactions are server-authoritative and visible to every client.
   Requires: an items module (L2) with floor/hand/container slots, a
   server-authoritative interaction model with range validation.

## Not in this arc

- **Client-side prediction and rollback.** Clients snap to server truth.
- **Delta compression or bandwidth optimisation.** The test room is 12x10.
- **Gas mixtures, temperature, or advanced atmos.**
- **Equipment slots, crafting, item-specific behaviour.** Items are inert.
- **Roles, authentication, lobby.** Names are self-reported strings.
- **Death, respawn, or observer mode.** Souls unbind on disconnect but
  there is no death mechanic or ghost state in this arc.
