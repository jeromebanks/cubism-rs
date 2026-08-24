# Time-Series Phase 30 Handoff

Date: 2026-08-24

Branch: `feature/timeseries-phase-0a`

Status: **Fixed [issue #21](https://github.com/jeromebanks/cubism-rs/issues/21)**
— the `AwaitingPublish` variant of #20's ABA replay gap, the top item on
Phase 29's deferred list. This slice also carried two explicitly ordered
housekeeping items: GitHub issue #20 (fixed in Phase 29 but left open)
was closed with a landing comment, and
`docs/TIMESERIES_IMPLEMENTATION_PLAN.md`'s stale status line ("proposed
plan, no production implementation started") was replaced with a
phase-by-phase status snapshot — the stale line is very likely why
fresh-context assistants misread this project's progress. (Naming note,
carried forward from every prior handoff in this series: this file's
number is a *session-slice* number, not a
`docs/TIMESERIES_IMPLEMENTATION_PLAN.md` phase number — this slice does
issue-fix and housekeeping work, not plan-phase or roadmap-milestone
work.)

The slice pick followed the roadmap's own fallback for an exhausted
milestone list (18/18 done): scan open issues and the latest handoff's
deferred list. #21 was chosen over #10 (object store/compaction) because
it is bounded to one slice and already issue-scoped, where #10 is a
multi-session effort. Advisor review before any code confirmed the pick
and reshaped the design materially: it caught that the identical ABA hole
exists one stage *earlier* than the filed scenario (`AwaitingAppend`
replays, not just `AwaitingPublish`), so the guard covers both non-
published stages; it vetoed carrying the new anchor inside the public
`RunState` enum; specified the `PRAGMA table_info` migration check over
error-swallowing; required the fresh-DB `CREATE TABLE` statements to gain
the columns natively; asked for one legacy-schema reopen test in the same
slice; and added the `current` field to the new error for parity with
`RunNoLongerCurrent`.

