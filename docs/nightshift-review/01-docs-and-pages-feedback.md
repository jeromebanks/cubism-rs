# Astra — making the Nightshift docs readable by a human

The content is strong and unusually honest. The packaging is optimized for
fidelity to a prior artifact, not for a person trying to decide something. The
notes below are about packaging only; the design critique is separate.

## 1. The headline problem is `docs/`, not `nightshift-design/`

`docs/` now holds three layers with nothing telling a reader which is live:

| Layer | Files | Status |
|---|---|---|
| `nightshift-design/` | 9 md + 9 html | Proposed future product |
| `docs/sdlc/` | `design.md`, `HUMAN_TUTORIAL.md`, `overview-slides.html`, manifest example | Current intended process |
| `TIMESERIES_*` at `docs/` root | ~45 files, ~700 KB | `SDLC.md` calls these "historical evidence, not active instructions" |

The third layer dominates the directory listing by file count. A human opening
`docs/` sees forty phase handoffs before anything current. Whatever else
changes, **move the retired time-series handoffs to `docs/history/timeseries/`
with a one-paragraph `README.md` explaining they are evidence, not
instructions.** That single move does more for comprehension than any edit
inside `nightshift-design/`.

Then add a `docs/README.md` that says, in five lines, which layer is
authoritative for what.

## 2. The reading guide navigates by a coordinate system that no longer exists

`README.md` offers two tables. The first maps documents to "Original sections
1–4." The second lists all 21 section titles pointing into 8 files. Both
describe *where content went during repackaging*. Neither tells a reader where
to start or what they will be able to decide when they finish.

That is a provenance manifest. Keep it — under a `## Provenance` heading near
the bottom, next to the evidence baseline, where it belongs. Replace the top of
the README with a route:

```
New here?          index.html — 10 minutes, the whole thesis
Deciding whether   01 (§1–4) then 08 (§19–21) — the pitch and the roadmap
  to fund this
Building Stage 0   01 §2 (defect table), 08 §19 Stage 0, 02 §6 (slice contract)
Security review    05, 07 §16
Reference          03 (state machines), 05 §11 (policy schema)
```

Five lines, and every reader self-routes. The section map's job is to prove
nothing was lost; the route's job is to get someone reading.

## 3. Chapter 03 is a reference appendix wearing a chapter number

`03-durable-state-machines.md` is ~2,600 words and roughly **80 transitions** —
S1–S12, A1–A11, Q1–Q10, G1–G14, M1–M10, F1–F9, I1–I8. Every one has an actor,
preconditions, a durable event, a timeout profile, a recovery rule, and an
evidence column.

Nobody reads that as chapter 3 of 8. Put it at the back as **Appendix A: State
transition reference**, and leave a 300-word section in its place covering the
three ideas a reader actually needs to carry forward:

1. PostgreSQL is the authority; unlisted transitions are rejected.
2. Terminal history is never rewritten — replacement, not mutation.
3. `T/R` means an attempt can die and its parent can spawn a successor without
   the failure being renamed away.

Those three sentences are what a reader remembers a week later. The table is
what an implementer greps.

The same applies, less severely, to 07's failure-behavior and delivery-guarantee
tables.

## 4. Replace the per-page hedge with the fact

Every chapter carries:

> Repository findings refer to the inspected checkpoint, not live project state.

This tells the reader something might have changed without telling them what. I
checked. **Nothing changed.** All five confirmed defects are present verbatim on
`docs/nightshift-design-draft` today:

(Status as reviewed on 2026-09-15. Four of the five have since been fixed in
this working tree — see `03-cubism-transition-plan.md` Step 1 — but they were
all present, verbatim, at review time.)

| Defect | Location as reviewed | Status |
|---|---|---|
| Applied merge raises `NameError` | `scripts/sdlc.py:686` — `if add or remove:` (both are locals of `command_set_gate` at :715) | present → **fixed** |
| Self-authored receipt satisfies the gate | `scripts/sdlc.py:467` — eligibility requires only non-empty `author`/`reviewer` | present → **fixed** |
| Later failure does not invalidate earlier pass | `scripts/sdlc.py:467` — any matching pass wins | present → **fixed** |
| Feedback label bypasses both parent gates | `scripts/sdlc.py:372` | present → **fixed** |
| `issue/N` push is not an exclusive claim | `scripts/sdlc.py:594` | present → **open** (needs a design decision, not a patch) |

