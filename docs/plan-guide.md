# How to Write a Plan

This document codifies the conventions used for planning in Geostationary.
A **plan** is a vertical cut through the layer stack — the minimum viable
implementation of a feature that proves the architecture works end-to-end.

## Document location

Plans live in `docs/plans/` and are named `<short-name>.md`.

## Structure

Every plan follows the same skeleton:

```
# Plan: <Name>

> **Stage goal:** <2-3 sentences. What this plan proves. Written in
> present tense as if describing the finished state.>

## What "done" looks like

<Numbered list of observable outcomes. A tester could read this list
and verify each point without looking at source code.>

## Strategy

<Prose explanation of the overall approach — why this order, what
principles guide the work. Not implementation detail; this is the
reasoning layer.>

### Layer participation

<Table mapping each touched layer/module to its scope in this plan.>

| Layer | Module | Plan scope |
|-------|--------|------------|
| L0    | `foo`  | What this module does in this plan |

### Not in this plan

<Explicit list of related concerns that are deliberately excluded.
This prevents scope creep and makes review faster.>

### Module placement

<Where new code goes — workspace crates in `modules/`, game systems
in `src/`. Include a directory tree snippet if helpful.>

### Design sections

<One subsection per significant design decision. Name them after the
concern they address (e.g., "Movement design", "Camera design",
"Collision strategy"). Each should be short — a few paragraphs — and
explain the *what* and *why*, not line-by-line code.>

## Post-mortem

<Filled in after the plan ships. Left as a placeholder in the
initial plan.>
```

## Writing principles

1. **Observable outcomes over implementation detail.** "What done looks
   like" describes behaviour a human can verify, not code structure.

2. **Scope by exclusion.** Explicitly list what is *not* in the plan.
   This is more useful than listing what is, because it prevents the
   slow expansion of "while we're here" additions.

3. **One design section per decision.** If a choice needs explaining,
   give it a heading. If it doesn't, a sentence in the strategy section
   is enough.

4. **Layer participation table is mandatory.** It forces you to think
   about which layers you're touching and whether you're violating the
   downward dependency rule.

5. **Module placement is mandatory.** New code needs a home before it
   gets written. Deciding workspace crate vs `src/` module up front
   prevents mid-implementation reorganisation.

6. **Post-mortem is mandatory.** Even if the plan goes perfectly, the
   post-mortem records what shipped, what deviated from the plan, what
   went wrong, and what to do differently. Future plans reference
   past post-mortems.

## Post-mortem structure

When filling in the post-mortem after a plan ships, use these sections:

- **Outcome** — One paragraph summary: did it ship what it promised?
- **What shipped beyond the plan** — Table of unplanned additions and
  why they were worth doing.
- **Deviations from plan** — Bullet points. What changed from the
  original plan and why.
- **Hurdles** — Numbered list of problems encountered, how they were
  solved, and what lesson each one taught.
- **Remaining open issues** — Table of issues that were discovered but
  not fixed in this plan.
- **What went well** — Bullet points.
- **What to do differently next time** — Bullet points.

## Branching convention

Plans use a two-tier branch structure to keep `main` clean. Individual task
commits accumulate on a **plan branch**; `main` only receives the finished
feature as a single squash-merge.

### Lifecycle

```
main ← plan PR (from plan-<name> branch, includes TODO.md)
  │      ↓ on merge, GitHub Actions creates plan/<name>
  │
  └─ plan/<name>
       ← task PR (from task/<issue#>-<short-desc>)
       ← task PR
       ← …
       ← post-mortem commit
       │
       └─ ONE squash-merge PR into main
```

1. The plan is written on a `plan-<name>` branch and PR'd into `main`.
2. When that PR merges, the `create-plan-branch` workflow automatically
   creates `plan/<name>` from `main` at the merge commit.
3. Each task is implemented on its own branch (e.g.
   `task/42-add-tile-colliders`) and PR'd into **the plan branch**, not
   `main`.
4. When all tasks are done and the post-mortem is written, one final PR
   squash-merges `plan/<name>` into `main`.

### Why squash-merge?

`main` reads as a sequence of complete features. Individual task commits
are preserved on the plan branch for archaeology but don't clutter the
main history. The squash-merge commit message should reference the plan
file (e.g. `Plan: Physics Foundation (docs/plans/physics-foundation.md)`).

## Task breakdown for TODO.md

After the plan is approved, break it into tasks in a `TODO.md` file at the
repository root. A GitHub Actions workflow converts each entry into a labeled
issue automatically (see `README.md` for format details).

### Principles

1. **One module per task.** Each task should touch one workspace crate or one
   `src/` module wherever possible. When two modules must change atomically
   (e.g., a spawn site and the system that reads the spawned components), group
   them into one task and name both files explicitly.

2. **No overlap.** If two tasks could both plausibly include the same change,
   one of them is scoped wrong. Every line of code should belong to exactly
   one task.

3. **Unambiguous completion criteria.** A task is done when a specific,
   observable thing is true. "Add colliders to walls" is good. "Improve
   physics" is not. Name the files being touched, the components being added,
   and the behaviour being removed or introduced.

4. **Dependency order.** Tasks are listed in implementation order. Each task
   states its dependencies ("depends on X") so the sequence is clear. A task
   should be implementable and testable using only the work from prior tasks.

5. **Link to the plan.** Every task description must include a link back to
   the plan file for cross-reference. Use the format:
   `**Plan:** [docs/plans/<name>.md](docs/plans/<name>.md)`

6. **Use the headers-with-description format.** Each `## ` heading becomes
   the issue title. The body below it should include:
   - What files/modules are touched
   - Bullet list of concrete changes
   - What is explicitly *not* included (if ambiguity is likely)
   - Link to the plan doc (see above)

7. **PRs target the plan branch.** Task PRs merge into `plan/<name>`,
   never directly into `main`. The plan branch is created automatically
   when the plan PR merges.

### How to decompose a plan

Walk the layer participation table bottom-up:

1. Start with new modules (they have no dependents yet).
2. Next, modules that gain new dependencies on the new module.
3. Then, modules that change behaviour based on the new capabilities.
4. Finally, integration or verification tasks if needed.

Each row in the participation table typically maps to one task. If a row
is trivial (adding a single component to a spawn call), it can be folded
into an adjacent task that touches the same file. If a row is large
(rewriting a system + changing a spawn site), it may need to stay as one
task if the changes are atomic, or be split if they can be tested
independently.

## Tips

- Keep the plan as short as practical. As a rule of thumb, if it grows much
  beyond ~200 lines before the post-mortem section, consider whether the plan
  should be split or simplified.
- Write the plan before writing code. Resist the urge to prototype
  first and document later — the plan is a thinking tool, not
  paperwork.
- Reference architecture docs by link, don't duplicate them. The plan
  says what this plan does within the architecture; the architecture
  docs say what the architecture is.
- Use the previous plan's post-mortem "what to do differently" section
  as a checklist for the new plan.
