# Testing Strategy

This document defines the testing approach for Geostationary: what to test, how
to test it, what tooling to use, and how continuous integration fits together.
The goal is a strategy that scales with the project without becoming a burden
during early development.


## Principles

**Test the seams, not the framework.** Bevy's ECS, rendering, input handling,
and state machine are already tested by the Bevy project. Our tests should
focus on the logic we write — game rules, system interactions, data
transformations — not on verifying that Bevy works.

**Separate pure logic from ECS glue.** The most valuable and most testable code
is the code with no framework dependency at all. When a system function contains
interesting logic, extract that logic into a pure function that takes plain
arguments and returns plain values. The system function becomes thin glue that
reads components, calls the pure function, and writes results back. This
pattern makes the logic trivially testable with standard `#[test]` and keeps
system functions easy to read.

```rust
// Pure logic — testable without Bevy
pub fn resolve_movement(pos: Vec3, delta: Vec3, tilemap: &Tilemap) -> Vec3 {
    let target = pos + delta;
    if tilemap.is_walkable(target.x as i32, target.z as i32) {
        target
    } else {
        // Slide along the wall on the unblocked axis
        let slide_x = Vec3::new(target.x, pos.y, pos.z);
        let slide_z = Vec3::new(pos.x, pos.y, target.z);
        if tilemap.is_walkable(slide_x.x as i32, slide_x.z as i32) {
            slide_x
        } else if tilemap.is_walkable(slide_z.x as i32, slide_z.z as i32) {
            slide_z
        } else {
            pos
        }
    }
}

// ECS glue — thin, not worth unit testing
fn move_creatures(
    time: Res<Time>,
    tilemap: Res<Tilemap>,
    mut query: Query<(&mut Transform, &MovementSpeed), With<Creature>>,
    input: Res<ButtonInput<KeyCode>>,
) {
    let delta = input_to_direction(&input) * time.delta_secs();
    for (mut transform, speed) in &mut query {
        transform.translation = resolve_movement(
            transform.translation,
            delta * speed.0,
            &tilemap,
        );
    }
}
```

**Test at the right altitude.** Not everything needs a test. A 14-line loading
screen that spawns a text node is not worth testing. A movement resolver that
handles wall sliding across a tile grid absolutely is. Invest testing effort
where bugs would actually hurt.

**No mocking frameworks.** Bevy's `App` can be constructed headlessly with
`MinimalPlugins`, resources can be inserted directly, and the world can be
queried after updates. This gives us a real ECS environment for integration
tests without any mocking overhead. For pure logic tests, there is nothing to
mock — just call the function.


## Test tiers

### Tier 1 — Pure logic unit tests

Standard `#[test]` functions with zero Bevy dependency. These are the fastest
to write, fastest to run, and highest value per line of test code.

**Where they live:** `#[cfg(test)] mod tests` at the bottom of the source file
containing the logic.

**What they cover:**
- Tile grid operations (bounds checking, walkability queries, coordinate
  conversions)
- Movement resolution (wall collision, sliding, edge cases at grid boundaries)
- Data structure invariants (tilemap construction, default values)
- Enum round-trips and conversions
- Any mathematical or algorithmic logic extracted from systems

**Current candidates:**
- `UiTheme` default values are sane (colours are non-zero, font sizes are
  positive, spacing is reasonable)
- `ButtonColors` construction from theme produces distinct normal/hovered/pressed
  states

**Future candidates (playable character plan and beyond):**
- `Tilemap::is_walkable` boundary conditions
- `resolve_movement` against various wall configurations
- Chemistry reaction lookups
- Atmospherics gas conservation invariants

These tests should run in microseconds individually and should be written
alongside the code they test — not as an afterthought.

### Tier 2 — Bevy App tests

Headless integration tests that construct a Bevy `App` with `MinimalPlugins`,
add the systems under test, insert resources, send messages, call
`app.update()`, and assert against the resulting world state.

These tests verify that systems interact correctly with each other and with
the ECS — that messages are routed properly, that state transitions fire, that
components are spawned with the right data.

**Where they live:** `#[cfg(test)] mod tests` in the module file, or in a
`tests/` directory for cross-module scenarios.

**What they cover:**
- Message routing (e.g. `MenuEvent::Play` produces `NetCommand::Host { port: 7777 }`)
- State transitions (e.g. `NetEvent::Connected` moves `AppState` to `InGame`)
- Button interaction → message emission pipeline
- Network event draining (async channel → Bevy messages)
- System ordering and set configuration

**Constraints:**
- Must use `MinimalPlugins`, not `DefaultPlugins`. There is no GPU, window, or
  audio device in CI. Any plugin that requires a render backend will panic.
- Systems that depend on `Time` can use Bevy's `TimeUpdateStrategy` resource
  or manual time injection.
- Tests that need multiple update cycles should call `app.update()` in a loop
  with clear assertions between iterations.

**Example pattern:**

