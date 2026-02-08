# TODO

## Config management library

Add a configuration system to manage game settings (network port, graphics options, keybindings, etc.) via a config file and/or Bevy resource. Currently, values like the default port (`7777`) are hard-coded as constants. A config management library would provide a single source of truth for all tuneable parameters, support loading from disk (e.g., TOML/RON), and allow runtime overrides. Evaluate options like `bevy-settings`, `config-rs`, or a lightweight custom `Ron`-based loader.
