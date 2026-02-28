# Arc 2: Tangible Station

> **Goal:** The station is a designed, authored environment — not a hardcoded
> test room. Characters are animated 3D models, items have distinct meshes,
> tiles are textured, interactions produce spatial audio, and doors open and
> close with full model/animation/sound/networking/atmos integration. A
> content creator can paint a station layout in-game, save it, and load it
> on the server.

## Plans

1. **Map authoring & loading** — Station layouts are data files, not code.
   An in-game paint-mode editor lets a player place and remove tiles (floor,
   wall, airlock placeholder) and save the result. The server loads a map
   file on startup and replicates it to clients via the existing tilemap
   stream. The hardcoded test room in `world_setup.rs` is replaced by a
   default map file. Item spawn points (type and position) are defined in
   the map data and placeable in the editor. Requires: a map file format
   (RON), an editor UI overlay with tile palette, item placement, and
   save/load controls, a map loader that feeds `Tilemap` and spawns items
   on the server.

2. **Character models & animation** — Player creatures are rigged GLTF
   models with walk, idle, and hold-item animation states. The capsule
   placeholder is replaced. An animation state machine transitions between
   states based on movement velocity and hand contents. Animation state is
   replicated so all clients see the correct animation on every character.
   The GLTF loading pattern established here becomes the standard for all
   future 3D assets. Requires: GLTF asset loading integrated with
   `ThingRegistry`, an animation controller system (L0), creature template
   updated to reference a model asset, hand anchor (`HandSlot`)
   repositioned to match the model's hand bone, animation state replicated
   via stream 3 (small enum: idle/walk/hold).

3. **Tile art & lighting** — Floor and wall tiles use textured 3D meshes or
   materials instead of solid-colour primitives. Wall variants (corners,
   T-junctions, end caps) are selected automatically based on neighbour
   connectivity. Ambient lighting gives the station a baseline lit
   appearance. Dynamic light sources can be placed per-tile in the editor
   with placeholder models, providing localised illumination. Breached or
   vacuum areas are dark. Requires: tile mesh/material assets, an
   auto-tiling system that reads neighbour data from `Tilemap`, an ambient
   lighting model, per-tile placeable light entities with placeholder
   meshes, integration with the map editor (painted tiles use the new art,
   lights are placeable).

4. **Sound & ambience** — Interactions and environment produce spatial
   audio. Footsteps, item pickup/drop, wall break, decompression whoosh,
   and ambient station hum. The audio system is a thin wrapper at L0 that
   maps game events to sound assets with spatial positioning. Requires:
   an audio module (L0) that listens to Bevy messages and plays sounds,
   sound asset loading, spatial audio positioning relative to the listener
   (camera), a small library of sound effects.

5. **Doors** — The first full "content object" that exercises every
   pipeline built in plans 1–4. A door is a map-placed tile type with a
   GLTF model, open/close animation, sound effects, a click interaction
   (open/close toggle), networked state, and atmos integration (closed
   doors block gas flow like walls, open doors allow it). Proves that the
   content pipeline — model, animation, sound, map placement, interaction,
   networking, simulation coupling — works end to end. Requires: door tile
   type in the map format and editor, door model with open/close animation,
   door interaction (click to toggle), server-authoritative door state
   replicated to clients, atmos `GasGrid` treats closed doors as walls and
   open doors as passable, door sounds (open, close).

## Not in this arc

- **Advanced auto-tiling.** Diagonal walls, multi-tile structures, or pipe
  overlays. Basic 4-directional neighbour connectivity only.
- **Character customisation.** No clothing, hair, skin colour selection.
  All characters use the same model.
- **Item-specific animations.** Items display in-hand but the character has
  a single generic hold pose, not per-item animations.
- **Power grid or machines.** Lights are visual only — no power simulation,
  no APCs, no wiring. Lights are always on unless the tile is in vacuum.
  Power-dependent lighting is a future arc.
- **Access control on doors.** Doors open for everyone. Keycards, job-locked
  doors, and hacking are future mechanics.
- **Music or voice.** Sound is limited to spatial effects and ambient loops.
- **Client-side prediction of doors.** Door state waits for server
  confirmation, same as other interactions.
