# Time-Series Phase 20 Handoff

Date: 2026-08-15

Branch: `feature/timeseries-phase-0a`

Status: **Landed Milestone 10, narrowed** (`docs/TIMESERIES_ROADMAP.md`'s
"Phase 5 Milestones" section) — `CoveragePlan`/`SegmentCoverage`, resolving
a `ResolutionPlan`'s segments against caller-supplied real publication
state (provenance, `missing` markers, `exact=true` failure), added to the
existing `crates/cubism-datafusion/src/range_query.rs` module. Value
materialization (`AggregateState` decode/merge, i.e. the roadmap's original
"`SeriesResponse`" half of this milestone's title) is explicitly deferred
as a not-yet-scoped "Milestone 10b" — see "What was actually verified" and
the roadmap entry for why. Six new unit tests plus one new integration test
against a real `cubism-iceberg` in-memory catalog. Milestone 10 flipped to
`Done` (narrowed) in the roadmap; no new GitHub issues filed. One tooling
incident this session: an out-of-band reformat touched five files this
session never edited, reverted before committing (see below).

(Despite the filename, this doc documents a session slice, not "Phase 20"
of the implementation plan — same convention every prior handoff in this
series has used: plan Phase 5 is DataFusion range queries and serving; the
plan's Phase 4 (`docs/TIMESERIES_IMPLEMENTATION_PLAN.md:582-670`) has every
`docs/TIMESERIES_ROADMAP.md` Phase-4 milestone done but is not itself fully
done — see that roadmap's "Phase 4 done" section, unchanged by this
session, for what remains.)

**Superseded by:** (none yet — this is the latest handoff)

## What this session built

Read `docs/TIMESERIES_PHASE_19_HANDOFF.md` and confirmed the branch in sync
with `origin` (`git rev-list --left-right --count` reported `0 0`), then
listed all open issues (`#1`-`#18`, unchanged from Phase 19). Phase 19's
deferred item 1 named Milestone 10 as the natural next slice, and the
roadmap confirmed it as the next Phase 5 milestone with "Depends on:
Milestones 7, 8, 9," all now `Done`. Read its full spec
(`docs/TIMESERIES_ROADMAP.md:670-706`, pre-edit) and confirmed it with the
advisor before writing anything, per the skill's step 1.

Two advisor calls shaped this slice before any code was written:

- **First call** confirmed Milestone 10 as the right slice and flagged two
  blocking checks to run before designing anything: (1) the dependency
  direction between `cubism-datafusion` and `cubism-iceberg` — the roadmap
  text (line 542, pre-edit) floated promoting `cubism-iceberg` to a normal
  dependency "in one line," but reading both `Cargo.toml`s showed it is
  currently a dev-dependency only, with no cycle either direction; and (2)
  that `WindowId` (`crates/cubism-core/src/temporal.rs:527-543`) is an
  opaque, caller-assigned string with no derivation from a
  `TimeRange`/`Resolution` pair anywhere in the codebase —
  `FixedResolution::bucket` returns a `TimeBucket`, not a `WindowId` — so
  "for each segment, resolve whether it's backed by a current published
  revision" is not implementable as the roadmap text describes without a
  segment→window mapping the caller supplies. The advisor's guidance:
  don't invent a canonical encoding here (a durable `cubism-core` decision
  out of scope for this milestone); take the mapping as an input; keep
  `cubism-iceberg` as a dev-dependency so `CoveragePlan` stays pure,
  synchronous computation matching `TemporalQuery`/`ResolutionPlan`; reuse
  `PublicationStore::current` (not `read_window`'s `Ok`/`Err`) to
  distinguish "unpublished" from "backend failure"
  (`crates/cubism-iceberg/src/control.rs:315-318`); and confirmed the
  value-materialization cut (below) directly from the roadmap's own
  **Test** bullet rather than needing a second confirmation of it.
- **Second call**, after a concrete API design was drafted, confirmed the
  two blocking checks came back as expected (dev-dependency only, no
  cycle) and flagged one blocking correctness issue in the draft: a design
  that set `is_exact = true` whenever a segment's only window was
  published, regardless of `ResolutionSegment::aligned`, would let a
  partial (unaligned) segment claim exactness it cannot have — the plan's
  own words (`docs/TIMESERIES_IMPLEMENTATION_PLAN.md:721-722`) are
  "`exact=true` fails clearly if retained buckets/raw data cannot exactly
  cover a partial boundary," and a published window's aggregate state is
  bucket-granularity, not sub-bucket. Fixed before writing: `is_exact` now
  requires both `aligned` and full publication
  (`SegmentCoverage::is_exact`, `range_query.rs:362-369`), and a dedicated
  test (`coverage_plan_exact_true_fails_on_unaligned_segment_even_when_published`)
  proves the unaligned-but-published case fails `exact: true` on its own,
  distinct from the missing-window failure case. Two smaller fixes from the
  same call: report every offending segment in the `exact=true` error, not
  just the first; and reuse `Exactness`/`Coverage`/`BucketValue`'s existing
  vocabulary deliberately or document the divergence — resolved by *not*
  reusing `Exactness` (its binary Exact/Inexact conflates "partial but
  published" with "wholly missing," which the roadmap's "identifies
  segments needing a raw-event scan **or** reporting `missing`" treats as
  two different things) in favor of the `published`/`missing` field pair,
  documented in the module doc comment.

Per the advisor's process note, before treating anything as newly
fileable, the open-issue list (`#1`-`#18`) was checked against this
session's work: nothing in it covers `CoveragePlan` territory, and #16
(event-time window identification for corrections) is a related but
distinct gap — it's about identifying windows a *correction* touches from
source event time, not about a *query-serving* segment→window mapping — so
no overlap and no issue filed for the `WindowId`-mapping deviation (same
"record in the roadmap entry" precedent Milestone 9 set for its own scope
cuts, not a new GitHub issue).

**A second tooling incident, same class as Phase 19's, different
trigger:** after implementing and testing, `git status` showed five files
this session never opened or edited (`build.rs`, `state_udaf.rs`,
`temporal_build.rs`, `udaf.rs`, `udf.rs`) modified — 1,275 insertions/260
deletions across all nine touched files in `git diff --stat`, of which
these five accounted for the bulk. This session's only explicit formatting
commands were `rustfmt --edition 2024` invoked on exactly the three files
this milestone touched (`range_query.rs`, `lib.rs`, `iceberg_bridge.rs`) —
never `cargo fmt -p`, per Phase 19's own deferred-item-3 workaround. The
five extra files were reformatted by something else in the toolchain path
(not identified further this session — a hook or environment-level
formatter is the leading candidate, but this wasn't confirmed). Verified
each of the five diffs was import-reordering/line-wrap only (no semantic
change: `git diff -w`, then a manual read of each full diff) before
reverting all five with `git checkout --` back to their committed state,
then re-running the full step-4 battery fresh a second time to confirm the
revert introduced no regression (see "Verification performed"). No new
issue filed beyond `#3`, which already tracks this class of rustfmt/
toolchain discrepancy; recorded here as a second, differently-triggered
occurrence for whichever session investigates `#3` next.

Scope fence held per the advisor's confirmation: no `AggregateState`
decode/merge, no `read_window`/`AggregateReader` call from `src/`, no
`SeriesResponse` value/presentation field — those are Milestone 10b's job,
not this one's. `CoveragePlan`'s `published`/`missing` fields carry
provenance and the missing marker only.

Primary files changed:

- **`crates/cubism-datafusion/src/range_query.rs`** (modified): added
  `SegmentCoverage` (`:356-360`) and its `is_exact` (`:362-369`),
  `CoveragePlan` (`:377-380`) and its `new` (`:382-461`), plus a module doc
  comment addendum (`:70-121`) describing what Milestone 10 does and does
  not decide and its two dependency/mapping deviations, and six new unit
  tests (`:706-811`).
- **`crates/cubism-datafusion/src/lib.rs`** (modified, +2/-1 lines): the
  re-export line now also exports `CoveragePlan`, `SegmentCoverage`.
- **`crates/cubism-datafusion/tests/iceberg_bridge.rs`** (modified): added
  `coverage_plan_resolves_real_publication_state_across_two_windows`
  (`:213-311`) — the real end-to-end wiring, publishing one day-resolution
  window and leaving an adjacent one unpublished, reusing Milestone 7's
  in-memory-catalog harness.
- **`docs/TIMESERIES_ROADMAP.md`** (modified): Milestone 10's `**Status:**`
  flipped from `Not started` to `Done` (narrowed), with both deviations,
  the stricter `is_exact` rule, and landed line numbers recorded; the
  "Phase 5 'done' condition" section's walk of completion criteria 772/775
  corrected from "met by Milestone 10" to "not yet met / partially met,
  pending Milestone 10b," since the original text assumed value
  materialization was part of Milestone 10's scope.
- **`docs/TIMESERIES_PHASE_19_HANDOFF.md`** (modified): added
  `**Superseded by:**` line.

No `Cargo.toml` change: `cubism-iceberg` deliberately stays a
`cubism-datafusion` dev-dependency (see deviations above); `CoveragePlan`
only needs `WindowId`/`WindowRevision`, already exported by `cubism-core`
and already a regular dependency.

## What was actually verified

That `CoveragePlan::new` correctly classifies segments against
caller-supplied publication state for five shapes, all pure unit tests in
`range_query.rs`: an aligned, fully-published segment is exact with
correct provenance
(`coverage_plan_is_exact_when_segment_aligned_and_all_windows_published`);
an aligned segment with an unpublished window is reported `missing`
without `CoveragePlan::new` itself failing when `exact: false`
(`coverage_plan_reports_missing_window_without_failing_when_exact_not_requested`);
the same shape fails clearly when `exact: true`
(`coverage_plan_exact_true_fails_on_missing_window`); an **unaligned**
segment with its only window **published** still fails `exact: true` —
proving the stricter `aligned && published` rule, not just "published"
(`coverage_plan_exact_true_fails_on_unaligned_segment_even_when_published`);
and a single aligned segment backed by two windows, one published and one
not, reports both correctly in the same `SegmentCoverage` and fails
`exact: true`
(`coverage_plan_mixed_published_and_missing_within_one_segment` — this is
the shape closest to the roadmap's own **Test** bullet). A sixth test
(`coverage_plan_rejects_windows_length_mismatch`) proves a caller supplying
the wrong number of window-list entries is rejected rather than panicking
or silently ignoring the mismatch.

That the real end-to-end wiring works, via one `#[tokio::test]` against a
real `cubism-iceberg` in-memory catalog
(`coverage_plan_resolves_real_publication_state_across_two_windows`,
`iceberg_bridge.rs:213`): a `ResolutionPlan` built over two contiguous
day-resolution windows stays one aligned interior segment (Milestone 9's
own guarantee, re-asserted here as a precondition); one window is claimed,
appended, and published through `AggregateWriter`/`PublicationStore`
exactly as Milestone 7's round-trip test does; the other is never touched;
`PublicationStore::current` is called directly (not `read_window`) for
each, resolving to `Some(revision)` and `None` respectively; and feeding
those into `CoveragePlan::new` produces a `SegmentCoverage` whose
`published` field holds the real `WindowRevision` the publish call
returned and whose `missing` field names the untouched window, with
`exact: true` failing on a message containing `"exact=true"`.

That the full step-4 verification battery — including `cargo clippy … -D
warnings` — is clean with this change in place, run fresh this session
(see "Tests" below), and re-run a second time in full after reverting the
five unrelated files an out-of-band reformat had touched, to confirm the
revert introduced no regression.

It does **not** prove: anything about `AggregateState` value decoding or
merging (`AggregateState::decode`/`merge`, `state_udaf.rs`) — `read_window`
is never called from `src/`, and no test in this session decodes a value
back out of a published window; that is Milestone 10b's job, not
demonstrated here at all, deliberately. It does not prove anything about
storage pruning across many windows — `read_window`'s predicate remains a
single `(window_id, revision)` equality (plan completion-criterion 774,
unchanged, gated on `#8`). It does not prove a canonical segment→`WindowId`
mapping exists or is correct in general — the integration test's mapping
(`w1`/`w2` as literal day-string ids matching the query's day-resolution
buckets) is hand-constructed by the test itself, exactly the "caller's
contract" the module doc comment describes, not something `CoveragePlan`
derives or validates. It does not identify the root cause of the
out-of-band reformat incident described above — only that it happened, was
reverted, and left no regression.

## GitHub issues touched

- No new issues filed. The out-of-band reformat of five untouched files is
  a new, differently-triggered *symptom* of `#3`'s already-tracked
  rustfmt/toolchain discrepancy (Phase 19's occurrence was `cargo fmt -p`
  reformatting a whole crate; this session's explicit formatting commands
  never did that, yet five files changed anyway) — recorded above for
  whichever session next investigates `#3`, not filed as a separate issue.
- No comments added to any other open issue; nothing Milestone 10 touches
  narrows or resolves any of `#1`-`#18`. `#16` was read in full to confirm
  it does not already cover the `WindowId`-mapping gap (see "What this
  session built") — it doesn't; no comment needed since no overlap exists
  to flag.

## Deferred / not done this session

1. **"Milestone 10b" (value materialization)** — not yet added to the
   roadmap as its own numbered milestone; recorded in Milestone 10's own
   entry as the natural next slice. Build `SeriesResponse`'s
   value/presentation fields on top of the `CoveragePlan` this session
   landed: for each `published` window a `SegmentCoverage` reports, call
   `AggregateReader::read_window` and merge via the existing
   `AggregateState::decode`/`merge`/`state_udaf.rs` machinery. This is
   also where `cubism-iceberg`'s promotion from dev-dependency to a normal
   `cubism-datafusion` dependency would actually become necessary (this
   session confirmed no cycle blocks that promotion).
2. **Out-of-band reformat root cause** — not identified this session
   (worked around by reverting and re-verifying, same as Phase 19's
   `cargo fmt -p` incident). A future session hitting this again should
   check whether a hook or environment-level formatter runs on file edits
   independent of any explicit `cargo fmt`/`rustfmt` invocation, since
   this session's explicit commands were already scoped to individual
   files and the extra reformatting happened anyway.
3. **#17** (append-committed-but-not-recorded recovery) — unchanged; still
   needs a design decision before it can be sized into a bounded milestone.
   Not re-read this session (Phase 19 already re-confirmed its state; no
   reason to expect it changed).
4. **#16** (event-time window identification + recompute-equality proof) —
   unchanged; re-read in full this session (see "GitHub issues touched")
   and confirmed distinct from this milestone's `WindowId`-mapping
   deviation, not touched by `CoveragePlan` itself.
5. **#10** (real object store + Iceberg maintenance) — unchanged.
6. **#11/#12** — unchanged; not touched this session.
7. **Plan completion-criterion 774** (storage pruning across many windows)
   — unchanged; still gated on #8's SQL/pushdown half.
8. **`/api/series` (`crates/cubism-serve`) and plan-Phase 6** — unchanged;
   not reached by Milestones 8-10(b).
9. **`#14`** (retry-loop/connection-poisoning gap) — unchanged, still open;
   its own suggested next step (force a `SQLITE_BUSY`/`SQLITE_LOCKED`
   retry past `busy_timeout`) is still not implemented, a deliberately
   slow (>5s) test not added to keep the fast suite fast.

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log`
for the exact hash — this doc deliberately doesn't hardcode it, per this
series' established convention). That commit contains:

- New: `docs/TIMESERIES_PHASE_20_HANDOFF.md` (this file).
- Modified: `crates/cubism-datafusion/src/range_query.rs` (Milestone 10's
  `CoveragePlan`/`SegmentCoverage`), `crates/cubism-datafusion/src/lib.rs`
  (re-export), `crates/cubism-datafusion/tests/iceberg_bridge.rs` (new
  integration test), `docs/TIMESERIES_ROADMAP.md` (Milestone 10 status,
  Phase 5 done-condition correction), `docs/TIMESERIES_PHASE_19_HANDOFF.md`
  (added `**Superseded by:**` line).
- Untouched: `crates/cubism-core/src`, `crates/cubism-iceberg/src`,
  `crates/cubism-datafusion/src/{build,state_udaf,temporal_build,udaf,udf}.rs`
  (briefly reformatted out-of-band mid-session and reverted with `git
  checkout --` before staging; see "What this session built"),
  `crates/cubism-serve/src`, `crates/cubism-cli/` (built and
  clippy-checked, not modified).

Also present, deliberately uncommitted per prior-session convention:
`.serena/` (local tooling state), `examples/web_analytics_demo/events.csv`
(generated demo output).

## Tests (33 passed + 1 ignored in `cubism-iceberg`, unchanged; 49 passed in `cubism-datafusion`, up from 42; 189 passed / 2 ignored in workspace, up from 182)

All figures re-run fresh this session, not carried forward from Phase 19,
and re-run a second full time after reverting the five out-of-band-
reformatted files. `cargo test -p cubism-iceberg` reports 33 passed, 1
ignored (6 suites) — identical to Phase 19, expected since no source in
that crate changed this session. `cargo test -p cubism-datafusion` reports
49 passed (3 suites) — up from Phase 19's 42 by exactly the seven new
tests (six `CoveragePlan` unit tests plus the one new integration test).
`cargo test --workspace --exclude cubism-py` reports 189 passed, 2 ignored
(23 suites) — up from Phase 19's 182 by exactly the same seven.

## Verification performed

```text
cargo test -p cubism-datafusion --lib range_query                        # 17 passed (11 prior + 6 new)
cargo test -p cubism-datafusion --test iceberg_bridge                    # 2 passed (1 prior + 1 new)
cargo test -p cubism-iceberg                                              # 33 passed, 1 ignored (6 suites)
cargo test -p cubism-iceberg --test concurrency                           # 4 passed, 1 ignored
cargo test -p cubism-iceberg --test durability                            # 7 passed
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings     # clean
cargo build -p cubism-cli                                                 # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings        # clean
cargo test -p cubism-datafusion                                           # 49 passed (3 suites)
cargo clippy -p cubism-datafusion --all-targets --no-deps -- -D warnings  # clean
cargo test --workspace --exclude cubism-py                                # 189 passed, 2 ignored (23 suites)
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
```

The `cubism-datafusion`/workspace test and clippy lines above were each run
twice this session: once before the five-file out-of-band reformat was
noticed, and once fresh after reverting it, to confirm the revert
introduced no regression. Figures shown are from the post-revert run.

## Primary files

- [`../crates/cubism-datafusion/src/range_query.rs`](../crates/cubism-datafusion/src/range_query.rs)
  (Milestone 10's `CoveragePlan`/`SegmentCoverage`)
- [`../crates/cubism-datafusion/src/lib.rs`](../crates/cubism-datafusion/src/lib.rs)
  (re-export)
- [`../crates/cubism-datafusion/tests/iceberg_bridge.rs`](../crates/cubism-datafusion/tests/iceberg_bridge.rs)
  (new integration test, line 213)
- [`../docs/TIMESERIES_ROADMAP.md`](TIMESERIES_ROADMAP.md) (Milestone 10,
  lines 670-760 post-edit; "Phase 5 'done' condition," lines 761-799
  post-edit)
- [`TIMESERIES_PHASE_19_HANDOFF.md`](TIMESERIES_PHASE_19_HANDOFF.md) (prior
  handoff, superseded by this one)
- GitHub issue [`#3`](https://github.com/jeromebanks/cubism-rs/issues/3)
  (rustfmt/toolchain discrepancy — this session's out-of-band reformat is a
  second, differently-triggered symptom of the same tracked problem)
- GitHub issue [`#16`](https://github.com/jeromebanks/cubism-rs/issues/16)
  (read in full this session to confirm no overlap with the
  `WindowId`-mapping deviation)
