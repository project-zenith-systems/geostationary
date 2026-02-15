# Plan: Atmospherics

> **Stage goal:** A tile-based fluid simulation models gas pressure and flow
> across the station grid. Walls block gas propagation, removing a wall causes
> air to rush into the vacuum, and debug visuals let the developer see pressure
> values and flow direction in real time. The simulation runs as a pure-logic
> core with no Bevy dependency, wrapped by a thin plugin that bridges it into
> the ECS.

## What "done" looks like

1. A new `modules/atmospherics` workspace crate provides `AtmosphericsPlugin`
2. Floor tiles hold gas — each floor cell has a pressure value and gas mixture
3. Wall tiles block gas flow — gas cannot pass through walls
4. Gas diffuses between adjacent floor tiles towards pressure equilibrium
5. Removing a wall at runtime (changing a tile from Wall to Floor) causes gas
   to flow into the newly opened cell, visibly equalising pressure
6. Adding a wall at runtime (changing a tile from Floor to Wall) seals the
   boundary — gas on either side evolves independently
7. A debug overlay renders pressure per tile as a coloured quad (blue=vacuum,
   green=normal, red=high pressure) and can be toggled with a key
8. The simulation conserves total gas mass — the sum of all moles across the
   grid remains constant (within floating-point tolerance) unless gas is
   explicitly added or removed
9. The fluid core is a plain Rust struct with no Bevy imports, testable with
   `#[test]`
10. No `atmospherics` internals (grid storage, diffusion math) leak outside
    the module — game code interacts through the plugin's public API

## Strategy

Atmospherics is computationally demanding and algorithmically tricky. The
physics foundation post-mortem showed that unverified assumptions about runtime
behaviour cost structural rewrites. This plan front-loads the **debug tooling**
— the overlay and wall toggling — so that every subsequent step of the
diffusion work can be observed and troubleshot in real time.

The implementation order is:

1. **Scaffold the module and GasGrid data structure.** Create the crate, the
   grid with passability derived from the tilemap, and the ability to set/read
   moles per cell. No simulation yet — just static data.
2. **Debug overlay.** Render pressure-coloured quads on every floor tile so
   the developer can see the grid state. At this point the overlay just shows
   uniform green (all cells initialised to standard pressure).
3. **Wall toggling.** A keypress flips a tile between Wall and Floor. The
   tilemap rebuilds its meshes, the GasGrid syncs its passability mask, and
   the overlay updates — newly opened cells show blue (vacuum), sealed areas
   stay green. Still no diffusion; this validates the wall-sync pipeline.
4. **Spike the diffusion algorithm.** With the overlay and wall toggle already
   working, the spike can be run visually in-game as well as in a headless
   unit test. This makes it far easier to diagnose instability or mass loss.
5. **Implement diffusion.** Wire the validated algorithm into the fixed-tick
   schedule. Open a wall and watch gas flow into the vacuum in real time.

The architecture separates the simulation into two layers:

- **`GasGrid`** — a pure Rust struct that owns the gas data and runs the
  diffusion step. No Bevy types. This is where all the interesting logic lives
  and where all the interesting tests run.
- **`AtmosphericsPlugin`** — thin ECS glue that owns the `GasGrid` as a
  resource, ticks it on a fixed schedule, syncs with the tilemap for wall
  changes, and drives the debug overlay.

This separation follows the testing strategy's "separate pure logic from ECS
glue" principle and makes the notoriously fiddly diffusion algorithm easy to
test in isolation.

### Layer participation

| Layer | Module | Plan scope |
|-------|--------|------------|
| L2 | `atmospherics` | **New.** Workspace crate. Pure-logic `GasGrid` + `AtmosphericsPlugin`. Diffusion simulation, wall sync, debug overlay. |
| L1 | `tiles` | Add `Tilemap::set` support at runtime (already exists). Add a wall-toggle input system for testing. |
| — | `world_setup` | Initialise the atmosphere with standard air pressure after tilemap insertion. |

### Not in this plan

- **Gas mixtures / multiple species.** The grid stores a single "air" value
  (moles + derived pressure). Oxygen, nitrogen, CO2, toxins are future work.
  The data structure will be designed to accommodate mixtures later (moles per
  species), but only one species is simulated.
- **Temperature.** Thermal simulation and its interaction with pressure
  (ideal gas law) is a separate concern. Pressure in this plan is derived
  from moles alone using a simplified model.
- **Hull breaches / space exposure.** Tiles at the grid edge or adjacent to
  "space" tiles drain gas to zero, but there is no exterior environment model,
  decompression force, or entity interaction.
- **Entity interaction.** Creatures do not breathe, suffocate, or get pushed
  by gas flow in this plan. That is L3 behaviour.
- **Networking.** The atmospherics grid is not replicated. Server-authoritative
  atmos is future work.
- **Performance optimisation.** No spatial partitioning, SIMD, chunk sleeping,
  or threading. The test room is 12×10 — performance is not a concern at this
  scale. The pure-logic separation makes future optimisation straightforward.
