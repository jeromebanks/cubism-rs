# Time-Series Phase 19 Handoff

Date: 2026-08-15

Branch: `feature/timeseries-phase-0a`

Status: **Landed Milestone 9** (`docs/TIMESERIES_ROADMAP.md`'s "Phase 5
Milestones" section) — `ResolutionPlan`, non-overlapping resolution-segment
selection over a `TemporalQuery`/`TemporalSpec` pair, added to the existing
`crates/cubism-datafusion/src/range_query.rs` module. Single-resolution
decomposition only (partial head / aligned interior / partial tail, 1-3
segments); cross-resolution decomposition explicitly deferred (see below).
Seven new unit tests. Milestone 9 flipped to `Done` in the roadmap; no new
GitHub issues filed.

(Despite the filename, this doc documents a session slice, not "Phase 19"
of the implementation plan — same convention every prior handoff in this
series has used: plan Phase 5 is DataFusion range queries and serving; the
plan's Phase 4 (`docs/TIMESERIES_IMPLEMENTATION_PLAN.md:582-670`) has every
`docs/TIMESERIES_ROADMAP.md` Phase-4 milestone done but is not itself fully
done — see that roadmap's "Phase 4 done" section, unchanged by this
session, for what remains.)

**Superseded by:** (none yet — this is the latest handoff)

## What this session built

Read `docs/TIMESERIES_PHASE_18_HANDOFF.md` and confirmed the branch in sync
with `origin` (`git rev-list --left-right --count` reported `0 0`), then
listed all open issues (`#1`-`#18`, unchanged from Phase 18). Phase 18's
deferred item 1 named Milestone 9 as the natural next slice, and the
roadmap confirmed it as the first `Not started` Phase 5 milestone with
"Depends on: Milestone 8," now done. Read its full spec
(`docs/TIMESERIES_ROADMAP.md:606-621`, pre-edit) and confirmed it with the
advisor before writing anything, per the skill's step 1.

The advisor confirmed Milestone 9 as the right slice and, before any code
was written, flagged that `TemporalSpec::validate`'s body
(`crates/cubism-core/src/temporal.rs:606-649`) had only been read by
signature in Phase 18, not by body — its rules (rollups must be an integer
multiple of and strictly coarser than the base resolution; `Calendar`
base/rollups are rejected) are enforced only there, not by
`TemporalQuery::new`, so `ResolutionPlan` cannot assume a spec reaching it
has already satisfied them. That read confirmed the divisibility/coarseness
guarantee exists in `validate()` but is genuinely not enforced at
construction — which is what motivated the scope fence below.

The advisor's scope fence, applied as written: do **not** build the classic
coarse-interior/fine-edges *multi-resolution* decomposition Milestone 9's
"resolution choice never overlaps or double-counts" text could be read as
inviting, since it would depend on that unenforced divisibility guarantee —
a false premise this milestone shouldn't build on. Instead, `ResolutionPlan`
picks **one** resolution (the query's `Some`, or an auto-selected one for
`None`) and decomposes `[start, end)` into at most three contiguous
segments at that single resolution: an optional partial head, an optional
aligned interior (kept as one segment even when it spans many whole
buckets, not split per-bucket), and an optional partial tail. Non-overlap
and full coverage follow directly from building the segments off one
shared `interior_start`/`interior_end` boundary pair, not from any
cross-resolution reasoning — stated explicitly in the module doc comment,
this series' convention for a claim a new
module is making.

Two more decisions came out of the same advisor call:

- **`Resolution::Calendar` is rejected explicitly**, not filtered out
  silently. Milestone 8 deliberately let a `Calendar` base/rollup pass its
  own membership check (`TemporalQuery::new` only checks equality/
  containment, not fixed-vs-calendar), so a `TemporalQuery` carrying
  `Some(Resolution::Calendar(_))`, or built against a spec whose
  `base_resolution` is `Calendar`, can legally reach `ResolutionPlan::new`.
  No calendar equivalent of `FixedResolution::bucket` exists anywhere in
  this codebase, so both the explicit-selection and the auto-selection
  paths reject `Calendar` via a shared `as_fixed` helper
  (`range_query.rs:197-206`) rather than silently producing a plan that
  doesn't cover the range. Two tests cover this
  (`resolution_plan_rejects_calendar_resolution`,
  `resolution_plan_auto_select_rejects_calendar_base` — the name
  deliberately doesn't mention "no fixed rollup": `auto_select_resolution`
  rejects a Calendar base via `as_fixed` before it ever inspects rollups,
  so an empty rollup list isn't what triggers the rejection; a Calendar
  base with a `Fixed` rollup present would reject identically).
- **Auto-selection rule, since the plan text doesn't dictate one**: the
  coarsest `Fixed` candidate (the spec's base resolution or one of its
  rollups) whose width fits within the query's duration, falling back to
  the base resolution when no rollup fits. Implemented in
  `auto_select_resolution` (`range_query.rs:217-232`). Recorded as a
  decision in the roadmap's Milestone 9 entry, not left implicit in code —
  same treatment Milestone 8 gave its two deviations.

One implementation detail the advisor flagged ahead of time: `Resolution`
derives `PartialEq, Eq, Hash` but not `Ord` (`crates/cubism-core/src/temporal.rs:330`),
so picking a "coarsest" candidate has to compare on the inner
`FixedResolution` (which does derive `Ord`) via `.micros()`, not by calling
`.max()` on a `Vec<Resolution>` directly.

Per the advisor's process note, before treating anything as newly
fileable, the open-issue list (`#1`-`#18`) was checked against this
session's work: nothing in it covers `ResolutionPlan` territory, and #14
(reconciled last session) remains the retry-loop tracker with no new
overlap.

**A tooling issue found and corrected mid-session:** `cargo fmt -p
cubism-datafusion` (skill step 4 does not call for `cargo fmt` explicitly,
but it was run to format the new code before verification) reformatted six
files this session never touched (`build.rs`, `state_udaf.rs`, `udaf.rs`,
`udf.rs`, `temporal_build.rs`, `tests/iceberg_bridge.rs`) — 760 insertions
and 222 deletions (982 changed lines) of pure formatting churn across
those six files alone, from what should have been a single-file change.
(The initial `git diff --stat` total of 1,109 insertions/229 deletions
covered all ten touched files, including this milestone's own intended
`range_query.rs`/`lib.rs`/doc changes alongside the six-file churn — not
the churn by itself.) This is a local rustfmt/toolchain mismatch, not new
source drift;
it's the same class of problem `#3` ("rustfmt --edition 2024 --check does
not reproduce clean") already tracks, just surfaced by `cargo fmt -p`
running crate-wide instead of file-scoped. The six unrelated files were
reverted with `git checkout --` back to their committed state before
staging, and the full `cubism-datafusion` test+clippy pass was re-run
afterward to confirm nothing besides formatting had changed in them (see
"Verification performed"). No issue filed for this beyond `#3`, which
already covers the underlying rustfmt discrepancy; a future session
touching this repeatedly should format only the changed file(s)
(`rustfmt --edition 2024 <path>` — plain `rustfmt <path>` with no
`--edition` flag fails outright on this crate's let-chain syntax) rather
than `cargo fmt -p <crate>`.

Scope fence held per the advisor's confirmation: no `CoveragePlan`/
`is_exact`/`coverage`/`source_resolution` fields or logic — those are
Milestone 10's job, not this one's. `ResolutionSegment` carries only
`range: TimeRange` and `aligned: bool`.

Primary files changed:

- **`crates/cubism-datafusion/src/range_query.rs`** (modified, +313/-0
  net across the file): added `ResolutionSegment` (`:160-163`),
  `ResolutionPlan` (`:170-173`) and its `new` (`:179-192`), the `as_fixed`
  helper (`:197-206`), `auto_select_resolution` (`:217-232`), `decompose`
  (`:238-290`), plus a module doc comment addendum (`:36-68`) describing
  what Milestone 9 does and does not do, and seven new unit tests plus two
  shared test helpers (`:392-524`, tests themselves at `:429-524`).
- **`crates/cubism-datafusion/src/lib.rs`** (modified, +1 line): the
  Milestone 8 re-export line now also exports `ResolutionPlan`,
  `ResolutionSegment`.
- **`docs/TIMESERIES_ROADMAP.md`** (modified): Milestone 9's `**Status:**`
  flipped from `Not started` to `Done`, with the single-resolution-only
  deviation, the auto-selection rule, and landed line numbers recorded.
- **`docs/TIMESERIES_PHASE_18_HANDOFF.md`** (modified): added
  `**Superseded by:**` line.

No `Cargo.toml` change: `ResolutionPlan` only needs types already imported
by `range_query.rs` from `cubism-core`.

## What was actually verified

That `ResolutionPlan::new` correctly decomposes `[start, end)` into
non-overlapping, gap-free segments for four shapes: an interval exactly
aligned to one resolution bucket (one aligned segment,
`resolution_plan_aligned_interval_is_one_segment`); an interval spanning a
partial head, one full aligned interior bucket, and a partial tail
(`resolution_plan_unaligned_interval_has_head_interior_tail`); an interval
entirely inside a single bucket, narrower than the resolution itself, which
produces one partial segment with no aligned interior at all
(`resolution_plan_range_within_single_bucket_is_one_partial_segment`); and
that a shared `assert_exact_cover` helper's three checks (first segment
starts at the range start, last segment ends at the range end, each
adjacent pair is exactly contiguous with no gap or overlap) hold for each
of the five tests that construct a plan — the two Calendar-rejection tests
below build no plan to check, so `assert_exact_cover` does not run in
those two. That auto-selection (`resolution: None`) picks the coarsest
rollup that fits a 2-hour range
(`resolution_plan_auto_selects_coarsest_resolution_that_fits`) and falls
back to the base resolution when the only rollup is wider than a 30-minute
range (`resolution_plan_auto_falls_back_to_base_when_no_rollup_fits`). That
a `Calendar` resolution is rejected with `CubismError::Temporal` both when
requested explicitly against a spec whose base is `Calendar`
(`resolution_plan_rejects_calendar_resolution`) and when auto-selection has
a `Calendar` base with no `Fixed` rollup present
(`resolution_plan_auto_select_rejects_calendar_base` — the rejection is
actually triggered by the Calendar base alone, before rollups are ever
inspected; the empty rollup list in this particular test isn't load-bearing).
That the full step-4 verification battery — including `cargo clippy … -D
warnings` — is clean with this change in place, run fresh this session
(see "Tests" below), and re-run a second time after reverting the six
files an over-broad `cargo fmt -p` had touched, to confirm the revert
introduced no regression.

It does **not** prove: anything about cross-resolution decomposition
(serving part of a range from a coarser rollup and part from the base
resolution) — deliberately out of scope this milestone, per the scope
fence above. It does not prove `ResolutionPlan` behaves correctly when
`query.resolution` and `spec` are *inconsistent* with each other (e.g. a
`TemporalQuery` validated against one spec, then passed to
`ResolutionPlan::new` with a different spec) — the one test that does this
deliberately (`resolution_plan_rejects_calendar_resolution`) only exercises
the Calendar-rejection path, not general spec-mismatch behavior. It does
not prove anything about `is_exact`, `coverage`, or `source_resolution` —
those are Milestone 10's `CoveragePlan` fields. It does not exercise any
DataFusion execution path, `cubism-iceberg` I/O, or the
`AggregateState::decode`/`merge` machinery — Milestone 9, like Milestone 8,
is pure computation over window boundaries.

## GitHub issues touched

- No new issues filed. The `cargo fmt -p` over-formatting problem described
  above is a new *symptom* of `#3`'s already-tracked rustfmt/edition-2024
  discrepancy, not a new root cause — no separate issue filed for it, and
  no comment added to `#3` since this session didn't investigate the
  discrepancy itself, only worked around it by reverting and using a
  narrower `cargo fmt` scope going forward.
- No comments added to any other open issue; nothing Milestone 9 touches
  narrows or resolves any of `#1`-`#18`.

## Deferred / not done this session

1. **Milestone 10 (`CoveragePlan`/`SeriesResponse`)** — the roadmap's next
   Phase 5 milestone (`docs/TIMESERIES_ROADMAP.md:666-727`, post-edit:
   this session's Milestone 9 edit added lines above it, shifting it down
   from its pre-edit 623). Depends on Milestones 8 (done) and 9 (now
   done), plus the `cubism-iceberg` dependency Milestone 7 added. Natural
   next slice.
2. **Cross-resolution decomposition** (coarse interior / fine edges spanning
   multiple resolutions) — not filed as a separate issue; recorded in the
   roadmap's Milestone 9 entry as a deliberate scope cut. Revisit once a
   caller actually needs multi-resolution serving and once
   `TemporalSpec::validate`'s divisibility/coarseness guarantee can be
   assumed to hold for specs reaching this module (currently it's only
   enforced by `validate()`, which `ResolutionPlan::new` does not call).
3. **`cargo fmt -p <crate>` reformatting unrelated files** — worked around
   this session by reverting and re-verifying, but the underlying rustfmt/
   edition-2024 discrepancy `#3` tracks is unchanged. A future session
   should default to `rustfmt --edition 2024 <changed-file-path>` (plain
   `rustfmt <path>` with no `--edition` flag fails outright on this crate's
   let-chain syntax — confirmed this session) rather than
   `cargo fmt -p <crate>` in this repo until `#3` is resolved.
4. **#17** (append-committed-but-not-recorded recovery) — unchanged; still
   needs a design decision before it can be sized into a bounded milestone.
   Not re-read this session (Phase 18 already re-confirmed its state; no
   reason to expect it changed).
5. **#16** (event-time window identification + recompute-equality proof) —
   unchanged; still needs a decision on which crate closes it. Not touched
   by Milestone 9 (`range_query.rs` doesn't read or write anything from
   `cubism-iceberg`).
6. **#10** (real object store + Iceberg maintenance) — unchanged.
7. **#11/#12** — unchanged; not touched this session.
8. **Plan completion-criterion 774** (storage pruning across many windows)
   — unchanged; still gated on #8's SQL/pushdown half.
9. **`/api/series` (`crates/cubism-serve`) and plan-Phase 6** — unchanged;
   not reached by Milestones 8-10.
10. **`#14`** (retry-loop/connection-poisoning gap) — unchanged, still open;
    its own suggested next step (force a `SQLITE_BUSY`/`SQLITE_LOCKED`
    retry past `busy_timeout`) is still not implemented, a deliberately
    slow (>5s) test not added to keep the fast suite fast.

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log`
for the exact hash — this doc deliberately doesn't hardcode it, per this
series' established convention). That commit contains:

- New: `docs/TIMESERIES_PHASE_19_HANDOFF.md` (this file).
- Modified: `crates/cubism-datafusion/src/range_query.rs` (Milestone 9's
  `ResolutionPlan`/`ResolutionSegment`), `crates/cubism-datafusion/src/lib.rs`
  (re-export), `docs/TIMESERIES_ROADMAP.md` (Milestone 9 status),
  `docs/TIMESERIES_PHASE_18_HANDOFF.md` (added `**Superseded by:**` line).
- Untouched: `crates/cubism-core/src`, `crates/cubism-iceberg/src`,
  `crates/cubism-datafusion/src/{build,state_udaf,temporal_build,udaf,udf}.rs`,
  `crates/cubism-datafusion/tests/iceberg_bridge.rs`,
  `crates/cubism-serve/src`, `crates/cubism-cli/` (built and clippy-checked,
  not modified) — the last six of those were briefly reformatted by an
  over-broad `cargo fmt -p cubism-datafusion` mid-session and reverted with
  `git checkout --` before staging (see "What this session built").

Also present, deliberately uncommitted per prior-session convention:
`.serena/` (local tooling state), `examples/web_analytics_demo/events.csv`
(generated demo output).

## Tests (33 passed + 1 ignored in `cubism-iceberg`, unchanged; 42 passed in `cubism-datafusion`, up from 35; 182 passed / 2 ignored in workspace, up from 175)

All figures re-run fresh this session, not carried forward from Phase 18,
and re-run a second time after reverting the six unrelated files. `cargo
test -p cubism-iceberg` reports 33 passed, 1 ignored (6 suites) — identical
to Phase 18, expected since no source in that crate changed this session.
`cargo test -p cubism-datafusion` reports 42 passed (3 suites: 41 unit + 1
integration) — up from Phase 18's 35 by exactly the seven new
`ResolutionPlan` unit tests. `cargo test --workspace --exclude cubism-py`
reports 182 passed, 2 ignored (23 suites) — up from Phase 18's 175 passed
by exactly the same seven.

## Verification performed

```text
cargo test -p cubism-datafusion --lib range_query                        # 11 passed (4 Milestone 8 + 7 new)
cargo test -p cubism-iceberg                                              # 33 passed, 1 ignored (6 suites)
cargo test -p cubism-iceberg --test concurrency                           # 4 passed, 1 ignored
cargo test -p cubism-iceberg --test durability                            # 7 passed
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings     # clean
cargo build -p cubism-cli                                                 # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings        # clean
cargo test -p cubism-datafusion                                           # 42 passed (3 suites)
cargo clippy -p cubism-datafusion --all-targets --no-deps -- -D warnings  # clean (re-run after reverting the over-broad fmt)
cargo test --workspace --exclude cubism-py                                # 182 passed, 2 ignored (23 suites)
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
```

## Primary files

- [`../crates/cubism-datafusion/src/range_query.rs`](../crates/cubism-datafusion/src/range_query.rs)
  (Milestone 9's `ResolutionPlan`/`ResolutionSegment`)
- [`../crates/cubism-datafusion/src/lib.rs`](../crates/cubism-datafusion/src/lib.rs)
  (re-export, line 17)
- [`../docs/TIMESERIES_ROADMAP.md`](TIMESERIES_ROADMAP.md) (Milestone 9,
  lines 606-664 post-edit; Milestone 10, now starting at line 666, is the
  natural next slice)
- [`TIMESERIES_PHASE_18_HANDOFF.md`](TIMESERIES_PHASE_18_HANDOFF.md) (prior
  handoff, superseded by this one)
- GitHub issue [`#3`](https://github.com/jeromebanks/cubism-rs/issues/3)
  (rustfmt/edition-2024 discrepancy — this session's `cargo fmt -p`
  over-formatting is a new symptom of the same tracked problem)
