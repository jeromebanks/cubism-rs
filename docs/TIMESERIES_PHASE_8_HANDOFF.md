# Time-Series Phase 8 Handoff

Date: 2026-08-13

Branch: `feature/timeseries-phase-0a`

Status: **Milestone 1 (`LatenessPolicy`) landed and closed.** New pure type
`LatenessPolicy`/`Lateness` in `crates/cubism-core/src/temporal.rs`, one unit
test, full step-4 verification battery clean. `docs/TIMESERIES_ROADMAP.md`
updated: introduced the milestone `**Status:**` marking convention (none
existed before this session), marked Milestone 1 `Done`, recorded the plan
line 659 decision, and added an `AllowedLateness`/`IngestionTime` finding to
"Current state" that Milestone 3 (and later) should read instead of
re-deriving. Nothing else in Phase 4's scope moved this session — Milestones
2-6 are untouched. Committed and pushed to `feature/timeseries-phase-0a` —
see "Worktree state" below.

(Despite the filename, this doc documents a session slice, not "Phase 8" of
the implementation plan — same convention every prior handoff in this series
has used: plan Phase 5 is DataFusion range queries and serving; plan Phase 4
is now one milestone into its six-milestone roadmap. See
`docs/TIMESERIES_ROADMAP.md` for what "Milestone 1" means relative to plan
phases.)

**Superseded by:** (none yet — this is the latest handoff)

## What this session built

Read `docs/TIMESERIES_PHASE_7_HANDOFF.md` and `docs/TIMESERIES_ROADMAP.md`,
confirmed the branch in sync with `origin` (`git rev-list --left-right
--count` reported `0 0`), and read all 15 open issues. Per the roadmap's own
step-1 instructions (added last session), checked the milestone list before
any ad hoc deferred-list scan: Milestone 1 was the first (and only)
milestone not marked done, with no dependencies — the unambiguous pick.
Consulted the advisor before implementing, which confirmed the pick and set
four constraints: record the plan-line-659 decision *and* introduce a
done-marker convention in the roadmap (neither existed yet — the roadmap
defines no marking syntax, so nothing besides this session's own addition
tells a future slice "Milestone 1 is done"); default to
`crates/cubism-iceberg/src` for the type's location unless one specific
check overturns it; keep `classify` a pure function over caller-supplied
timestamps, not one that reaches into `PublicationStore`; and stay
two-valued (on-time/late) rather than silently also answering plan line 662
("too old, reject as backfill").

