# Time-Series Phase 16 Handoff

Date: 2026-08-15

Branch: `feature/timeseries-phase-0a`

Status: **Extended `docs/TIMESERIES_ROADMAP.md` to plan-Phase 5** (#15's
own second suggested step, deliberately deferred until every Phase 4
milestone was `Done` — confirmed true as of `docs/TIMESERIES_PHASE_15_HANDOFF.md`).
Doc-only session: no Phase 5 source code landed, only the roadmap's four
new milestones (7-10), a comment narrowing #8's scope, and a small
additive doc-comment correction in `crates/cubism-iceberg/src/reader.rs`
that carried the same overstated framing #8 did. The ad hoc deferred-list
scan (`#17`, `#16`, `#10`, `#11`, `#12`) turned up nothing newly sized to
a bounded slice — all five still need a design decision or further
scoping, unchanged from Phase 15's assessment, independently re-confirmed
this session by reading each issue's current body — which is why this
session's advisor-confirmed pick was the roadmap's own step-1 fallback
("starting with #10 and this roadmap's own 'Deferred' note about Phase
5") rather than another code slice.

(Despite the filename, this doc documents a session slice, not "Phase 16"
of the implementation plan — same convention every prior handoff in this
series has used: plan Phase 5 is DataFusion range queries and serving; the
plan's Phase 4 (`docs/TIMESERIES_IMPLEMENTATION_PLAN.md:582-670`) has every
`docs/TIMESERIES_ROADMAP.md` Phase-4 milestone done but is not itself
fully done — see that roadmap's "Phase 4 done" section, unchanged by this
session, for what remains.)

**Superseded by:** (none yet — this is the latest handoff)

## What this session built

Read `docs/TIMESERIES_PHASE_15_HANDOFF.md` and `docs/TIMESERIES_ROADMAP.md`,
confirmed the branch in sync with `origin` (`git rev-list --left-right
--count` reported `0 0`), and listed all open issues (`#1`-`#18`). Every
Phase-4 milestone (1-6) in the roadmap was already `Done`, so per the
roadmap's own step-4 fallback this session started from the ad hoc
deferred-list/issue scan: read the current bodies of `#17`, `#16`, `#10`,
`#11`, `#12` (all still stating in their own text that they need a design
decision or further scoping before they're sizeable — unchanged from
Phase 15's read, independently reconfirmed rather than assumed stale) and
checked whether Phase 15's deferred item 6 (a test forcing
`MAX_TX_ATTEMPTS` exhaustion) had become attemptable — it hasn't:
`MAX_TX_ATTEMPTS` is 8, `backoff`'s schedule contributes only tens of
milliseconds next to each attempt's up-to-5s `busy_timeout` wait, so
exhausting it costs roughly 40s minimum, confirmed by reading
`crates/cubism-iceberg/src/durable_control.rs`'s `MAX_TX_ATTEMPTS`/`backoff`
definitions directly rather than re-deriving the estimate from prose.

Consulted the advisor before picking a slice. It confirmed no open issue
was newly sized, and pointed at the roadmap's own step-1 fallback
instruction — "starting with #10 and this roadmap's own 'Deferred' note
about Phase 5" — as the face-value next move once #10 was confirmed
unshaped: extend the roadmap to plan-Phase 5, per `#15`'s own suggested
steps. It flagged two checks before committing:

1. **Read `#15`'s full body** to confirm it actually covers Phase 5 (not
   just Phase 4). Confirmed — `#15`'s "Suggested next steps" explicitly
   says "Extend past #13 to Phase 5... once Phase 4's milestones are
   drafted."
2. **Count how many Phase 5 milestones are decomposable without `#8`
   resolved** (the DataFusion 53/54 convergence blocker), since `#8`'s own
   text says Phase 5 "cannot proceed" until that convergence happens. If
   everything reduced to "resolve `#8` first," a one-milestone roadmap
   addition wasn't a real slice.

