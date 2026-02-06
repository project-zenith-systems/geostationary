# Dependency Policy

## The Downward Rule

All dependencies flow downward. A module at layer N may depend on:

1. Any module at layer 0 through N-1
2. External crates and libraries, subject to the gravity principle below

A module may **never** depend on anything at layer N+1 or above.

## External Dependencies

Most modules will not need external crate dependencies beyond Bevy. External
dependencies are the exception, not the rule. Where one is needed, it should
sit as low in the stack as practical so the layers above don't need to know
about it.

## Upward Communication

When a lower layer needs to notify or influence a higher layer, it must do so
through one of these sanctioned patterns:

| Pattern              | Example |
|----------------------|---------|
| **Events**           | L2 `atmospherics` fires a pressure-change event; L6 `menu` reads it to update a HUD warning |
| **Trait objects**     | L2 `abilities` defines an `Ability` trait; L4 `genetics` registers specific ability implementations |
| **Callbacks**        | L3 `electronics` accepts a callback for hacking outcomes; L4 `machines` registers machine-specific hacking results |
| **Resources**        | L3 `station` writes alert-level state; L5 `comms` reads it to determine available announcement channels |

Direct `use` imports from a higher layer are a build error and a design error.

## Compile Horizon Boundary

The boundary between L3 and L4 is the most critical interface in the
architecture. L3 must expose a **stable, well-documented API surface** that the
scripting runtime can bind against. Changes to this surface ripple through the
entire canopy.

Guidelines for this boundary are documented in
[L3 - Core](layers/L3-core.md) and [L4 - Mechanics](layers/L4-mechanics.md).
