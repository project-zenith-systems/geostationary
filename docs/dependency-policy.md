# Dependency Policy

## The Downward Rule

All dependencies flow downward. A module at layer N may depend on:

1. Any module at layer 0 through N-1
2. External crates and libraries, subject to the gravity principle below

A module may **never** depend on anything at layer N+1 or above.

## The Gravity Principle

External dependencies have **gravity** - they are pulled toward the bottom of
the stack. The lower a dependency sits, the more modules benefit from it, and
the fewer integration surfaces need to be maintained.

```
  L7  ·                          Minimal external dependencies.
  L6  ·                          Rely on engine abstractions.
  L5  · ·
  L4  · · ·
 ━━━━━━━━━━━━━━━━━━━━━━━  compile horizon  ━━━
  L3  · · · · ·                  Moderate external dependencies.
  L2  · · · · · ·                Wrapped and re-exported upward.
  L1  · · · · · · · ·
  L0  · · · · · · · · · · ·      Heaviest external dependency surface.
```

As a guideline:

- **L0-L1** may freely depend on external crates (networking, physics,
  input, rendering, math, allocation)
- **L2-L3** should wrap external crates behind engine-owned traits and types
  (e.g. atmospherics may use an external math library internally, but the
  API it exposes to L3 and above uses engine-owned types)
- **L4-L5** should depend almost exclusively on engine APIs exposed by the
  L3 compile horizon (souls, surgery, weapons, roles - all talk to the
  substrate through the scripting bridge, not through vendor libraries)
- **L6-L7** should have zero or near-zero direct external dependencies
  (menus, camera, auth all operate through engine abstractions)

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
