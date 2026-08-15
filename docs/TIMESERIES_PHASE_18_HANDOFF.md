# Time-Series Phase 18 Handoff

Date: 2026-08-15

Branch: `feature/timeseries-phase-0a`

Status: **Landed Milestone 8** (`docs/TIMESERIES_ROADMAP.md`'s "Phase 5
Milestones" section) — `TemporalQuery`, the Phase 5 request shape
(`docs/TIMESERIES_IMPLEMENTATION_PLAN.md:697-706`), as a new
`crates/cubism-datafusion/src/range_query.rs` module. Pure request-shape
logic: no DataFusion execution types, no `cubism-iceberg` I/O, no new
crate dependency. Four unit tests cover valid construction (explicit
supported resolution, and auto/`None`) and both required rejection cases
(`start >= end`; an unsupported resolution). Milestone 8 flipped to `Done`
in the roadmap; no new GitHub issues filed.

(Despite the filename, this doc documents a session slice, not "Phase 18"
of the implementation plan — same convention every prior handoff in this
series has used: plan Phase 5 is DataFusion range queries and serving; the
plan's Phase 4 (`docs/TIMESERIES_IMPLEMENTATION_PLAN.md:582-670`) has every
`docs/TIMESERIES_ROADMAP.md` Phase-4 milestone done but is not itself
fully done — see that roadmap's "Phase 4 done" section, unchanged by this
session, for what remains.)

**Superseded by:** (none yet — this is the latest handoff)

## What this session built

Read `docs/TIMESERIES_PHASE_17_HANDOFF.md` and confirmed the branch in
sync with `origin` (`git rev-list --left-right --count` reported `0 0`),
then listed all open issues (`#1`-`#18`, unchanged from Phase 17). Phase
17's deferred item 1 named Milestone 8 as the natural next slice, and the
roadmap confirmed it as the first `Not started` Phase 5 milestone with
"Depends on: nothing new." Read its full spec
(`docs/TIMESERIES_ROADMAP.md:568-585`, pre-edit) and confirmed it with the
advisor before writing anything, per the skill's step 1.

The advisor confirmed Milestone 8 as the right slice and flagged the one
decision that shapes the whole struct: Milestone 9's own roadmap text
says "given a `TemporalQuery` **and** a cube's `TemporalSpec`"
(`docs/TIMESERIES_ROADMAP.md:610`, post-edit line — the advisor read it
pre-edit at line 591, before this session's Milestone 8 edit shifted
everything below it) — meaning `TemporalQuery` must not store the spec,
or Milestone 9's signature becomes redundant. `TemporalQuery` therefore
holds `cube: String` (an identifier only); `TemporalQuery::new` takes
`&TemporalSpec` purely as a validation input it does not retain.

Four more decisions came out of the same advisor call, all applied as
written:

- **`start`/`end` as raw `EventTime`, not a pre-built `TimeRange`.**
  `TimeRange::new` (`crates/cubism-core/src/temporal.rs:71-80`) already
  enforces `start < end`, but Milestone 8's roadmap text requires *this*
  milestone's own rejection test for that case. Taking raw bounds and
  building `TimeRange::new(start, end)?` inside `TemporalQuery::new`
  means the rejection genuinely flows through this constructor (proven by
  `rejects_start_not_before_end`, `range_query.rs:181-196`) without
  duplicating the invariant — the module doc comment says explicitly that
  the check is delegated.
- **`resolution: Option<Resolution>`**, `None` = auto. The membership
  check (`requested == spec.base_resolution || spec.rollups.contains(&requested)`)
  only runs when `Some`; the doc comment states that *selecting* an
  actual resolution for `None` is Milestone 9's job, not this
  constructor's, so `None` reads as deliberate rather than unimplemented.
- **Supported set = `base_resolution` ∪ `rollups`**, a plain membership
  check — `TemporalSpec::validate()` (which returns `Vec<String>`, not a
  `Result`) is not called from the constructor; re-validating the whole
  spec isn't this milestone's job. One consequence recorded in both the
  module doc comment and the roadmap: a `Resolution::Calendar` value
  sitting in `spec.base_resolution`/`spec.rollups` passes this membership
  check the same as `Fixed`, because rejecting `Calendar` support is
  `TemporalSpec::validate`'s job
  (`crates/cubism-core/src/temporal.rs:625-633`), not this one's.
- **Timezone omitted entirely this slice.** The plan lists "timezone/
  display options" as a request field, but `TemporalSpec::validate`
  already hard-rejects any non-`"UTC"` spec
  (`crates/cubism-core/src/temporal.rs:619-624`) — a free-form timezone
  request field would be unimplementable without re-deriving that check,
  which is out of scope here. Recorded as a deviation in the roadmap's
  Milestone 8 entry rather than silently dropped.

Scope fence held per the advisor's confirmation: no `CubeSpec` threaded
through the module at all — measures are plain `Vec<String>` names and
`XUnit` selectors are plain `Vec<XUnit>`, neither validated against a
cube's actual measure/dimension set, since that's not in Milestone 8's
"e.g." validation list. `GapPolicy` (`Missing`/`Zero`) is a plain field
forwarded to later milestones, not exercised by any logic in this module
beyond existing.

Per the advisor's process note, before treating any deferred item as
newly-fileable this session, `#14`'s full body was read (not just its
truncated list title): it already covers the retry-loop/connection-
poisoning gap that Phase 15-17's "Deferred" lists have been carrying
forward as a plain prose item — confirmed still the same open issue, no
duplicate filed (see "GitHub issues touched").