- **`crates/cubism-core/src/temporal.rs`** (modified): added `Lateness`
  (`OnTime`/`Late`) and `LatenessPolicy::{new, classify}`, placed
  immediately after `AllowedLateness`. Landed in `cubism-core`, not
  `cubism-iceberg`, on the advisor's discriminating check: `rtk proxy grep
  -rn "IngestionTime" crates/ | grep -v target` returned only
  `IngestionTime`'s own definition (line 41) and its `lib.rs` re-export —
  nothing in the workspace consumed it — while `temporal.rs`'s module doc
  (lines 4-5) already states "Ingestion time exists only for
  watermark/lateness decisions." An unclaimed purpose statement for a type
  already in `cubism-core::temporal` is evidence the classification concept
  belongs beside it, not in the Iceberg persistence crate. `classify`
  compares `event_time` against `bucket_end + allowed` only, with no I/O —
  no `PublicationStore`, no real ingestion-time watermark — but
  `event_time` itself functions as the caller's watermark position; the
  advisor's second pass caught that the first version of this doc comment
  said "does not consult... a watermark," which reads as if `event_time`
  were something else. Fixed: the doc comment now states plainly that
  `bucket_end` must be an already-closed *earlier* window than whatever
  bucket `event_time` would compute to on its own (passing an event's own
  `bucket_end` always yields `OnTime` — bucket membership already
  guarantees `event_time < bucket_end`, and `AllowedLateness` can't be
  negative). Also records plan line 659's decision (allowed-lateness is
  independent of window duration by construction — see the doc comment for
  the full reasoning) and plan line 662's non-scope (deliberately left
  open).
- **`crates/cubism-core/src/lib.rs`** (modified): re-exports `Lateness`,
  `LatenessPolicy` alongside the rest of `temporal`'s public surface.

**On scope:** `.claude/skills/timeseries-slice/SKILL.md`'s own frontmatter
says not to use this skill "for work outside `crates/cubism-iceberg` and its
docs." This session's source change lands in `crates/cubism-core/src/`, not
`crates/cubism-iceberg/`. That's a real boundary crossing, not something to
leave unremarked — the `IngestionTime` evidence above is why it's the
technically correct location (the concept's designated home already exists
in `cubism-core::temporal`, unclaimed by any consumer), and the roadmap
itself, which this skill's step 1 now consults first, explicitly scopes
Milestone 1's target to "`cubism-core` if it needs to be shared with
non-Iceberg readers" (the roadmap's original wording, before this session
resolved the "decide at implementation time" it left open). So: an
exception to the skill's stated scope line, made deliberately and for a
reason the roadmap already anticipated, not a silent drift — same pattern
Phase 7 used when it edited the skill's own step 1.
- **`docs/TIMESERIES_ROADMAP.md`** (modified): added a `**Status:**` line to
  every milestone (`Done — <handoff>` for Milestone 1, `Not started` for
  2-6) and a sentence in "How `timeseries-slice` step 1 should use this doc"
  naming that as the doc's marking convention going forward. Rewrote
  Milestone 1's target/what-it-does/test/done-when to describe what actually
  landed (previously speculative — "new type in `cubism-iceberg/src` (or
  `cubism-core`... decide at implementation time"). Added the
  `AllowedLateness`/`IngestionTime` finding to "Current state" (verified
  citations: `temporal.rs:377`, `:584`, `:41`, `:4-5` — the `:584` line
  moved from where it was when first read, `:530`, because this session's
  69-line insertion above it shifted it; re-verified after the edit rather
  than citing the pre-edit number).

## What was actually verified

One new unit test,
`lateness_policy_classifies_on_the_allowed_lateness_boundary`
(`crates/cubism-core/src/temporal.rs`, `mod tests`): with a 10-second
allowed-lateness bound and `bucket_end` at Unix epoch, an event one
microsecond before the deadline (`9_999_999` µs) classifies `OnTime`, and an
event exactly at the deadline (`10_000_000` µs) classifies `Late`. This
proves the half-open boundary arithmetic (`<` not `<=`) is correct at the
single point where an off-by-one would show up. It does **not** prove
anything about a real ingestion-time watermark, a real `PublicationStore`
interaction, or multi-window/concurrent classification — `classify` never
touches those, by design (see doc comment), so there is nothing further for
a unit test at this milestone to exercise.

The claim that `LatenessPolicy`/`CorrectionPlan`/`ReconciliationRecord`/
`CompactionPlan`/`RetentionPlan`/`WindowLease` did not exist anywhere before
this session (roadmap's "Current state," inherited from Phase 7) was not
re-verified this session — it was Phase 7's finding, unchanged by this
slice's own scope, which only added `LatenessPolicy` itself.

## GitHub issues touched

None. Checked Phase 7's deferred list against all 15 open issues per skill
step 0/3:

- Deferred item 1 (extend roadmap to Phase 5) is already `#15`'s own
  suggested next step and the roadmap's own "Deferred" section — not
  refiled.
- Deferred item 2 (implement Milestone 1) is this session's own code slice,
  not an issue to file.
- Deferred item 3 (the rollback-point milestone gap) is already tracked in
  the roadmap's "Phase 4 done" section and its own "Deferred" list, tied to
  whichever slice implements Milestone 6 — read `#13` and `#15` in full and
  neither's scope list mentions it, but it is not "untracked": the roadmap
  doc itself is where it's recorded, and it isn't actionable independent of
  Milestone 6. Filing a standalone issue now would be speculative ahead of
  that milestone, so none was filed — matching the skill's "filing nothing
  is an acceptable outcome" allowance.
- Deferred item 4 (`#10`/`#8`/`#9`/`#11`/`#12`/`#14`) unchanged, no new
  findings against any of them this session.

## Deferred / not done this session

1. **Milestone 2** (formalize `ExpectedRevision`, correction-shaped
   stale-rejection test) — the roadmap's next milestone, depends on nothing
   structurally new but should follow Milestone 1 (done now) so its test
   can plausibly describe the corrected write as "late." This is the next
   code slice per the roadmap.
