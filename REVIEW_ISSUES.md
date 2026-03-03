# Networked Items Branch — Review Issues

Temporary file tracking issues found during review of `plan/networked-items`.
Delete after issues are resolved or converted to GitHub issues.

## High

### ~~1. Unsafe `camera_q.single()` in resolve_world_hits~~ FIXED

Early-returns when no unique `Camera3d` exists. Tests updated to spawn a
camera entity.

## Medium

### ~~2. `handle_menu_selection` unordered relative to `build_context_menu`~~ FIXED

Added `.after(build_context_menu)` to `handle_menu_selection` in plugin
`build()`.

### ~~3. Non-physical drop path in items~~ FIXED

Server-side pickup already rejects non-physical items. Client-side
`Dropped` handler now warns and skips instead of silently placing an inert
entity.

### ~~4. No duplicate guard in `Container::insert`~~ FIXED

`insert` now checks for existing presence and returns `None` if the entity
is already in a slot.

## Low / Informational

### 5. Broadcast systems in Update, not PostUpdate
**Files:** modules/things/src/lib.rs, modules/atmospherics/src/lib.rs

`broadcast_state`, `broadcast_item_event` run in `Update`;
module-coordination.md recommends `PostUpdate`. Already tracked in TODO.md.

### ~~6. Repeated `NetIdIndex` check in `dispatch_interaction`~~ FIXED

Deduplicated warning messages across item match arms. Hoisting above the
match isn't practical since `TileToggle` doesn't need `NetIdIndex`.

### ~~7. `Item` without `NetId` silently skipped in context menu~~ FIXED

`build_context_menu` now warns when an `Item` entity has no `NetId`.