Primary files changed:

- **`crates/cubism-datafusion/src/range_query.rs`** (new, 216 lines): the
  `GapPolicy` enum (lines 48-53), `TemporalQuery` struct (lines 60-69),
  `TemporalQuery::new` (lines 77-114), and four unit tests (lines
  118-216) — plus a module doc comment (lines 1-34) stating exactly what
  construction does and does not validate, this series' standing
  convention for a new module making a claim about a request-shape
  contract. `GapPolicy`'s own doc comment (lines 38-46) cross-references
  `cubism_core::BucketValue` (`crates/cubism-core/src/temporal.rs:523`) —
  checked this session (`rtk proxy grep -rn "Gap\|gap\|Missing"
  crates/cubism-core/src/`) to confirm no existing type already covers
  this policy before adding a new one beside it. `TemporalQuery::new`
  carries `#[allow(clippy::too_many_arguments)]` (9 parameters), disclosed
  in a doc comment immediately above it rather than worked around with a
  builder — out of scope for a request-shape-only milestone.
- **`crates/cubism-datafusion/src/lib.rs`** (modified, +2 lines): added
  `pub mod range_query;` (line 8) and re-exported `GapPolicy`,
  `TemporalQuery` (line 17).
- **`docs/TIMESERIES_ROADMAP.md`** (modified): Milestone 8's `**Status:**`
  flipped from `Not started` to `Done`, with both deviations (no stored
  `TemporalSpec`; no timezone field) and the landed line numbers recorded.
- **`docs/TIMESERIES_PHASE_17_HANDOFF.md`** (modified): added
  `**Superseded by:**` line.

No `Cargo.toml` change: `TemporalQuery` only needs `cubism-core` types
(`EventTime`, `Resolution`, `TemporalSpec`, `TimeRange`, `XUnit`), already
a normal (non-dev) dependency of `cubism-datafusion`.

## What was actually verified

That `TemporalQuery::new` constructs successfully for two valid shapes —
an explicit resolution that is one of the spec's rollups
(`valid_construction_with_explicit_supported_resolution`,
`range_query.rs:144-160`), and `resolution: None` with no membership check
run at all (`valid_construction_with_auto_resolution`,
`range_query.rs:163-178`) — and rejects both required cases with
`CubismError::Temporal`: `start == end`
(`rejects_start_not_before_end`, `range_query.rs:181-196`) and a
resolution that is neither the base resolution nor a rollup
(`rejects_resolution_not_in_base_or_rollups`, `range_query.rs:199-215`,
using a day resolution against a minute-base/hour-rollup spec). That the
full step-4 verification battery — including `cargo clippy … -D
warnings`, clean with `TemporalQuery::new`'s `#[allow(clippy::too_many_arguments)]`
in place — is clean with this change in place, run fresh this session
(see "Tests" below).

It does **not** prove: anything about resolution *selection* — `None`
constructs without error, but nothing here chooses a concrete resolution
from the spec; that is Milestone 9's `ResolutionPlan`. It does not prove
measure names or `XUnit` selectors are valid against any particular cube
— no `CubeSpec` is consulted, only a bare cube-name `String` is stored.
It does not prove anything about `Resolution::Calendar` rejection — a
`TemporalSpec` carrying a `Calendar` base/rollup would pass this module's
membership check; only `TemporalSpec::validate` (called elsewhere, not
from this constructor) rejects `Calendar` support. It does not exercise
any DataFusion execution path, `cubism-iceberg` I/O, or the
`AggregateState::decode`/`merge` machinery `state_udaf.rs` already has —
Milestone 8 is request-shape-only, consistent with the roadmap's own
"touches no DataFusion execution types" framing.

## GitHub issues touched