```rust
#[cfg(test)]
mod tests {
    use bevy::prelude::*;
    use super::*;

    #[test]
    fn play_event_sends_host_command() {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        app.add_message::<MenuEvent>();
        app.add_message::<NetCommand>();
        app.add_systems(PreUpdate, menu_message_reader);

        // Insert required resources
        app.insert_resource(UiTheme::default());
        // ... spawn MenuRoot entity ...

        // Write the event and tick
        app.world_mut().send_message(MenuEvent::Play);
        app.update();

        // Assert NetCommand::Host was emitted
        let commands: Vec<_> = app.world().read_messages::<NetCommand>().collect();
        assert!(matches!(commands[0], NetCommand::Host { port: 7777 }));
    }
}
```

### Tier 3 — Cross-boundary integration tests

Tests that exercise the boundary between major subsystems, particularly where
async code meets synchronous ECS code. These live in `tests/` directories at
the crate or workspace level.

**Primary candidate: network roundtrip.**

The network module's sealed async boundary — where tokio tasks communicate with
Bevy via an `mpsc::unbounded_channel` — is the most architecturally significant
seam in the current codebase. A test that hosts a server, connects a client,
and verifies that the expected `NetEvent` values arrive through the channel
validates the entire async bridge without needing Bevy at all.

```rust
// modules/network/tests/roundtrip.rs

#[tokio::test]
async fn host_and_connect_produces_events() {
    let (tx, mut rx) = mpsc::unbounded_channel();

    // Start server
    let server_tx = tx.clone();
    tokio::spawn(server::run_server(0, server_tx)); // port 0 = OS-assigned

    // Wait for HostingStarted to learn the actual port
    let port = match rx.recv().await.unwrap() {
        NetEvent::HostingStarted { port } => port,
        other => panic!("expected HostingStarted, got {:?}", other),
    };

    // Connect client
    let addr = SocketAddr::from(([127, 0, 0, 1], port));
    let client_tx = tx.clone();
    tokio::spawn(client::run_client(addr, client_tx));

    // Verify connection events arrive
    // ...
}
```

This kind of test is high value but should be used sparingly. One or two
integration tests per major boundary is enough. If you find yourself writing
many integration tests, that's a signal that the boundary's API surface is too
complex or that more logic should be extracted into pure functions and tested
at tier 1.

### What not to test

- **Visual rendering.** Screenshot comparison and pixel-perfect tests are
  fragile, slow, and not worth the maintenance burden at this stage. Visual
  correctness is verified by running the game.
- **Bevy internals.** Do not test that `DespawnOnExit` despawns entities, that
  `Changed<T>` fires on mutation, or that `in_state()` gates systems correctly.
  These are framework guarantees.
- **Thin UI scaffolding.** Screen-building functions that just spawn nodes and
  text are not worth testing. They change frequently during development and
  correctness is immediately visible when you run the game.
- **Trivial getters and constructors.** If a function does nothing interesting,
  a test for it is just noise.


## Test organisation

```
modules/
  network/
    src/
      lib.rs               #[cfg(test)] — event draining, command dispatch
      config.rs            #[cfg(test)] — TLS config construction (if non-trivial)
    tests/
      roundtrip.rs         async host→connect via channels (tier 3)
  ui/
    src/
      button.rs            #[cfg(test)] — builder produces correct components
      theme.rs             #[cfg(test)] — default values
  tiles/     (future)
    src/
      lib.rs               #[cfg(test)] — grid ops, walkability, coordinate math
    tests/
      edge_cases.rs        large grids, boundary conditions
  things/    (future)
    src/
      lib.rs               #[cfg(test)] — WorldPosition arithmetic
src/
  creatures/ (future)
    movement.rs            #[cfg(test)] — resolve_movement pure function tests
  camera.rs  (future)
    (probably not worth testing — just lerp and offset)
tests/                     workspace-level integration tests (use sparingly)
```

Each workspace crate (`modules/*`) owns its own tests. The root crate's
`tests/` directory is reserved for full-stack scenarios that span multiple
modules. Keep that directory thin — most testing should happen at the crate
level.


## Dependencies

No additional crates are needed to start. The standard `#[test]` harness
combined with Bevy's `MinimalPlugins` covers everything in tiers 1 and 2.
The network module already depends on tokio, so `#[tokio::test]` is
available at no extra cost.

Crates to consider adding as `dev-dependencies` later:

| Crate | When to add | Purpose |
|-------|-------------|---------|
| `proptest` | When tiles and simulation systems land (L2-L3) | Property-based testing for invariants like "no movement ever clips through a wall" and "gas is conserved across atmos ticks". These systems have large input spaces where hand-written examples miss edge cases. |
| `criterion` | When simulation performance matters | Micro-benchmarks for hot loops — tilemap iteration, atmos tick, pathfinding. Only useful once there is meaningful work to measure. |
| `pretty_assertions` | Any time | Drop-in replacement for `assert_eq!` with coloured diffs. Small quality-of-life improvement for debugging test failures. Essentially free. |