**Superseded by:** [`docs/TIMESERIES_PHASE_31_HANDOFF.md`](TIMESERIES_PHASE_31_HANDOFF.md)
— a decision session that closed Phase 0B outright (Rust selected, Spark
path descoped per the maintainer's call), closed #5 and #6, filed the
Phase 7 umbrella #22, and retargeted #1/#2/#4.

## What this session built

**The mechanism: per-window publication generations.** A revision-value
comparison cannot distinguish "nothing ever moved" from "moved, then was
rolled back to the same value" — after the operator's rollback in #21's
scenario, `current == A == observed_current`, so *any* value-only guard
passes and the CAS publish silently overrides the rollback. Both stores
therefore now track a monotonic per-window generation counter:
incremented by every *changing* publish (first publication, later
correction, or rollback republish — `crates/cubism-iceberg/src/control.rs`,
bump at the changing-write site; `crates/cubism-iceberg/src/durable_control.rs`,
computed alongside the upsert); deliberately **not** incremented by the
idempotent same-revision early-return nor by rejected CAS attempts.
Counting idempotent republishes would let an unrelated no-op replay
permanently poison every concurrently-recovering run's anchor.

**Claim-time anchoring** (`control.rs`'s `claim_generations` map, written
on the fresh-insert path inside the same mutex critical section as the
insert; `durable_control.rs`'s nullable `observed_generation` column,
written transactionally inside the claim's `BEGIN IMMEDIATE`). The anchor
records what the window looked like when the run was first claimed; an
existing run keeps its original observation (`ON CONFLICT DO NOTHING`),
and the column is never updated afterward, so a replay can trust it.
Deliberately *no* validation of `observed_current` against live `current`
at claim time — pure recording, so a stale caller anchor still fails
exactly as today (`StaleRevision` at publish).

**New public API** on `PublicationStore`: `publication_generation`
(live counter, 0 while unpublished) and `run_observed_generation`
(`None` = unknown run or pre-migration row). In-memory:
`control.rs:258` and `control.rs:265`; dispatchers at `control.rs:384`
and `control.rs:398`; SQLite: `durable_control.rs:439` and
`durable_control.rs:454`. `RunState`'s public shape is frozen — the
anchor travels beside it, not inside it.

**Schema migration** (`durable_control.rs:186` and `:191`, existence
check via `sqlite_column_exists` at `durable_control.rs:553`): fresh
databases get both columns natively in their `CREATE TABLE` statements;
pre-existing databases get them via `ALTER TABLE ADD COLUMN`, gated on a
`pragma_table_info` check (SQLite has no `ADD COLUMN IF NOT EXISTS`, and
matching ALTER's duplicate-column error text is brittle across sqlx
versions). Legacy rows keep `NULL` anchors.

**The guard** (`coordinator.rs:427`, error variant
`WindowChangedSinceClaim` at `error.rs:78`): for any non-`Published`
classification — both `AwaitingAppend` and `AwaitingPublish` — `execute`
refuses with `WindowChangedSinceClaim { run_id, window_id,
observed_generation, current_generation, current }` when the window's
live generation differs from the run's recorded anchor. A `None` anchor
falls through unprotected rather than refusing: refusing would permanently
break crash recovery for any run claimed before an upgrade — the exact
path this function exists to serve. The #20 `Published`-arm guard above
it is untouched and stays value-based (its benign-replay contract survives
generation counting because a run's own publish is the last *changing*
write in the benign case, and subsequent replays hit the early-return,
which does not bump).

**Residual gaps, documented in the guard's comment rather than fixed:**
(1) staleness that predates the claim itself — an A→C→A cycle completing
entirely between request planning and the first `execute` — is invisible
to a claim-time anchor by construction (byte-for-byte today's behavior;
fixing it needs the request envelope to carry a generation, an API
change); (2) the generation read and the CAS publish are separate store
calls, so a concurrent rollback-to-same-value slipping between them
reopens a narrow race — the same exposure class the plain CAS already
has. Closing that fully needs a generation-aware CAS inside `publish`.

**Housekeeping:** #20 closed on GitHub (fix had landed in Phase 29's
e060ee2 but the issue was never closed);
`docs/TIMESERIES_IMPLEMENTATION_PLAN.md` line 3's status line replaced
and a phase-status snapshot table inserted below it (Phases 0A/1–4/
5-functional landed; 0B partial with the Spark decision pending; 6–7 not
started; release progression at stage 2 of 7).

## What was actually verified

Three new tests:

1. `tests/coordinator.rs`'s
   `coordinator_rejects_a_replayed_awaiting_publish_correction_after_a_rollback_restored_its_observed_current`
   (line 575) — reproduces #21's numbered sequence end to end against the
   in-memory backend: initial publish (generation 1), correction staged
   to `AwaitingPublish` via direct claim/append/record calls (the same
   staging `coordinator_skips_a_redundant_append_when_the_run_was_already_appended`
   uses), a second real correction publishing revision 3 (generation 2),
   the operator's deliberate rollback to revision 1 (generation 3), then
   the original request replayed through `execute`. Asserts
   `WindowChangedSinceClaim { observed_generation: 1, current_generation:
   3, current: Some(1) }` — the generation delta proves the refusal fired
   on movement, not value — and that `current` stays at revision 1.
2. `tests/coordinator.rs`'s
   `coordinator_completes_an_interrupted_correction_when_nothing_changed_since_claim`
   (line 691) — the benign side: the same staged interruption, replayed
   with nothing else having touched the window, completes its publish and
   returns the correct `Publication`; the returned
   `aggregate_snapshot_id` equals the staged append's own snapshot ID,
   directly proving no second append occurred.
3. `tests/durability.rs`'s
   `sqlite_control_store_migration_adds_generation_columns_and_preserves_recovery`
   (line 1011) — hand-builds exactly the pre-#21 two-table schema with a
   mid-recovery (`appended`) row and a live publication, reopens through
   the real `PublicationStore::sqlite` entry point, and asserts: the
   migrated schema answers queries against both new columns; the legacy
   row's anchor reads back `None` (the fall-through case, not a refusal);
   the interrupted run still publishes across the upgrade; and a claim
   taken post-migration carries a real anchor (`Some(1)`).

**What these do not prove:** the coordinator-level *refusal* path against
the SQLite backend specifically (deliberately deferred — the durability
suite's existing legs are the regression net for the benign SQLite paths
since every claim now writes the new columns; a dedicated SQLite refusal
leg would mirror #20's precedent but was trimmed to keep the slice
bounded); the `AwaitingAppend` arm has no dedicated end-to-end test (the
guard covers it by construction — `!matches!(record, Published)` — and
every fresh-claim execution exercises that path's fall-through, but no
test reproduces an `AwaitingAppend` *replay* across a moved window);
either residual gap above; concurrent migration by two handles racing
`open()`.

Full-suite counts, re-verified this session: `cargo test -p
cubism-iceberg` reports **40 passed, 1 ignored (6 suites)** — up from 37,
matching the three new tests exactly (13 unit + 4 concurrency + 7
coordinator + 10 durability + 6 phase3 + 0 doctests). `cargo test
--workspace --exclude cubism-py` reports **216 passed, 2 ignored (24
suites)** — up from 213, same delta. The workspace total was summed from
the captured log's per-suite `test result:` lines, not read off a
truncated tail. All four prior #20-era coordinator tests pass unchanged.

## GitHub issues touched

- **[#21](https://github.com/jeromebanks/cubism-rs/issues/21)** — fixed
  this session (comment documenting the fix and the residual gaps posted
  at push time).
- **[#20](https://github.com/jeromebanks/cubism-rs/issues/20)** — closed
  this session (housekeeping; fix landed in Phase 29).
- `docs/TIMESERIES_IMPLEMENTATION_PLAN.md` — stale status line replaced
  with a maintained snapshot table (no issue; user-directed maintenance).

## Deferred / not done this session

1. **A SQLite-backend coordinator refusal leg** for the new guard —
   mirrors #20's precedent, deferred this slice; candidate to fold into
   whatever next touches `tests/durability.rs`.
2. **An `AwaitingAppend`-arm repro test** (claim recorded, crash before
   append completes, intervening correction-plus-rollback, replay) — the
   guard covers the arm, but only the `AwaitingPublish` shape has a
   dedicated test.
3. **Hoist the generation guard above the append branch** (advisor
   follow-up finding, dispositioned as deferred): a replay doomed to be
   refused currently commits its Iceberg append first — purely additive
   and never published, so no safety hole, but an orphaned snapshot and
   wasted writes. Hoisting is safe for fresh claims (they pass the guard)
   but changes ordering semantics and wanted its own look, not a
   mid-review move.
3. **Hole 1 residuals** (see the guard's comment): pre-claim staleness
   needs a request-envelope generation (API change); the
   check-vs-publish race needs a generation-aware CAS inside `publish`.
   Neither is tracked as an issue yet — file if/when the coordinator gets
   real callers (#11's CLI is the first candidate).
4. **#10** (compaction/retention/object-store) — untouched, still the
   largest open item and the hard prerequisite for cloud ambitions.
5. **#12** (Postgres/MySQL catalog/control store) — the new columns'
   migration logic is SQLite-specific; a second durable backend will need
   its own equivalent of the `pragma_table_info` gate.
6. Carried from Phase 29: #16, #14, #9, #8, #3, and the Phase 0B cluster
   (#1, #2, #4, #5, #6) — unchanged.

## Worktree state

Committed and pushed to `feature/timeseries-phase-0a` (see `git log` for
exact hashes; this doc deliberately doesn't hardcode them, per this
series' convention).

- Modified: `crates/cubism-iceberg/src/control.rs`,
  `crates/cubism-iceberg/src/durable_control.rs`,
  `crates/cubism-iceberg/src/error.rs`,
  `crates/cubism-iceberg/src/coordinator.rs`,
  `crates/cubism-iceberg/tests/coordinator.rs`,
  `crates/cubism-iceberg/tests/durability.rs`,
  `docs/TIMESERIES_IMPLEMENTATION_PLAN.md` (status line + snapshot
  table), `docs/TIMESERIES_PHASE_29_HANDOFF.md` (`Superseded by` line
  added), `docs/handoff_latest.md` (symlink retarget).
- New: `docs/TIMESERIES_PHASE_30_HANDOFF.md` (this file).
- Untouched: every other crate; `docs/TIMESERIES_ROADMAP.md` (this slice
  is issue work, not milestone work — the roadmap's milestone list
  remains exhausted at 18/18).

Also present, deliberately uncommitted, prior-session convention:
`.serena/` (local tooling state);
`examples/web_analytics_demo/events.csv` (static demo output);
`.claude/skills/timeseries-slice/SKILL.md` (pre-existing uncommitted
edits); `docs/CODEX_TELEMETRY_ROADMAP.md` (untracked, unrelated to this
slice's scope).

## Tests (40 passed + 1 ignored in `cubism-iceberg`, up from 37; 216 passed / 2 ignored in workspace, up from 213)

## Verification performed

```text
cargo test -p cubism-iceberg                                              # 40 passed, 1 ignored (6 suites)
cargo test -p cubism-iceberg --test concurrency                           # 4 passed, 1 ignored
cargo test -p cubism-iceberg --test durability                            # 10 passed
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings      # clean (after collapsible_if fix)
cargo build -p cubism-cli                                                  # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings          # clean
cargo test --workspace --exclude cubism-py                                 # 216 passed, 2 ignored (24 suites), exit 0
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
```

The workspace run was captured to a file first and grepped for
`test result:` afterward (24 suites, all `ok.`), not piped through
`tail`; totals were summed from those lines. `rustfmt --edition 2024
--check` on all six touched `.rs` files reported pre-existing diffs
consistent with [#3](https://github.com/jeromebanks/cubism-rs/issues/3)'s
known toolchain drift, so per the skill's step-2 gate, plain `rustfmt`
was not run; new lines were hand-written to match surrounding wide-line
style. **Correction from the advisor follow-up round:** this session's
first pass also claimed none of the session's own lines appeared in any
fmt diff — that check (a keyword grep over the diff) missed one
session-introduced violation, where an edit had collapsed
`fetch_run_row`'s signature and body onto one line in
`durable_control.rs`; found by the advisor's review, fixed in the
follow-up commit. Lesson recorded: verify fmt diffs by reading them, not
by grepping for expected keywords. The advisor round also corrected an
inverted comment about concurrent-migration behavior at the ALTER site
(sequential reopens are idempotent; two racing handles make one ALTER
fail loudly with duplicate-column — not silently idempotent) and tightened
the migration test's doc-comment phrasing; its remaining findings were
dispositioned without code changes: guard-below-append placement
(refused `AwaitingAppend` replays still commit their Iceberg append —
additive-only, no safety hole; hoisting is deferred as a candidate next
slice item), `format!`-built PRAGMA query (table names are call-site
literals only; documented in place), and the handoff's "committed and
pushed" wording (accurate as of push).

## Primary files

- [`coordinator.rs`](../crates/cubism-iceberg/src/coordinator.rs) (the
  guard, both arms)
- [`control.rs`](../crates/cubism-iceberg/src/control.rs) (in-memory
  generations + anchors)
- [`durable_control.rs`](../crates/cubism-iceberg/src/durable_control.rs)
  (schema, migration, SQLite generations)
- [`error.rs`](../crates/cubism-iceberg/src/error.rs)
  (`WindowChangedSinceClaim`)
- [`tests/coordinator.rs`](../crates/cubism-iceberg/tests/coordinator.rs)
  (#21 repro + benign recovery)
- [`tests/durability.rs`](../crates/cubism-iceberg/tests/durability.rs)
  (migration reopen)
- [`TIMESERIES_PHASE_29_HANDOFF.md`](TIMESERIES_PHASE_29_HANDOFF.md)
  (prior handoff, superseded by this one)
- GitHub issue [#21](https://github.com/jeromebanks/cubism-rs/issues/21)
  (fixed this session)
