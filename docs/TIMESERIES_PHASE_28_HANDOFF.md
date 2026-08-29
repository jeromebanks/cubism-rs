# Time-Series Phase 28 Handoff

Date: 2026-08-19

Branch: `feature/timeseries-phase-0a`

Status: **Landed Milestone 13** (`docs/TIMESERIES_ROADMAP.md`'s "POC
Milestones" section) — `docs/TIMESERIES_ARCHITECTURE.md`, a new doc
tracing one real event through all seven stages of the pipeline (the
plan's own six plus the served-HTTP hop Milestones 11-12b added), with
every claim cited into either a source file or a handoff. This closes
out the roadmap's tracked milestone list: **18/18**. (Naming note,
carried forward from every prior handoff in this series: this file's
number is a *session-slice* number, not a
`docs/TIMESERIES_IMPLEMENTATION_PLAN.md` phase number — this slice does
POC-milestone work, not plan-phase work.)

`docs/TIMESERIES_PHASE_27_HANDOFF.md`'s deferred list named Milestone 13
as the natural next slice (item 2), now unblocked with both 12a and 12b
`Done`. It's the only "Not started" entry in the roadmap's tracked scope.
Advisor review (before any writing) confirmed the pick and answered the
one open sizing question orientation raised: Milestone 13 is a
documentation task with no test and a much wider surface than this
series' usual "one integration test plus supporting code" slice — is it
still one bounded slice, or does it need splitting the way Milestone 12
did?

**Advisor's answer: don't split by pipeline stage — bound the *depth*
instead of the *breadth*.** Splitting by stage would leave a half-doc
that fails its own done-condition ("trace a single event... using this
doc alone") until the last piece landed — worse than 12a/12b, where each
half was independently demonstrable on its own. The bounded version:
write all seven stages end to end, but hold detail to one concrete traced
example (Milestone 12b's own demo — a real event, window, and revision
bump already on hand), with everything off that path getting one line and
a cross-link rather than a section. Advisor also flagged the specific
failure mode to guard against: this doc's job is "what's true today," not
"what the plan intends" — a stage's description that would read
identically whether or not the code existed is a sign of transcribing the
plan instead of citing the build.

Two cheap checks before writing, per advisor: whether an
architecture-shaped doc already existed (`docs/architecture.md` does —
the pre-existing *static*-cube pipeline doc; cross-linked and explicitly
distinguished rather than duplicated), and whether 8a applies (it
doesn't — the "POC Milestones" section states outright it isn't
8a-gated, `docs/TIMESERIES_ROADMAP.md:1304-1310`, and Milestone 13 closes
no `## Phase N "done" condition` section).

**Superseded by:** `docs/TIMESERIES_PHASE_29_HANDOFF.md` — picked up this
handoff's deferred-list item 2 (the roadmap's tracked milestones being
exhausted, falling back to the open-issue scan) and fixed
[#20](https://github.com/jeromebanks/cubism-rs/issues/20), this handoff's
deferred-list item 1 and the top-priority open issue at the time.

## What this session built

**`docs/TIMESERIES_ARCHITECTURE.md`** (new, ~340 lines) — traces one
event (`evt_000261`, a held-back `signup_completed` row, `plan: pro`,
`revenue: 99`, `country: DE`) through all seven pipeline stages:

1-2. observed event -> allowed sparse XUnits (`cubism-core`'s `ypath.rs`/
`lattice.rs`, reused unchanged from the static pipeline)
3. event-time bucket (`temporal.rs`'s `TemporalSpec`/`WindowId`/
`FixedResolution`/`AllowedLateness`)
4. versioned mergeable aggregate state (`aggregate_state.rs`'s
`AverageState`, `temporal_build.rs`'s build path)
5. immutable Iceberg window revision (`writer.rs`'s `append_window`,
`control.rs`'s `claim_run`/`record_append`/`publish` CAS protocol)
6. atomically published range-query visibility (`reader.rs`'s
`read_window`, `range_query.rs`'s `TemporalQuery`/`ResolutionPlan`/
`CoveragePlan`, `series_merge.rs`, `series_response.rs`'s `SeriesResponse`)
7. served answer, the hop beyond the plan's own six-stage contract
(`cubism-serve`'s `SeriesState`/`series_router`, `cubism-cli`'s `serve`/
`temporal-build`/`iceberg-build` subcommands)

Each stage section states what's narrowed inline (single `XUnit`
selector, `AverageState` only, no `LatenessPolicy`/`CorrectionPlan` wired
into the CLI path, no manifest semijoin) with a cross-link to the
tracking issue or roadmap section rather than restating the roadmap's own
"Phase 4/5 done condition" write-ups. A closing table collects all nine
narrowing points in one place for lookup.

**`docs/TIMESERIES_ROADMAP.md`** — Milestone 13 marked `Done`, its entry
rewritten with the "one worked example, full depth; everything else, one
line" sizing decision recorded (advisor's own recommendation, so a later
reader understands why the doc isn't structured as seven independent
sections of equal depth).

## What was actually verified

Every citation in the new doc was grep'd fresh against the current source
this session, not reused from an earlier session's citations — this
mattered more than usual here, since Milestone 13 is the first slice in
three to carry many `file.rs:N` references (the last two touched no
`.rs` file at all). Two real drift/error cases were caught and fixed
during that pass, not after:

1. A caveat about `is_exact` never being re-verified against `batches`
   was first cited to `series_response.rs:101-130` (the `impl
   SeriesResponse` block) — the actual text lives in the *module* doc
   comment at `series_response.rs:37-49`. Fixed before commit.
2. `LatenessPolicy` and `CorrectionPlan` were first described as both
   living in `cubism-iceberg` — `rtk proxy grep -rln` found
   `LatenessPolicy` actually lives in `crates/cubism-core/src/temporal.rs`
   (Milestone 1's own target crate); only `CorrectionPlan` is in
   `crates/cubism-iceberg/src/correction.rs` (Milestone 3). Fixed before
   commit.

The worked example's own numbers were independently recomputed, not
copied from the Phase 27 handoff's prose: `avg_revenue`'s revision-2
value (`1.6638655462184875`) is `594 / 357` — six `pro`-plan
`signup_completed` conversions at `99` each, over 357 total event rows in
window `2026-04-07` (most carrying `revenue: 0`), confirmed by summing
`generate_temporal_events.py`'s own output directly rather than trusting
the prior session's stated value. All 6 held-back events (`evt_000261`,
`evt_000310`, `evt_000658`, `evt_000930`, `evt_000997`, `evt_001031`)
were confirmed to be `signup_completed`/`pro`/`99` by inspecting the
generated CSV directly — the doc's specific claim about `evt_000261`
(`country: DE`) is a real field read from that row, not invented for the
example.

**What this does not prove:** the doc is documentation, not code — there
is no test to run against it, and "done" for Milestone 13 means the
citations check out and the traced example holds together, which is what
this session's re-verification pass did. It does not prove the doc will
stay accurate as later slices change the cited files; the doc's own
opening section says as much ("if a citation and the code disagree, trust
the code and treat this doc as stale for that line").

Since this slice touches no `.rs` file, the standard test-count claims
are **unchanged**: `cargo test -p cubism-iceberg` reports 35 passed, 1
ignored (6 suites); `cargo test --workspace --exclude cubism-py` reports
211 passed, 2 ignored (24 suites) — both re-run and re-verified this
session, identical because nothing under any crate's `src/` or `tests/`
changed. `rustfmt` was not run — no `.rs` file was touched.

## GitHub issues touched

None. Every gap the new doc surfaces was already tracked (`#20`, `#16`,
`#9`, `#8`, `#10`, `#19` — the last already closed) or is a narrowing
already documented inline in the source it cites. `#20` remains open and
untouched, still the highest-priority item.

## Deferred / not done this session

1. **`#20`** (a delayed correction replay can silently undo an intentional
   rollback) — unchanged, still the most consequential open item; needs a
   design decision, not a mechanical fix. With Milestone 13 landed, this
   is now the top item on both the deferred list and the roadmap's own
   "Deferred (not in scope for this roadmap doc)" section.
2. **The roadmap's tracked milestone list is now exhausted (18/18).** Per
   `docs/TIMESERIES_ROADMAP.md`'s own step-1 guidance (lines 52-64), the
   next session slice should fall back to its pre-roadmap behavior: scan
   open issues and the latest handoff's deferred list directly, starting
   with `#10` (compaction/retention/object-store, explicitly out of this
   roadmap's scope) or `#20` (correctness, needs a design decision first).
   This is not a claim that Phase 4, Phase 5, or the implementation plan
   overall are finished — see the roadmap's own "Phase 4 done condition"
   and "Phase 5 done condition" sections for what's excluded (`#10`, `#8`)
   and what's tracked separately (`#16`, `#20`).
3. Every item on `docs/TIMESERIES_PHASE_27_HANDOFF.md`'s deferred list
   items 3-5 (the TOCTOU gap, concurrent-request coverage for
   `/api/series`, and the older items gated on `#8`/`#16`/`#10`/`#9`/
   `#14`/`#4`/`#3`) — unchanged, none touched this session, all now
   cross-linked from the new architecture doc's "What's narrowed" table
   instead of only living in handoff prose.

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log`
for the exact hash — this doc deliberately doesn't hardcode it, per this
series' established convention).

- New: `docs/TIMESERIES_ARCHITECTURE.md`,
  `docs/TIMESERIES_PHASE_28_HANDOFF.md` (this file).
- Modified: `docs/TIMESERIES_ROADMAP.md` (Milestone 13 marked `Done`),
  `docs/TIMESERIES_PHASE_27_HANDOFF.md` (`Superseded by` line added),
  `docs/handoff_latest.md` (symlink retarget).
- Untouched: every crate under `crates/`, `examples/web_analytics_demo/`
  — this slice only added/read files, confirmed via `git status` before
  staging and after commit.

Also present, deliberately uncommitted: `.serena/` (local tooling state,
prior-session convention); `examples/web_analytics_demo/events.csv` (the
*static* demo's own generated output, unrelated to this session,
prior-session convention); `.claude/skills/timeseries-slice/SKILL.md`
(pre-existing uncommitted edits from before this session started —
unrelated to this slice's scope, left untouched, not staged).

## Tests (35 passed + 1 ignored in `cubism-iceberg`, unchanged; 211 passed / 2 ignored in workspace, unchanged)

Identical to Phase 27's counts across the board — this slice added one
new doc and touched two others, no `.rs` file.

## Verification performed

```text
cargo test -p cubism-iceberg                                              # 35 passed, 1 ignored (6 suites) — re-verified, unchanged
cargo test -p cubism-iceberg --test concurrency                            # 4 passed, 1 ignored — re-verified, unchanged
cargo test -p cubism-iceberg --test durability                             # 9 passed — re-verified, unchanged
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings      # clean
cargo build -p cubism-cli                                                  # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings         # clean
cargo test --workspace --exclude cubism-py                                 # 211 passed, 2 ignored (24 suites) — re-verified, unchanged
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
```

Every `cargo` leg's full, non-truncated output was captured to a file
first and grepped for `test result:`/`error` afterward (not piped through
`tail` directly), per the skill's own step-4 note about a truncated
capture producing a wrong total before. Totals summed by hand from each
suite's own `test result:` line match Phase 27's totals exactly.

## Primary files

- [`TIMESERIES_ARCHITECTURE.md`](TIMESERIES_ARCHITECTURE.md) (new — the
  architecture doc)
- [`TIMESERIES_ROADMAP.md`](TIMESERIES_ROADMAP.md) (Milestone 13 marked
  `Done`; roadmap's tracked milestone list now exhausted, 18/18)
- [`TIMESERIES_PHASE_27_HANDOFF.md`](TIMESERIES_PHASE_27_HANDOFF.md)
  (prior handoff, superseded by this one)
- GitHub issue [`#20`](https://github.com/jeromebanks/cubism-rs/issues/20)
  (untouched this session, still the top deferred item)