Avoid mocking frameworks (`mockall`, etc.). The combination of pure function
extraction and Bevy's headless `App` eliminates most scenarios where mocking
would be tempting. If something is hard to test without mocks, that is a
signal to restructure the code, not to add a mocking layer.


## Continuous integration

### Pipeline design

A single GitHub Actions workflow triggered on pull requests and pushes to
`main`. Three jobs, in order:

1. **`fmt`** — `cargo fmt --workspace -- --check`. Catches formatting drift.
   Runs in seconds regardless of project size since it does not compile.
2. **`clippy`** — `cargo clippy --workspace -- -D warnings`. Catches lint
   issues and common mistakes. Requires a full compile but shares the
   cached `target/` directory.
3. **`test`** — `cargo test --workspace`. Runs all tests across all crates.
   Also requires compilation but benefits from the same cached state.

`clippy` and `test` can run in parallel since they both read from the same
compiled artifacts, but both depend on a successful `fmt` check (no point
compiling code that is not formatted).

### Workflow configuration

```yaml
concurrency:
  group: ci-${{ github.ref }}
  cancel-in-progress: true
```

This cancels in-flight CI runs when a new push arrives on the same branch.
During active development you often push multiple times in quick succession —
there is no value in completing a CI run for a commit that has already been
superseded.

### Self-hosted runners

Rust compilation is expensive. A clean Bevy build from scratch can take 15-20
minutes. GitHub's hosted runners provide 2,000 minutes per month on the free
tier, which can be exhausted in days during active development. Worse, hosted
runners are ephemeral — every run starts cold, and while `actions/cache` can
partially restore the `target/` directory, cache download and extraction still
adds minutes, and cache eviction means periodic full rebuilds.

Self-hosted runners solve both problems.

**Why self-hosted runners are the right choice for this project:**

- **Persistent `target/` directory.** The runner's working directory survives
  between jobs. After the first full build, subsequent runs only recompile
  changed crates. A typical incremental build drops from 15+ minutes to under
  a minute. This is the single biggest advantage — hosted runners can never
  fully replicate it.
- **Persistent cargo registry.** The `~/.cargo/registry` and `~/.cargo/git`
  directories stay populated. No downloading or extracting dependency sources
  on every run.
- **No minute limits.** You own the hardware. Run as many builds as you want
  without watching a quota.
- **No cache management.** No `actions/cache` keys to maintain, no 10 GB cache
  size limits, no eviction surprises. The disk just has the state it has.

**Hardware requirements:**

Minimal. Any modern machine with 8+ GB of RAM and an SSD will produce fast
incremental builds. The development machine itself can double as the runner
for a solo or small-team project — the runner agent is lightweight and only
activates when a job is queued.

**Setup:**

GitHub's self-hosted runner agent is a single binary. On Windows, it installs
as a service and polls GitHub for queued jobs. The setup wizard in
`Settings → Actions → Runners → New self-hosted runner` provides copy-pasteable
commands. Total setup time is under 5 minutes.

The workflow file targets the self-hosted runner with a `runs-on` label:

```yaml
runs-on: self-hosted
```

Or, if you want to distinguish between multiple machines or environments:

```yaml
runs-on: [self-hosted, windows, x64]
```

**Security considerations:**

If the repository is **public**, self-hosted runners are a security risk.
Anyone can open a pull request, and the PR's workflow runs on your machine with
access to the local filesystem. Malicious PRs could execute arbitrary code.

Mitigations for public repos:
- Require workflow approval for first-time contributors
  (`Settings → Actions → Fork pull request workflows`)
- Use `pull_request_target` instead of `pull_request` and gate on labels
- Run the runner in a VM or container with limited privileges
- Restrict the runner to a dedicated unprivileged user account

If the repository is **private**, these risks are minimal since only
collaborators can trigger workflows.

**Maintenance:**

The runner agent auto-updates. The main maintenance task is keeping the Rust
toolchain current (`rustup update`), which can be done as a step in the
workflow itself or on a schedule. Periodically clearing old build artifacts
(`cargo clean` on a cron) prevents unbounded disk usage, though in practice
incremental builds keep the `target/` directory from growing excessively.

### Caching strategy on self-hosted runners

Since the runner is persistent, there is no need for `actions/cache`. However,
two things are worth configuring:

1. **Fixed working directory.** Ensure the runner always checks out the
   repository to the same path so that `target/` accumulates correctly. This
   is the default behaviour for self-hosted runners — they reuse the workspace
   directory — but it is worth verifying.

2. **Periodic cleanup.** Over weeks or months, the `target/` directory can
   accumulate stale artifacts from deleted branches or renamed crates. A
   weekly `cargo clean` (via a scheduled workflow or cron job) keeps this in
   check without impacting day-to-day incremental builds.

### When to add the CI pipeline

The pipeline should be added before the first pull request on the playable
character plan. Even before any tests exist, the `fmt` and `clippy` jobs
provide immediate value as a quality gate. Tests are then added incrementally
as modules are built — starting with the network roundtrip test and tile grid
unit tests.
