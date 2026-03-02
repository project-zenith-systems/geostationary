## Add custom NetworkReceive and NetworkSend schedules

Replace the current `NetworkSet::Receive` / `NetworkSet::Send` system sets
(which run inside `PreUpdate`) with dedicated Bevy schedules that run at
well-defined points in the frame. This would give modules a clearer
contract: "your systems in `NetworkReceive` drain inbound messages; your
systems in `NetworkSend` flush outbound messages" — without sharing a
schedule with unrelated `PreUpdate` work.

See `docs/module-coordination.md` for the current game-loop timeline.
