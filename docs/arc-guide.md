# How to Write an Arc

An **arc** is a sequence of plans that share a strategic goal. Where a plan
is a vertical cut through the layer stack, an arc is the narrative thread
connecting several cuts into a coherent capability.

## Why arcs exist

A plan delivers a complete, testable feature — but features build on each
other. Without an arc, each plan is scoped in a vacuum and the connections
between them are implicit. An arc makes the sequence explicit so that each
plan knows what comes before it and what it's setting up for next.

## Document location

Arcs live in `docs/arcs/` and are named `<short-name>.md`.

## Structure

Every arc follows this skeleton:

```
# Arc: <Name>

> **Goal:** <1-2 sentences. What capability exists when the arc is complete.
> Written as an observable statement, not a task.>

## Plans

<Ordered list of plans. Each entry names the plan, summarises its
deliverable, and lists the key technical work required.>

1. **Plan name** — What it delivers. Requires: key technical work
   (new modules, protocol changes, systems to add).
2. **Plan name** — What it delivers. Requires: ...
3. ...

## Not in this arc

<Explicit list of related concerns that are deliberately excluded from the
entire arc, not just individual plans. Prevents the arc from growing
unboundedly.>
```

## Writing principles

1. **Goal is observable.** "Players can pick up items and see each other do
   it" is good. "Implement the items system" is not. The goal describes what
   a playtester would see, not what a developer would build.

2. **Each plan is a full vertical cut.** A plan delivers a complete,
   playtester-visible feature — not a foundation, not a refactor, not a
   single module. If a plan only makes sense as setup for the next plan,
   fold it in. "Items on the floor" is a task; "pick up and drop items
   visible to all clients" is a plan. Think bigger.

3. **Spell out what each plan requires.** After the deliverable sentence,
   list the key technical work: new modules, protocol messages, systems,
   data structures. This is not full design (that lives in the plan doc)
   but enough that the plan author knows what they're signing up for and
   doesn't miss anything. "Replicate atmos" is vague; "ServerMessage
   variants for atmos grid snapshots, client-side rendering without local
   simulation" is actionable.

4. **Arcs can overlap in time.** Two arcs can have plans interleaved. The
   arc document tracks its own sequence; the actual implementation order
   across arcs is decided at planning time.

5. **Keep it short.** An arc is a table of contents with a paragraph of
   rationale, not a design document. The design lives in each plan.

6. **Retrospective after the last plan ships.** Add a one-paragraph
   retrospective at the bottom: did the arc deliver what it promised? Did
   the sequencing work? What would you change?

## Relationship to plans

```
Arc (strategic sequence)
 └── Plan 1 (vertical cut — has its own branch, TODO.md, post-mortem)
 └── Plan 2
 └── Plan 3
```

- An arc does **not** have its own branch. Only plans have branches.
- A plan references its arc in the strategy section ("This is plan 2 of the
  Items arc") but is otherwise self-contained.
- A plan can belong to at most one arc. If a plan serves two arcs, one of
  the arcs is scoped wrong.

## Tips

- Write the arc before writing the first plan in it. The arc's sequencing
  informs each plan's "not in this plan" section — deferred work is easier
  to justify when you can point to a later plan that will handle it.
- Re-read the previous arc's retrospective before starting a new arc. The
  same sequencing mistakes tend to recur.
- It's fine to revise the arc after a plan ships. If plan 2's post-mortem
  reveals that plan 3 needs a different scope, update the arc document.
  The arc is a living document, not a contract.
