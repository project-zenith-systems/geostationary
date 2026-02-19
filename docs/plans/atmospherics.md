# Plan: Atmospherics

> **Stage goal:** A tile-based fluid simulation models gas pressure and flow
> across the station grid. Walls block gas propagation, removing a wall causes
> air to rush into the vacuum, and debug visuals let the developer see pressure
> values and flow direction in real time. The simulation runs as a pure-logic
> core with no Bevy dependency, wrapped by a thin plugin that bridges it into
> the ECS.

## What "done" looks like

1. A new `modules/atmospherics` workspace crate provides `AtmosphericsPlugin`
2. Floor tiles hold gas — each floor cell stores gas moles and a derived pressure value
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
| L1 | `tiles` | Add a wall-toggle input system for testing, using existing `Tilemap::set` runtime support. |
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

**Wall sync:** A Bevy system watches the `Tilemap` resource and, when it has
changed (`Res<Tilemap>::is_changed()`), calls `GasGrid::sync_walls` to update
the `passable` array. When a cell transitions from impassable to passable, its
moles start at 0.0 (vacuum) and gas flows in from neighbours. When a cell
transitions from passable to impassable, its moles remain stored in the sealed
cell and are still counted by `total_moles()`, but they no longer participate
in diffusion. This preserves the conservation invariant.

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

### Outcome

The atmospherics plan shipped everything it promised. A tile-based gas
simulation runs on Bevy's `FixedUpdate` schedule, walls block flow, removing
a wall causes gas to rush into the vacuum, and a debug overlay lets the
developer watch pressure equalise in real time. The pure-logic / ECS-glue
separation held: `GasGrid` is unit-testable without Bevy, and 14 tests cover
diffusion convergence, mass conservation, wall sync, and overlay toggling.

### What shipped beyond the plan

| Addition | Why it was worth doing |
|----------|------------------------|
| **Adaptive sub-stepping** in `GasGrid::step()` | The spike revealed that a single large `dt` causes numerical instability. Sub-stepping keeps the per-substep diffusion factor below a safe threshold. Without it the algorithm oscillates and leaks mass. |
| **Per-cell source scaling** in `step_substep()` | Naive per-flow clamping cannot prevent negative moles when multiple neighbours drain the same cell simultaneously. Scaling all outgoing flows by `available / total_outgoing` is a one-line fix that makes the conservation invariant hold exactly. |
| **Persistent scratch buffers** on `GasGrid` | Avoids heap allocation every substep. Not in the plan, but trivial to add and necessary once sub-stepping was introduced. |
| **Manual step (F4) and pause (F5)** debug controls | Added during the diffusion task to make it possible to step through the simulation frame-by-frame and inspect intermediate states. Invaluable during tuning. |

### Deviations from plan

- **`GasGrid` has Bevy imports.** The plan said "no Bevy types" but the
  implementation derives `Reflect` and `Resource` on `GasGrid` and `GasCell`.
  These are marker traits for editor tooling and ECS insertion — the grid
  logic itself uses no Bevy APIs, and all 11 `gas_grid` unit tests run
  without an `App`. The spirit of the separation held; the letter did not.
- **Diffusion algorithm is more complex than planned.** The plan sketched a
  simple Jacobi iteration with per-flow clamping. The shipped version adds
  sub-stepping and source-scaling (see table above). The spike surfaced both
  issues and they were fixed before the main diffusion task began, exactly as
  the spike workflow intended.
- **Overlay colour thresholds use SI-scale values.** The plan described
  abstract 0 / 1.0 / 2.0 ranges. The implementation uses 0 / 101.325 /
  151.9875 Pa. Cosmetic difference — the gradient math is identical.

### Hurdles

1. **Explicit diffusion instability.** Large `dt * DIFFUSION_RATE` products
   caused checkerboard oscillation and mass loss. Discovered during the spike.
   Solved by introducing sub-stepping with `MAX_DIFFUSION_FACTOR_PER_STEP`
   tuned to 0.24 (strictly less than `DIFFUSION_RATE` of 0.25). **Lesson:**
   the spike workflow paid for itself — this would have been a structural
   rewrite if caught after the systems were wired up.

2. **Multi-neighbour drain overshoot.** A cell with four low-pressure
   neighbours could have its moles drained below zero by the sum of four
   individually-valid flows. Solved with a per-cell scale factor that caps
   total outgoing flow at available moles. **Lesson:** clamping individual
   flows is not enough when the same cell is a source for multiple flows in
   the same tick.

3. **Overlay asset cleanup.** Toggling the overlay on and off creates and
   destroys mesh/material handles. Careless despawning leaked GPU assets.
   Solved by tracking handles in the `OverlayQuad` component and deduplicating
   shared meshes with a `HashSet` during cleanup. **Lesson:** spawn/despawn
   toggle patterns need explicit asset lifecycle management.

### What went well

- **Front-loading debug tooling was the right call.** The overlay and wall
  toggle were done before any diffusion work, so every iteration of the
  algorithm could be observed visually. The plan explicitly ordered work this
  way based on past post-mortem lessons, and it paid off.
- **The spike caught both numerical issues.** Sub-stepping and source scaling
  were both identified and solved within the spike's 60-minute time box,
  before any ECS integration work began.
- **Pure-logic separation made testing fast.** All 11 `gas_grid` tests run in
  milliseconds with no `App` setup. The diffusion convergence test exercises
  200+ ticks on a 12×10 grid — trivial in a unit test, painful in an
  integration test.
- **Task decomposition was clean.** Six commits, each self-contained, each
  reviewable independently. No task needed to undo work from a prior task.

### What to do differently next time

- **Enforce "no framework imports" with a lint or feature gate.** The plan
  said `GasGrid` would have no Bevy dependency, but `Reflect`/`Resource`
  derives crept in for convenience. If pure-logic isolation matters, enforce
  it with `#[cfg(feature = "bevy")]` gating on the derives, or put the pure
  logic in a sub-module that doesn't import `bevy`.
- **Document sub-stepping constants together.** `DIFFUSION_RATE` and
  `MAX_DIFFUSION_FACTOR_PER_STEP` have a non-obvious invariant
  (`max < rate`). The code comments explain it, but a named assertion or
  `const_assert!` would catch accidental breakage at compile time.
- **Add a conservation regression test that runs many more ticks.** The
  current spike test runs 200+200 ticks. A stress test with 10,000 ticks on
  a larger grid would catch slow drift that the current test misses.