- **Sound effects.** No hissing, whooshing, or decompression audio.

### Module placement

```
modules/
  atmospherics/           # L2 workspace crate — NEW
    Cargo.toml            # depends on bevy 0.18, tiles
    src/
      lib.rs              # AtmosphericsPlugin, public API, ECS glue
      gas_grid.rs         # GasGrid pure logic, diffusion algorithm
      debug_overlay.rs    # Debug visualisation system
src/
  world_setup.rs          # MODIFIED — initialise atmosphere after tilemap
```

### GasGrid design

The `GasGrid` is a plain Rust struct with no Bevy dependency:

```rust
pub struct GasGrid {
    width: u32,
    height: u32,
    cells: Vec<GasCell>,
    passable: Vec<bool>,       // true = floor, false = wall/void
}

pub struct GasCell {
    pub moles: f32,            // total gas quantity
}
```

**Pressure derivation:** `pressure = moles * R * T / V` where R, T, and V
are constants for this plan (fixed temperature, unit cell volume). In practice
this reduces to `pressure = moles * PRESSURE_CONSTANT`.

**Wall sync:** Each tick, the plugin reads the `Tilemap` resource and updates
the `passable` array. When a cell transitions from impassable to passable, its
moles start at 0.0 (vacuum) and gas flows in from neighbours. When a cell
transitions from passable to impassable, its moles are distributed equally to
passable neighbours (or lost if none exist).

**Public API:**

- `GasGrid::new(width, height) -> Self`
- `GasGrid::sync_walls(tilemap: &Tilemap)` — update passability from tilemap
- `GasGrid::step(dt: f32)` — run one diffusion tick
- `GasGrid::pressure_at(pos: IVec2) -> Option<f32>`
- `GasGrid::set_moles(pos: IVec2, moles: f32)`
- `GasGrid::total_moles() -> f32` — for conservation checks

### Debug overlay design

A toggleable visual layer that renders one semi-transparent coloured quad per
floor tile, tinted by pressure:

- **Blue** (0.0, 0.0, 1.0) at 0 pressure (vacuum)
- **Green** (0.0, 1.0, 0.0) at standard pressure (~1.0 atm equivalent)
- **Red** (1.0, 0.0, 0.0) at 2× standard pressure or above

The quads sit slightly above the floor plane (y=0.01) to avoid z-fighting.
The overlay is toggled by a keyboard shortcut (F3 or similar debug key) and
is off by default. The overlay entities are spawned/despawned on toggle, not
hidden, to avoid per-frame cost when not debugging.

The overlay updates every frame by reading `GasGrid::pressure_at` for each
tile and updating the quad material colour. This is acceptable for the 12×10
test room. For larger grids, the overlay would need batching or a custom
shader — but that is not in this plan.

### Wall mutation design

For this plan, wall toggling is a simple test/debug mechanism. A system listens
for a keypress (e.g., pressing a key while looking at a tile) and calls
`Tilemap::set` to flip the tile between Wall and Floor. The tiles module
already has `set()` and marks the resource as changed, which triggers
`spawn_tile_meshes` to rebuild the visual geometry.

The atmospherics plugin detects tilemap changes via Bevy's change detection
(`Res<Tilemap>.is_changed()`) and calls `GasGrid::sync_walls` to update the
passability mask. This keeps the two systems loosely coupled — the tilemap is
the source of truth, and atmospherics reacts to it.

### Spike: diffusion algorithm validation

**Question:** Does a simple finite-difference pressure equalisation converge
to equilibrium, conserve mass, and remain stable with the wall-removal
scenario (sudden pressure discontinuity)?

**Method:** With the debug overlay and wall toggle already working, the spike
can be validated both visually (in-game) and in a headless unit test. Write
`GasGrid::step()` and exercise it with:
1. A 12×10 grid matching the test room layout, floor cells at 1.0 atm
2. Run 200 diffusion ticks, assert convergence (max-min < 0.01)
3. Remove a wall segment, run 200 more ticks, assert re-convergence
4. Assert total moles before and after are equal (within epsilon)
5. Visually confirm via the debug overlay that pressure flows as expected

**Success criteria:** Pressure converges, total moles are conserved, no NaN
or negative values appear, and the overlay shows a smooth gradient from high
to low pressure across the breach.

**Time box:** 60 minutes. If the simple approach is unstable, investigate
a flux-limiter or Gauss-Seidel iteration before updating the plan.

### Diffusion design

**Diffusion step:** For each cell, examine the four cardinal neighbours. For
each passable neighbour, compute `flow = (self.pressure - neighbour.pressure)
* DIFFUSION_RATE * dt`. Clamp flow so a cell never goes negative. Accumulate
flows into a scratch buffer, then apply all flows simultaneously (Jacobi
iteration). The simultaneous-apply is critical for stability and conservation
— updating in-place would create directional bias and leak mass.

---

## Post-mortem

*To be filled in after the plan ships.*
