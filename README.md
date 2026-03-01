# Geostationary

A round-based multiplayer space station simulation built with [Bevy](https://bevyengine.org/). Players are assigned roles on a station and must keep it running while hidden antagonists work to undermine, subvert, or destroy it.

See [docs/architecture.md](docs/architecture.md) for the full systems architecture.

## Project Structure

The workspace is split into two binaries and a set of shared modules:

```
bins/
  client/     — full game client with rendering, UI, input
  server/     — headless dedicated server (no windowing/rendering deps)
  shared/     — library used by both: AppState, config, ServerPlugin, WorldSetupPlugin
modules/      — gameplay crates (network, physics, tiles, things, etc.)
```

### Building

```sh
# Client (full game)
cargo run -p geostationary

# Dedicated server (headless)
cargo run -p geostationary-server

# Check server compiles
cargo check -p geostationary-server

# Run all tests
cargo test --workspace
```

## TODO.md

To batch-create GitHub issues, add a `TODO.md` file to the repository root and push to `main`. A workflow will automatically convert each entry into a labeled issue and remove the file.

Two formats are supported:

### Headers with descriptions

Each `## ` heading becomes an issue title. Lines below it become the issue body.

```markdown
## Implement feature X

Design doc is in docs/feature-x.md.
Should support both A and B.

## Fix bug in module Y

Crashes on empty input, see issue #12 for context.
```

### Bullet points

Each `- [ ] ` or `- ` line becomes a title-only issue.

```markdown
- [ ] Add mermaid diagrams to architecture docs
- [ ] Set up CI pipeline
- Investigate networking crate options
```

Both formats can be mixed in the same file. All created issues are tagged with the `TODO.md` label.
