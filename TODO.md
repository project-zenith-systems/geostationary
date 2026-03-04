## Add custom NetworkReceive and NetworkSend schedules

Replace the current `NetworkSet::Receive` / `NetworkSet::Send` system sets
(which run inside `PreUpdate`) with dedicated Bevy schedules that run at
well-defined points in the frame. This would give modules a clearer
contract: "your systems in `NetworkReceive` drain inbound messages; your
systems in `NetworkSend` flush outbound messages" — without sharing a
schedule with unrelated `PreUpdate` work.

See `docs/module-coordination.md` for the current game-loop timeline.

## Fix item drop position

Dropped items currently fall from the hand with gravity instead of
teleporting to the clicked world location. The `ItemDropRequest.drop_position`
is set correctly by `dispatch_interaction`, but the item appears to spawn
at the hand offset rather than the target position. Investigate whether
the `ChildOf` removal and `Transform` update are racing with physics.

## Move broadcast systems from Update to PostUpdate

`broadcast_state`, `broadcast_item_event` (items), and `broadcast_gas_grid`
(atmospherics) currently run in `Update`. They should run in `PostUpdate` so
that all gameplay systems have committed their changes before the state is
serialised and sent to clients. See the PostUpdate rationale in
`docs/module-coordination.md`.
