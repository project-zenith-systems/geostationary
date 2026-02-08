# TODO

## Config management library

Add a configuration system to manage game settings (network port, graphics options, keybindings, etc.) via a config file and/or Bevy resource. Currently, values like the default port (`7777`) are hard-coded as constants. A config management library would provide a single source of truth for all tuneable parameters, support loading from disk (e.g., TOML/RON), and allow runtime overrides. Evaluate options like `bevy-settings`, `config-rs`, or a lightweight custom `Ron`-based loader.

## Cap per-frame network event drain

`drain_net_events` currently drains the entire unbounded channel in a tight loop each frame. Once per-packet or per-tick events are added, this could stall the frame. Add a `MAX_NET_EVENTS_PER_FRAME` cap and log a warning when it's hit.

## Network task cancellation and lifecycle management

The network layer spawns long-lived server/client tasks but keeps no handles or cancellation tokens. Once `HostLocal` is issued the server loops forever, keeping the port bound even after state transitions. Track `JoinHandle`s and/or `CancellationToken`s in a resource and add `NetCommand` variants like `StopHosting` / `Disconnect` to cleanly shut down endpoints. This also prevents duplicate hosting if the user triggers Play multiple times.

## Configurable TLS server name for non-local connections

`NetCommand::Connect` hard-codes the TLS SNI to `"localhost"`. Once non-local connections and proper certificate verification are added, include a `server_name` field in the connect command and pass it to `endpoint.connect()`.
