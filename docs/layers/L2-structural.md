# L2 - Structural

> **Horizon:** Compiled Substrate
> **Depends on:** L0, L1, external libraries
> **Depended on by:** L3 and above

## Purpose

L2 takes the bare primitives from L1 - things and tiles - and gives them
shape. Where L1 says "things and tiles exist", L2 says "here is what the
world is made of and how it behaves at a physical level". This is the layer
that defines the structural vocabulary of the station: what can be built,
what can be placed, how spaces are identified, what fills them, and what
forces act upon the objects within.

These modules form the bones of the simulation. They are not concerned with
gameplay rules or player-facing mechanics - they define the *material reality*
of the game world that those higher systems operate on.

## Responsibilities

- Specialise L1 primitives into concrete world-object categories
- Define spatial organisation, connectivity, and environmental systems
- Provide the physical and structural simulation layers (gravity, atmospherics)
- Establish the capability framework for characters (abilities)

## Modules

| Module          | Description |
|-----------------|-------------|
| `items`         | Non-living things in the world. Tools, components, materials, consumables - anything a character might pick up, use, or interact with but which is not itself alive. Builds on the L1 `things` primitive with item-specific data and behaviour. Items are one of the most heavily extended primitives in the stack: L3 uses them as construction materials and chemistry containers, and L4 specialises them further into clothes, weapons, consumables, and implants. |
| `structures`    | Tile-bound constructions that compose to form the built environment. Structures are layerable: floor tiles sit on floor panels sit on girders; a window slots into a window frame. Multiple structures can occupy the same tile position at different layers, and it is their composition that defines what a tile *is*. Builds on L1 `tiles`. |
| `connectables`  | A tile behaviour system for structures that need visual continuity with their neighbours. Walls, pipes, cables, conveyors - anything where the sprite or mesh must adapt based on what is adjacent. Handles neighbour detection and graphic variant selection. |
| `locations`     | Spatial identity and area definition. Provides the means to name and reference points and regions of the station: raw coordinates, GPS beacons, room boundaries, department zones, sections. Anything that answers the question "where is this?" or "what area does this belong to?" lives here. Locations are the spatial foundation that L3 `station` organises into a functioning whole, that L4 `access` gates with permissions, and that L5 `roles` maps to departments. |
| `decals`        | Flat visual entities that sit on top of tiles. Drawings, bloodstains, scorch marks, liquid puddles, warning labels. Primarily a graphical concern - decals add visual detail to the tilemap without altering the structural composition of the tile beneath. |
| `gravity`       | A simple binary toggle: grounded or weightless. Entities either have floor contact and walk normally, or they are floating and must grab surfaces to manoeuvre. Not a physics simulation - just a property of a space that other systems can query. |
| `atmospherics`  | A full fluid-dynamics simulation for gas behaviour on the station. Models pressure differentials, gas flow, mixture composition, and propagation across the tile grid. When a hull breach opens, atmospherics is what makes the air rush out. One of the most computationally demanding systems in the substrate. |
| `abilities`     | The capability framework for characters. Defines what actions a character *can* perform, as a broadly extensible system. Specific abilities and their effects are defined at higher layers; L2 provides the structural scaffolding for registering, querying, and invoking them. L3 `creatures` use abilities to mediate what a body can do, and L4 `genetics` can modify them at a biological level. |

## Module Relationships

The modules at L2 split naturally along the L1 primitive they extend:

```
  L1 things ──► items
  L1 things ──► abilities  (attached to living things)

  L1 tiles  ──► structures
  L1 tiles  ──► connectables  (behaviour of certain structures)
  L1 tiles  ──► decals  (visual overlay on tiles)
  L1 tiles  ──► locations  (spatial identity of tile regions)

  L0 physics ──► gravity  (simplified force model)
  L0 physics ──► atmospherics  (fluid simulation on the tile grid)
```

## Design Notes

**Structures and composition.** The layering model for structures is central
to how the station is built. A single tile position is not "one thing" but a
vertical stack of structural layers. This has implications for construction,
deconstruction, damage, and rendering - all of which depend on being able to
query and manipulate individual layers within a tile.

**Atmospherics performance.** Fluid simulation across a full station grid is
expensive. This module will likely need careful attention to spatial
partitioning, update frequency, and approximation strategies. Its placement
at L2 in the compiled substrate (rather than in the scripted canopy) is
deliberate - this is a system where native performance is non-negotiable.

**Gravity as a toggle.** The binary model is straightforward by design -
entities are either grounded or floating. It affects movement and interaction
but is not a central gameplay pillar. It exists at L2 because it is a
structural property of a space, not a mechanic in its own right.