That second check surfaced more than expected: reading
`crates/cubism-iceberg/src/lib.rs`'s top-level doc comment,
`crates/cubism-iceberg/src/reader.rs`'s `read_window` doc comment, and
`crates/cubism-datafusion/src/state_udaf.rs` together showed `#8`'s "Phase
5 cannot proceed" framing is broader than its own evidence supports. `#8`
is right that *generic SQL-level* access — registering Iceberg tables in a
DataFusion `SessionContext` via `iceberg-datafusion`'s `TableProvider`
impl (DataFusion 53) so arbitrary predicate pushdown/joins work — needs
the version convergence. It is not right that Phase 5's *functional* path
needs it: calling `AggregateReader::read_window` directly from
`cubism-datafusion` (DataFusion 54, a plain path dependency, no
`iceberg-datafusion` involved) and merging the resulting `RecordBatch`es
with the merge machinery `state_udaf.rs` already has
(`AggregateState::decode`+`merge`) is the same "cross the crate boundary
via `arrow_array::RecordBatch`/`arrow_schema::Schema` only" strategy
`cubism-iceberg`'s own top-level doc comment already describes for Phase
3. `reader.rs`'s own doc comment carried the same overstated framing as
`#8` — a `read_window` call site describing range queries as "gated on
DataFusion 53/54 convergence" — corrected in place, additively, this
session (`reader.rs:29-45`, ahead of `read_window` itself at line 46),
matching this series' convention of adding a correction paragraph rather
than rewriting the original claim.

The advisor's second pass (after that finding, before writing the roadmap
content) drew the boundary precisely rather than accepting "nothing is
blocked": walked the plan's four Phase 5 completion criteria (lines
772-775) individually. Criteria 772/773/775 are reachable on the
direct-call path; criterion 774 ("range plans prune storage and stay
within latency/memory budgets") is not — `read_window`'s predicate is a
single `(window_id, revision)` equality per call, not a semijoin against
every published window's manifest, which is exactly what a real
`TableProvider` scan would buy. It also required an empirical check before
any milestone content was written, not just a reasoned one: `cargo tree -i
arrow --workspace` and `cargo tree -p cubism-iceberg` both confirm a
single unified `arrow` 58.3.0 resolves across the DF53 and DF54 dependency
trees, so a `RecordBatch` from `read_window` is already the type DF54's
`cubism-datafusion` expects — the premise every new milestone rests on,
verified via `cargo tree`, not assumed from `#8`'s own (pre-this-finding)
description of the dependency graph.

Primary files changed:

- **`docs/TIMESERIES_ROADMAP.md`** (modified): new "Phase 5 Milestones"
  section (Milestones 7-10 — a `RecordBatch`-into-DF54-`SessionContext`
  spike, `TemporalQuery`, `ResolutionPlan`, and `CoveragePlan`/
  `SeriesResponse` against real published windows) plus a "Phase 5 'done'
  condition" section walking the plan's four completion criteria
  individually, same convention as the existing "Phase 4 done" section.
  Updated the title, top-of-doc scope paragraph, "How step 1 should use
  this doc" step 4's fallback text, and "Deferred" item 1 (marked
  resolved, struck through) to reflect that the roadmap now covers part of
  Phase 5 instead of stopping at Phase 4. All Phase 5 milestones are
  `Status: Not started` — doc-only this session, no implementation.
- **`crates/cubism-iceberg/src/reader.rs`** (modified): additive
  correction paragraph on `read_window`'s doc comment (`reader.rs:29-45`,
  ahead of the function itself at line 46) narrowing "gated on DataFusion
  53/54 convergence" to the SQL/pushdown half specifically, cross-linking
  `#8` and the roadmap's Milestone 10. No behavior change — doc comment
  only.
- **`.claude/skills/timeseries-slice/SKILL.md`** (modified): step 1's
  parenthetical no longer claims the roadmap "does not cover plan-Phase 5+
  as of this writing" — it now says the roadmap covers Phase 4 and part of
  Phase 5, pointing at the roadmap's own "Phase 5 Milestones" section for
  what remains out of reach.

## What was actually verified