2. **Milestones 3-6** — untouched, each still depends on earlier milestones
   not yet done.
3. **Extending the roadmap to plan-Phase 5** — still deferred, per the
   roadmap's own "Deferred" section; unchanged from Phase 7.
4. **The rollback-point milestone gap** — still an open question the
   roadmap flags under "Phase 4 done," not resolved this session (see
   "GitHub issues touched" above for why no issue was filed for it).
5. Everything already deferred as of Phase 7 (`#10`/`#8`/`#9`/`#11`/`#12`/
   `#14`) is unchanged — this session did not touch any of that scope.

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log` for
the exact hash — this doc deliberately doesn't hardcode it, same convention
this series adopted after `docs/TIMESERIES_PHASE_4_HANDOFF.md` needed a
follow-up commit to fix a self-referential hash). That commit contains:

- New: `docs/TIMESERIES_PHASE_8_HANDOFF.md` (this file).
- Modified: `crates/cubism-core/src/temporal.rs` (`LatenessPolicy`,
  `Lateness`, one new test), `crates/cubism-core/src/lib.rs` (re-exports),
  `docs/TIMESERIES_ROADMAP.md` (status markers, Milestone 1 details,
  `AllowedLateness`/`IngestionTime` finding), `docs/TIMESERIES_PHASE_7_HANDOFF.md`
  (added `**Superseded by:**` line), `docs/handoff_latest.md` (symlink
  repointed).
- Untouched: everything under `crates/cubism-iceberg/`, `crates/cubism-cli/`
  — no Iceberg-crate or CLI source changed this session.

Also present, deliberately uncommitted per prior-session convention:
`.serena/` (local tooling state), `examples/web_analytics_demo/events.csv`
(generated demo output).

## Tests (21 in `cubism-iceberg`, unchanged; 92 in `cubism-core`, +1 this session)

This session's new test lives in `cubism-core`, not `cubism-iceberg` — see
"What this session built" for why. `cargo test -p cubism-iceberg` stays at
21 (8 unit + 6 Phase-3 integration + 3 durability integration + 4
concurrency integration, all passing, unchanged from Phase 7).
`cargo test -p cubism-core` reports 92 passed (4 suites) this session.
`cargo test -p cubism-core lateness_policy` isolates the new test directly:
`1 passed, 91 filtered out` — non-circular confirmation that the named test
ran, passed, and that the other 91 are exactly this session's starting
count (not derived by subtraction alone). Not independently re-verified
against a Phase 7 baseline (Phase 7's handoff never ran `cargo test -p
cubism-core` directly, only the workspace total), but consistent with the
workspace-level count below (157 → 158, a +1 matching this session's one
new test).

## Verification performed

```text
cargo test -p cubism-core                                                 # 92 passed (4 suites)
cargo test -p cubism-iceberg                                              # 21 passed
cargo test -p cubism-iceberg --test concurrency                           # 4 passed
cargo test -p cubism-iceberg --test durability                            # 3 passed
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings     # clean
cargo build -p cubism-cli                                                 # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings        # clean
cargo test --workspace --exclude cubism-py                                # 158 passed, 1 ignored (+1 from Phase 7's 157)
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
```

## Primary files

- [`../crates/cubism-core/src/temporal.rs`](../crates/cubism-core/src/temporal.rs)
  (`LatenessPolicy`, `Lateness`, new test)
- [`../crates/cubism-core/src/lib.rs`](../crates/cubism-core/src/lib.rs)
  (re-exports)
- [`TIMESERIES_ROADMAP.md`](TIMESERIES_ROADMAP.md) (Milestone 1 marked done,
  status-marker convention introduced)
- [`TIMESERIES_IMPLEMENTATION_PLAN.md`](TIMESERIES_IMPLEMENTATION_PLAN.md)
  (Phase 4 spec, lines 582-670 — plan lines 659 and 662 specifically)
- [`TIMESERIES_PHASE_7_HANDOFF.md`](TIMESERIES_PHASE_7_HANDOFF.md)
- GitHub issue [`#13`](https://github.com/jeromebanks/cubism-rs/issues/13)
  (Phase 4 proper, this milestone's parent scope)
- GitHub issue [`#15`](https://github.com/jeromebanks/cubism-rs/issues/15)
  (roadmap origin)