- No new issues filed. Per the advisor's process note, `#14`'s full body
  was re-read this session and confirmed to already cover the retry-loop/
  connection-poisoning gap Phase 15-17's deferred lists have carried
  forward as a plain prose item (Phase 17's deferred item 10) — that item
  is now pointed at `#14` explicitly below instead of repeated as a
  standalone untracked item.
- No comments added to any existing issue this session; nothing Milestone
  8 touches narrows or resolves any open issue's scope.

## Deferred / not done this session

1. **Milestone 9 (`ResolutionPlan`, non-overlapping segment selection)**
   — the roadmap's next Phase 5 milestone
   (`docs/TIMESERIES_ROADMAP.md:606-621`, post-edit: this session's
   Milestone 8 edit added lines above it, shifting it down from its
   pre-edit 587-602). Depends on Milestone 8, now done. Natural next
   slice — it's the first thing that actually *uses* the
   resolution-membership premise Milestone 8 just validated.
2. **Milestone 10 (`CoveragePlan`/`SeriesResponse`)** — depends on
   Milestones 8 (done) and 9 (not started); unstarted.
3. **`TemporalQuery`'s omitted timezone/display-options field** — not
   filed as a separate issue; recorded in the roadmap's Milestone 8 entry
   as a deliberate scope cut, revisit only if/when v2alpha1 timezone
   support beyond `"UTC"` becomes real (currently hard-rejected by
   `TemporalSpec::validate`).
4. **#17** (append-committed-but-not-recorded recovery) — unchanged; still
   needs a design decision before it can be sized into a bounded
   milestone. Not re-read this session (Phase 17 already re-confirmed its
   state; no reason to expect it changed).
5. **#16** (event-time window identification + recompute-equality proof)
   — unchanged; still needs a decision on which crate closes it. Not
   touched by Milestone 8 (`range_query.rs` doesn't read or write
   anything from `cubism-iceberg`).
6. **#10** (real object store + Iceberg maintenance) — unchanged.
7. **#11/#12** — unchanged; not touched this session.
8. **Plan completion-criterion 774** (storage pruning across many
   windows) — unchanged; still gated on #8's SQL/pushdown half.
9. **`/api/series` (`crates/cubism-serve`) and plan-Phase 6** — unchanged;
   not reached by Milestones 8-10.
10. **The retry-loop/connection-poisoning gap** — this is `#14`, already
    filed and already open; carried forward here as a pointer to that
    issue rather than as a fresh untracked item (see "GitHub issues
    touched" above). `#14`'s own suggested next step (force a
    `SQLITE_BUSY`/`SQLITE_LOCKED` retry past `busy_timeout`) is still not
    implemented — a deliberately slow (>5s) test, not added to keep the
    fast suite fast.

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log`
for the exact hash — this doc deliberately doesn't hardcode it, per this
series' established convention). That commit contains:

- New: `docs/TIMESERIES_PHASE_18_HANDOFF.md` (this file),
  `crates/cubism-datafusion/src/range_query.rs`.
- Modified: `crates/cubism-datafusion/src/lib.rs` (module declaration and
  re-export), `docs/TIMESERIES_ROADMAP.md` (Milestone 8 status),
  `docs/TIMESERIES_PHASE_17_HANDOFF.md` (added `**Superseded by:**`
  line).
- Untouched: `crates/cubism-core/src`, `crates/cubism-iceberg/src`,
  `crates/cubism-datafusion/src/{build,state_udaf,temporal_build,udaf,udf}.rs`,
  `crates/cubism-serve/src`, `crates/cubism-cli/` (built and
  clippy-checked, not modified).

Also present, deliberately uncommitted per prior-session convention:
`.serena/` (local tooling state), `examples/web_analytics_demo/events.csv`
(generated demo output).

## Tests (33 passed + 1 ignored in `cubism-iceberg`, unchanged; 35 passed in `cubism-datafusion`, up from 31; 175 passed / 2 ignored in workspace, up from 171)

All figures re-run fresh this session, not carried forward from Phase 17.
`cargo test -p cubism-iceberg` reports 33 passed, 1 ignored (6 suites) —
identical to Phase 17, expected since no source in that crate changed
this session. `cargo test -p cubism-datafusion` reports 35 passed (3
suites) — up from Phase 17's 31 by exactly the four new `range_query`
unit tests. `cargo test --workspace --exclude cubism-py` reports 175
passed, 2 ignored (23 suites) — up from Phase 17's 171 passed by exactly
the same four.

## Verification performed

```text
cargo test -p cubism-datafusion --lib range_query                        # 4 passed
cargo test -p cubism-iceberg                                              # 33 passed, 1 ignored (6 suites)
cargo test -p cubism-iceberg --test concurrency                           # 4 passed, 1 ignored
cargo test -p cubism-iceberg --test durability                            # 7 passed
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings     # clean
cargo build -p cubism-cli                                                 # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings        # clean
cargo test -p cubism-datafusion                                           # 35 passed (3 suites)
cargo test --workspace --exclude cubism-py                                # 175 passed, 2 ignored (23 suites)
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
```

## Primary files

- [`../crates/cubism-datafusion/src/range_query.rs`](../crates/cubism-datafusion/src/range_query.rs)
  (Milestone 8's `TemporalQuery`/`GapPolicy`)
- [`../crates/cubism-datafusion/src/lib.rs`](../crates/cubism-datafusion/src/lib.rs)
  (module declaration and re-export, lines 8 and 17)
- [`../docs/TIMESERIES_ROADMAP.md`](TIMESERIES_ROADMAP.md) (Milestone 8,
  lines 568-604 post-edit; Milestone 9, now lines 606-621, is the natural
  next slice)
- [`TIMESERIES_PHASE_17_HANDOFF.md`](TIMESERIES_PHASE_17_HANDOFF.md) (prior
  handoff, superseded by this one)
- GitHub issue [`#14`](https://github.com/jeromebanks/cubism-rs/issues/14)
  (retry-loop coverage gap — re-confirmed this session as already
  covering Phase 17's deferred item 10, no duplicate filed)