That the roadmap-extension reasoning's load-bearing premise is true today,
not merely plausible: `cargo tree -i arrow --workspace` shows one unified
`arrow` 58.3.0 across both the `datafusion` 53.1.0 and `datafusion` 54.0.0
subgraphs; `cargo tree -p cubism-iceberg` shows the same `arrow-array`/
`arrow-schema` 58.3.0 in `cubism-iceberg`'s own dependency tree; neither
`cubism-datafusion` nor `cubism-iceberg` currently depends on the other
(confirmed via each crate's `Cargo.toml`), so Milestone 7's proposed
dependency addition is new, not already wired. That the full step-4
verification battery stays clean after this session's two source-adjacent
edits (a doc comment and a roadmap doc) — every count re-run fresh this
session, not carried forward from Phase 15's numbers (see "Tests" below).

It does **not** prove: that a `RecordBatch` from `read_window` actually
round-trips through a live DF54 `SessionContext` without a runtime error
`cargo tree`'s static resolution can't see (an ABI mismatch, a feature-flag
divergence) — that's exactly what Milestone 7 is for, and it is
`Not started`. It does not prove any Phase 5 milestone's functional
content works — no Phase 5 code was written this session, only the
roadmap's decomposition of what to build and in what order. It does not
resolve `#8` — criterion 774 (storage pruning across many windows) is
explicitly recorded as still gated on it in both the roadmap's Milestone
10 and the `#8` comment.

## GitHub issues touched

- [#8](https://github.com/jeromebanks/cubism-rs/issues/8) — commented,
  narrowing "Phase 5 cannot proceed" to the SQL/pushdown half specifically,
  with the `cargo tree` evidence and a cross-link to the roadmap's
  Milestone 7 (which will convert the reasoning into an observed result).
  Left open — the narrowed half (criterion 774) is still real.
- [#15](https://github.com/jeromebanks/cubism-rs/issues/15) — commented,
  noting its second suggested step (extend the roadmap to Phase 5) is now
  done and its first and third were already done across Milestones 1-6.
  Left open — Phase 5's own milestones (7-10) aren't built, and the
  roadmap doesn't reach Phase 6.
- No new issues filed. `#17`, `#16`, `#10`, `#11`, `#12` were re-read in
  full this session and confirmed still not sized to a bounded slice —
  not re-litigated or re-filed.

## Deferred / not done this session

1. **Phase 5 Milestones 7-10 themselves** — the roadmap now describes
   them; none are implemented. Milestone 7 (the `RecordBatch`/DF54 spike)
   is the natural next slice once this handoff's advisor review is done,
   since Milestones 8-10 declare it as a dependency.
2. **#17** (append-committed-but-not-recorded recovery) — unchanged; still
   needs a design decision before it can be sized into a bounded
   milestone.
3. **#16** (event-time window identification + recompute-equality proof)
   — unchanged; still needs a decision on which crate closes it. Notably
   relevant to Phase 5: its own body suggests `cubism-datafusion` gaining
   a persistence-side integration test against `cubism-iceberg` is the
   likely home — exactly the dependency direction Milestone 7 adds. Worth
   re-reading once Milestone 7 lands, in case it unblocks part of `#16`
   too (not assumed here — a future slice's call).
4. **#10** (real object store + Iceberg maintenance) — unchanged; still
   not split into shaped, sizeable work per its own "Suggested next
   steps."
5. **#11/#12** — unchanged; this session did not touch either scope.
6. **Plan completion-criterion 774** (storage pruning across many
   windows) — explicitly out of Milestones 7-10's scope; stays gated on
   `#8`'s SQL/pushdown half or a not-yet-scoped multi-window
   `cubism-iceberg` read API. Recorded in the roadmap's "Phase 5 'done'
   condition" section, not filed as a separate issue (it's `#8`'s own
   scope, narrowed, not a new gap).
7. **`/api/series` (`crates/cubism-serve`) and plan-Phase 6** (rolling
   comparisons/trend inputs) — not covered by the roadmap's Phase 5
   section at all; the next roadmap extension once Milestones 7-10 close,
   per that section's own closing note.
8. **The `return Err(err)` branch's poisoning fix is unverified by a
   test** (carried forward from Phase 15, unchanged — still costs ~40s to
   exercise via `MAX_TX_ATTEMPTS` exhaustion, confirmed this session by
   reading `MAX_TX_ATTEMPTS`/`backoff` directly rather than re-estimating).

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log`
for the exact hash — this doc deliberately doesn't hardcode it, per this
series' established convention after `docs/TIMESERIES_PHASE_4_HANDOFF.md`
needed a follow-up commit to fix a self-referential hash). That commit
contains:

- New: `docs/TIMESERIES_PHASE_16_HANDOFF.md` (this file).
- Modified: `docs/TIMESERIES_ROADMAP.md` (Phase 5 Milestones section,
  scope/fallback text updates), `crates/cubism-iceberg/src/reader.rs`
  (additive doc-comment correction), `.claude/skills/timeseries-slice/SKILL.md`
  (stale Phase-5 parenthetical corrected), `docs/TIMESERIES_PHASE_15_HANDOFF.md`
  (added `**Superseded by:**` line).
- Untouched: `crates/cubism-core/src`, `crates/cubism-iceberg/src`
  (besides `reader.rs`'s doc comment), `crates/cubism-datafusion/src`,
  `crates/cubism-serve/src`, `crates/cubism-cli/` (built and
  clippy-checked, not modified).

Also present, deliberately uncommitted per prior-session convention:
`.serena/` (local tooling state), `examples/web_analytics_demo/events.csv`
(generated demo output).

## Tests (33 passed + 1 ignored in `cubism-iceberg`; 170 passed / 2 ignored in workspace)

Unchanged from Phase 15's counts — expected, since this session's only
source-adjacent edit was a doc comment. All figures re-run fresh this
session, not carried forward: `cargo test -p cubism-iceberg` reports 33
passed, 1 ignored (6 suites): 13 unit (`--lib`) + 6 Phase-3 integration
(`--test phase3`) + 7 durability integration (`--test durability`) + 4
passed/1 ignored concurrency integration (`--test concurrency`) + 3
coordinator integration (`--test coordinator`) — each suite run in
isolation this session as well as the full `cargo test -p cubism-iceberg`
run. `cargo test --workspace --exclude cubism-py` reports 170 passed, 2
ignored (22 suites) — re-run fresh in the foreground after an earlier
background invocation produced an empty output file (a harness artifact,
not a test failure; re-run directly to get a trustworthy number, per this
series' standing "don't guess counts from a truncated capture" rule).
`cargo test -p cubism-core` reports 92 passed (4 suites), unchanged from
Phase 15.

## Verification performed

```text
cargo test -p cubism-iceberg                                              # 33 passed, 1 ignored (6 suites)
cargo test -p cubism-iceberg --lib                                        # 13 passed
cargo test -p cubism-iceberg --test phase3                                # 6 passed
cargo test -p cubism-iceberg --test coordinator                           # 3 passed
cargo test -p cubism-iceberg --test concurrency                           # 4 passed, 1 ignored
cargo test -p cubism-iceberg --test durability                            # 7 passed
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings     # clean
cargo build -p cubism-cli                                                 # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings        # clean
cargo test --workspace --exclude cubism-py                                # 170 passed, 2 ignored (22 suites)
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
cargo test -p cubism-core                                                 # 92 passed (4 suites), unchanged from Phase 15
cargo tree -i arrow --workspace                                           # single unified arrow 58.3.0 (Milestone 7's premise)
cargo tree -p cubism-iceberg                                              # confirms same arrow-array/arrow-schema 58.3.0
```

## Primary files

- [`../docs/TIMESERIES_ROADMAP.md`](TIMESERIES_ROADMAP.md) ("Phase 5
  Milestones" section, Milestones 7-10, "Phase 5 'done' condition",
  updated scope/fallback text)
- [`../crates/cubism-iceberg/src/reader.rs`](../crates/cubism-iceberg/src/reader.rs)
  (`read_window`'s doc comment, additive correction paragraph at lines
  29-45, the function itself at line 46)
- [`../.claude/skills/timeseries-slice/SKILL.md`](../.claude/skills/timeseries-slice/SKILL.md)
  (step 1's Phase-5 parenthetical, corrected)
- [`TIMESERIES_PHASE_15_HANDOFF.md`](TIMESERIES_PHASE_15_HANDOFF.md) (prior
  handoff, superseded by this one)
- GitHub issue [`#8`](https://github.com/jeromebanks/cubism-rs/issues/8)
  (DataFusion 53/54 convergence — narrowed this session, still open for
  the SQL/pushdown half)
- GitHub issue [`#15`](https://github.com/jeromebanks/cubism-rs/issues/15)
  (roadmap-to-Phase-5 request — its second suggested step closed this
  session)
- GitHub issue [`#17`](https://github.com/jeromebanks/cubism-rs/issues/17)
  (append-committed-but-not-recorded recovery — still needs a design
  decision, read in full this session but not picked)
- GitHub issue [`#16`](https://github.com/jeromebanks/cubism-rs/issues/16)
  (event-time window identification — still needs a crate decision, read
  in full this session but not picked; possibly related to Milestone 7's
  new `cubism-datafusion`→`cubism-iceberg` dependency, not yet
  investigated)
- GitHub issue [`#10`](https://github.com/jeromebanks/cubism-rs/issues/10)
  (object store + Iceberg maintenance tracking issue — read in full this
  session, confirmed not yet split into shaped work)
