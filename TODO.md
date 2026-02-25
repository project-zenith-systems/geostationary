# TODO

## SystemSet adoption

`ThingsSet` (in `modules/things/src/lib.rs`) introduces a named [`SystemSet`] so that
other modules can declare explicit ordering constraints against things systems without
coupling to private function names.  The same pattern could benefit other modules where
cross-module system ordering matters (e.g. atmospherics, tiles, souls).

Evaluate extending this approach throughout the codebase once more cross-module
ordering dependencies emerge.  Candidates:

- `TilesSet::SendOnConnect` — tiles catch-up send, ordered before `ThingsSet::HandleClientJoined`
- `AtmosSet::SendOnConnect` — atmospherics catch-up send, same ordering concern

## Bidirectional streams per module

Module streams currently use `open_uni()` with a `StreamDirection` enum. Refactor to
bidirectional (`open_bi()`) per module tag — this removes `StreamDirection` entirely
and supports both server→client snapshots and client→server mutations on a single
stream. The next plan (tilemap mutation replication) will need client→server writes on
the tiles stream, making this a natural prerequisite.

## Network async code simplification

Network client/server code (`modules/network/src/client.rs`, `server.rs`) has deeply
nested `tokio::select!` blocks, many cloned cancellation tokens, and verbose error
handling. A simplification pass would improve readability and maintainability. Not
urgent — the code is correct — but the complexity ceiling will rise as more stream
types are added.

## Server side and client side config

The server should have its config separated from the client config and the client config 
should only contain fields that are irrelevant to the server. The server might also
want to sync specific config settings to the client.

## Module coordination design pass (next arc)

The current coordination patterns — `ModuleReadySent` events, one-frame `StreamReady`
deferral, `ThingsSet::HandleClientJoined` ordering — were discovered ad-hoc during
implementation. Each uses a slightly different mechanism. As more modules gain
bidirectional streams (tilemap mutations, atmos updates, items), the coordination
surface will grow.

The next arc should include a design pass that:

- Documents the full game-loop timeline: when network events drain, when state
  transitions fire, when module systems run, when stream writes flush
- Establishes a single convention for "module X is ready" signalling
- Defines how cross-module ordering constraints are declared (SystemSets, message
  ordering, or something else) — rather than inventing a new pattern per plan
- Produces a reference doc that future plans can point to for scheduling decisions

This is not a standalone cleanup task — it should be driven by the next plan's concrete
coordination needs and then generalised.
