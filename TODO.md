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