A dated "verified still present on `<sha>`" line is worth more than an
open-ended hedge, and it gives Stage 0 a real starting line.

One sharpening while you are in there: the `NameError` at :686 guards the
`gh pr merge` call itself and sits *after* the `--apply` dry-run early return.
So the documented auto-merge path has **never completed a single merge**. PRs
#58, #63, and #64 merged by some other route. Section 1 §2 describes this as "an
otherwise eligible applied merge crashes"; it is stronger and more useful to say
the deterministic merge gate has zero successful executions in its lifetime.

## 5. Nothing answers "what do I do on a Tuesday?"

The package explains what Nightshift *is* with real rigor and never shows a
person's day. `docs/sdlc/HUMAN_TUTORIAL.md` does exactly that for the current
process — it is the most readable document in the repository — and it is in a
different folder, unlinked from the Nightshift README, and describes a process
that isn't on `main`.

Add one page: **"A week under Nightshift."** Monday you approve an epic. Tuesday
through Thursday you do nothing and the cockpit shows four slices merging.
Friday a milestone report needs twenty minutes and a yes/no. Concrete hours,
concrete screens, concrete decisions. That page will sell this better than
sections 1–21 combined, and it is the one thing a reader cannot reconstruct from
what exists.

## 6. Question whether the HTML editions earn their machinery

Current cost: every document exists twice (md + html), plus `reader.css`,
`reader.js`, `index.html`, a pinned-renderer venv, `render-nightshift-docs.py`
with a `--check` reproducibility mode and a `--patch` mode, a requirements file,
a `.nojekyll`, a `nightshift-pages` branch, and a `git subtree split` publish
step — for nine documents. The README spends its final third on build and
publish instructions, which is the last thing a first-time reader needs.

`index.html` is genuinely good and worth all of it. The eight full reading
editions are a straight duplication of Markdown that GitHub already renders
well, and the duplication creates a real hazard: source and HTML can drift, and
`--check` only catches it if someone remembers to run it.

Two options, either defensible:

- **Keep `index.html`, drop the eight chapter HTML editions.** Pages publishes
  one hand-built overview that links to the Markdown on GitHub. Removes the
  venv, the digest embedding, and the drift risk entirely.
- **Keep all of it, but move build/publish instructions out of `README.md`**
  into `docs/nightshift-review/PUBLISHING.md` or a comment header in the
  renderer, and add a CI check so drift fails loudly rather than silently.

Also: the README's rebuild snippet hardcodes `/private/tmp/nightshift-docs-venv`
and then tells the reader to replace it. Just use `.venv-docs/` and add it to
`.gitignore`.

## 7. Smaller things

- **Move the evidence baseline table up.** Branch, commit, date, and "eight unit
  tests passed" is the credibility anchor for everything that follows. It is
  currently below the reading guide and the 21-entry section map.
- **The "Preservation and scope" section is addressed to the packaging
  reviewer**, not the reader. Fold it into `## Provenance` with the section map.
- **Mermaid requires a capable viewer**, per the README. The architecture
  diagram in 02 is one of the most useful things in the package and is invisible
  in most Markdown viewers. If the HTML editions survive, this is their best
  justification — say so explicitly rather than burying it in a note about
  external dependencies.
- **`docs/project-status.html` is a committed snapshot dated
  `2026-09-05T00:24:30+00:00`** — ten days stale as of this review, and it
  renders as authoritative. Either regenerate it as part of publishing or stamp
  the age prominently at the top.
- **Prose register.** The document reads as a legal instrument: "A model may
  propose a transition. Deterministic services decide whether it is permitted."
  That precision is right for sections 3, 5, and 7. Sections 1 and 8 are the
  persuasion sections and should be allowed to breathe — they are the only ones
  most readers will finish.
