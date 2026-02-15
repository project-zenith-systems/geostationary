## Scaffold atmospherics crate and GasGrid data structure

**Plan:** `plan/atmospherics` · [docs/plans/atmospherics.md](docs/plans/atmospherics.md)

Create the `modules/atmospherics` workspace crate with `AtmosphericsPlugin` and the pure-logic `GasGrid`.

**Files:**
- `modules/atmospherics/Cargo.toml` (new — depends on `bevy`, `tiles`)
- `modules/atmospherics/src/lib.rs` (new — `AtmosphericsPlugin`, resource wrapper)
- `modules/atmospherics/src/gas_grid.rs` (new — `GasGrid`, `GasCell`, passability, pressure derivation)
- Root `Cargo.toml` (add workspace member)

**Changes:**
- `GasGrid` struct with `width`, `height`, `cells: Vec<GasCell>`, `passable: Vec<bool>`
- `GasCell` with `moles: f32`
- `GasGrid::new(width, height)`, `sync_walls(&Tilemap)`, `pressure_at(IVec2)`, `set_moles(IVec2, f32)`, `total_moles()`
- `step(dt)` left as a no-op stub — diffusion comes later
- `AtmosphericsPlugin` registers the `GasGrid` as a Bevy resource
- Unit tests: grid construction, coordinate bounds, `sync_walls` marks walls impassable and floors passable, `total_moles` sums correctly, `set_moles` + `pressure_at` round-trip

**Not included:** debug overlay, wall toggling, diffusion algorithm

## Initialise atmosphere in world_setup

**Plan:** `plan/atmospherics` · [docs/plans/atmospherics.md](docs/plans/atmospherics.md)

After the tilemap is inserted, create a `GasGrid` from it and fill all floor cells with standard pressure.

Depends on: Scaffold atmospherics crate and GasGrid data structure

**Files:**
- `src/world_setup.rs` (modified — insert `GasGrid` resource after `Tilemap`)
- `src/main.rs` (modified — add `AtmosphericsPlugin`)

**Changes:**
- `setup_world` creates a `GasGrid` matching the tilemap dimensions, calls `sync_walls`, then `set_moles` on every passable cell to a standard pressure constant
- `cleanup_world` removes the `GasGrid` resource
- `AtmosphericsPlugin` added to the app plugin list

**Not included:** debug overlay, diffusion

## Add debug overlay for atmospherics pressure

**Plan:** `plan/atmospherics` · [docs/plans/atmospherics.md](docs/plans/atmospherics.md)

Render a toggleable pressure heatmap over the tilemap so the developer can see gas state at a glance.

Depends on: Initialise atmosphere in world_setup

**Files:**
- `modules/atmospherics/src/debug_overlay.rs` (new — overlay spawn/despawn, colour update system)
- `modules/atmospherics/src/lib.rs` (modified — register overlay systems, toggle resource)

**Changes:**
- `AtmosDebugOverlay` resource (bool toggle, off by default)
- F3 keypress toggles the overlay on/off
- When on: spawn one semi-transparent coloured quad per floor tile at y=0.01, coloured by pressure (blue=vacuum, green=normal, red=high)
- Each frame the overlay is active, update quad colours from `GasGrid::pressure_at`
- When toggled off: despawn all overlay entities
- At this point the overlay shows uniform green (all cells at standard pressure)

**Not included:** wall toggling, diffusion — overlay is tested with static data only

## Add wall toggle input and atmospherics wall sync

**Plan:** `plan/atmospherics` · [docs/plans/atmospherics.md](docs/plans/atmospherics.md)

A keypress toggles tiles between Wall and Floor. The GasGrid passability mask updates via change detection, and the debug overlay reflects the change.

Depends on: Add debug overlay for atmospherics pressure

**Files:**
- `modules/atmospherics/src/lib.rs` (modified — add wall-sync system, wall-toggle input system)

**Changes:**
- Wall-toggle system: a keypress (e.g., middle mouse or a debug key) while looking at a tile calls `Tilemap::set` to flip between Wall and Floor
- Wall-sync system: runs when `Tilemap.is_changed()`, calls `GasGrid::sync_walls` to update the passability mask
- When a wall is removed, the new floor cell starts at 0.0 moles (vacuum) — overlay shows blue
- When a wall is added, the cell becomes impassable and its moles are distributed to passable neighbours (or lost if fully enclosed)
- Tilemap mesh rebuild is already handled by `spawn_tile_meshes` via change detection
- Unit tests: `sync_walls` after wall removal marks cell passable with 0.0 moles; after wall addition marks cell impassable and redistributes moles

**Not included:** diffusion — pressure differences are visible in the overlay but gas does not flow yet

## Spike: validate diffusion algorithm

**Plan:** `plan/atmospherics` · [docs/plans/atmospherics.md](docs/plans/atmospherics.md)

Time-boxed experiment (60 min) to verify the finite-difference Jacobi diffusion converges, conserves mass, and handles sudden pressure discontinuities.

Depends on: Add wall toggle input and atmospherics wall sync

**Changes:**
- Write `GasGrid::step(dt)` implementation: for each passable cell, compute flow to each passable cardinal neighbour proportional to pressure difference, accumulate in scratch buffer, apply simultaneously (Jacobi iteration)
- Clamp flow so no cell goes negative
- Unit tests exercising the spike criteria:
  1. 12×10 test room grid, floor cells at 1.0 atm, run 200 ticks → pressure converges (max-min < 0.01)
  2. Remove a wall segment, run 200 more ticks → re-converges
  3. Total moles before and after are equal within epsilon
  4. No NaN or negative values
- Visually confirm via the debug overlay that pressure gradient looks correct
- If the approach is unstable, investigate flux-limiter or Gauss-Seidel before proceeding

**Output:** Comment on the task issue with findings. If the plan's algorithm assumption holds, continue to the next task. If not, update the Diffusion design section before proceeding.

**Not included:** wiring diffusion into the fixed-tick schedule — that is the next task

## Wire diffusion into the simulation tick

**Plan:** `plan/atmospherics` · [docs/plans/atmospherics.md](docs/plans/atmospherics.md)

Connect the validated diffusion algorithm to Bevy's fixed timestep so gas flows in real time.

Depends on: Spike: validate diffusion algorithm

**Files:**
- `modules/atmospherics/src/lib.rs` (modified — add diffusion tick system on `FixedUpdate`)

**Changes:**
- System runs in `FixedUpdate`: reads `GasGrid` as `ResMut`, calls `gas_grid.step(dt)` with the fixed timestep delta
- Wall sync runs before diffusion in the same schedule (sync first, then step)
- Observable result: toggle the debug overlay on, remove a wall, watch gas flow from the pressurised area into the vacuum as a smooth blue-to-green gradient
- Adding a wall mid-flow seals the boundary — gas on either side evolves independently
- Conservation test: `total_moles()` remains constant across ticks (within floating-point tolerance)

**Not included:** multiple gas species, temperature, entity interaction, networking, performance optimisation
