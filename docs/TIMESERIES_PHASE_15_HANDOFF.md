# Time-Series Phase 15 Handoff

Date: 2026-08-15

Branch: `feature/timeseries-phase-0a`

Status: **[#14](https://github.com/jeromebanks/cubism-rs/issues/14) (retry
loop has zero CI coverage) addressed — and a real bug in the retry loop
found and fixed along the way, not just a coverage gap closed.** With every
`docs/TIMESERIES_ROADMAP.md` milestone already `Done`, this session fell
back to the roadmap's own step-4 instruction to scan the open-issue/deferred
list directly. `#14` was the only deferred candidate already sized to one
bounded test with no open design decision (`#17` and `#16` both say in
their own bodies they need a decision first; `#10` says it isn't split into
shaped work yet). Writing the test `#14` asked for —
`retry_loop_resolves_a_write_lock_held_past_busy_timeout`
(`crates/cubism-iceberg/tests/concurrency.rs`, `#[ignore]`d, run explicitly)
— failed on every run, not from a timing issue but from
`with_immediate_tx`'s retry branch dropping a connection back into its pool
without a `ROLLBACK` after a failed `BEGIN IMMEDIATE`, poisoning the
store's single pooled connection for the very next attempt. Fixed in
`crates/cubism-iceberg/src/durable_control.rs`. This was a real,
previously-unverified correctness gap in code that has shipped since
Phase 4/5 — see "What this session built" for the exact before/after
evidence.

(Despite the filename, this doc documents a session slice, not "Phase 15"
of the implementation plan — same convention every prior handoff in this
series has used: plan Phase 5 is DataFusion range queries and serving; the
plan's Phase 4 (`docs/TIMESERIES_IMPLEMENTATION_PLAN.md:582-670`) has every
`docs/TIMESERIES_ROADMAP.md` milestone done but is not itself fully done —
see that roadmap's "Phase 4 done" section, unchanged by this session, for
what remains.)

**Superseded by:** `docs/TIMESERIES_PHASE_16_HANDOFF.md`, which extended
`docs/TIMESERIES_ROADMAP.md` to plan-Phase 5 (this doc's deferred item 4)
and left the rest of this doc's deferred list (`#17`, `#16`, `#10`, item
6's unverified `return Err(err)` branch) unchanged/re-confirmed.

## What this session built

Read `docs/TIMESERIES_PHASE_14_HANDOFF.md` and `docs/TIMESERIES_ROADMAP.md`,
confirmed the branch in sync with `origin` (`git rev-list --left-right
--count` reported `0 0`), and listed all open issues (`#1`-`#18`). Every
`docs/TIMESERIES_ROADMAP.md` milestone (1-6) is marked `Done`, so per the
roadmap's own step-4 instruction this session fell back to its ad hoc
deferred-list/issue scan instead of consulting a milestone list.

Read the full bodies of `#17` (`AwaitingAppend` recovery ambiguity), `#16`
(event-time window identification), `#10` (object store/maintenance
tracking issue), and `#14` (retry-loop CI coverage) before the advisor
call. `#17` and `#16` each state in their own body that they need a design
decision before they can be sized; `#10`'s own "Suggested next steps"
section says it isn't split into focused, shaped work yet. `#14` was the
only one already sized to this series' slice shape — one test, a concrete
suggested approach (hold a write lock past `busy_timeout` so the retry loop
is the only thing that can resolve it), and an explicit reason it hadn't
been added already (keeping the fast suite under 5s).

Consulted the advisor before writing any code. It confirmed `#14` as the
right pick and flagged the locking-model details that would have made a
naive version of the test either non-discriminating or wrong:

- **A copy of `publish_waits_for_a_concurrently_held_write_lock_then_succeeds`'s
  200ms hold does no discriminating work** — it passes identically whether
  the Rust retry loop fired or `busy_timeout` absorbed the whole wait. The
  hold needs to exceed `busy_timeout`'s 5s window, and the test needs to
  assert a *lower* bound on elapsed time (not just success) to prove the
  retry loop, not `busy_timeout`, was the resolving mechanism.
- **Not `tokio::test(start_paused)`** — the 5s wait happens inside SQLite's
  C busy handler on the connection's own thread, which a paused Tokio clock
  cannot advance, while it would advance `backoff`'s `tokio::time::sleep`,
  producing a hang or a misleading pass. Real-time or nothing.
- **`#[ignore]` has consequences for the step-4 battery and the count
  line** — the battery doesn't pass `--ignored`, so the new test proves
  nothing until run explicitly and recorded as a separate verification
  line; "What was actually verified" must say plainly that this path still
  has no *CI* coverage, since an `#[ignore]`d test doesn't close that half
  of `#14`'s title.

Writing the test to that spec surfaced a real bug on the first run (see
below), which needed a second advisor pass before the fix and its
verification could be trusted:

- **The first fix attempt's elapsed-time assertion was vacuous.** The
  `Instant::now()` call was placed around the spawned task, but the task
  was awaited only after this test's own 7s holder-release `sleep` — so
  `elapsed` measured that outer sleep, not how long `publish` itself
  blocked. Against the pre-fix broken code (which failed fast, at ~5.0s),
  the assertion still read ~7.0s and passed, defeating its own stated
  purpose. Fixed by timing from inside the spawned task and returning the
  duration alongside the result.
- **The module doc comment and the new test's own doc comment initially
  contradicted each other** on whether the new (`#[ignore]`d) test "closes"
  `#14`. Resolved in favor of the test's own, more accurate wording: it
  exercises `#14`, but being `#[ignore]`d it does not by itself close the
  "zero *CI* coverage" half of the issue's title.
- **`durable_control.rs`'s existing module doc comment became stale** the
  moment the bug was found — it described the retry loop as an
  untested-but-present backstop ("a gap, not a claim of coverage"); the
  truth was stronger and worse (it did not work at all for the `BEGIN`
  path, as shipped). Added a correction paragraph
  (`durable_control.rs:63-80`) rather than editing the original claim in
  place, matching this series' additive-correction convention.
- **The "poisoned for every subsequent call" claim about the sibling
  `return Err(err)` branch is reasoned from the code, not independently
  observed by a test** — reaching it requires exhausting `MAX_TX_ATTEMPTS`
  under sustained contention, which this session did not build a test for
  (it would need many retries, each up to 5s, well past one bounded
  slice's budget). Labeled as such rather than stated flat, per the
  advisor's explicit instruction.

Primary files changed:

- **`crates/cubism-iceberg/src/durable_control.rs`** (modified): the
  `with_immediate_tx` retry loop's failed-`BEGIN IMMEDIATE` branch
  (`durable_control.rs:396-415`) now issues a single `ROLLBACK`
  (`durable_control.rs:408`) before the retryable/non-retryable check, so
  it covers *both* exits from that branch — the retry-and-continue exit
  (`durable_control.rs:412`) and the terminal `return Err(err)` exit
  (`durable_control.rs:414`) — not just the retryable case the new test
  happens to force, since the same connection-poisoning mechanism applies
  to the terminal case too. Added a correction paragraph to the module doc
  comment
  (`durable_control.rs:63-80`) recording the observed bug and fix in place
  of editing the prior (now-inaccurate) claim that this path was merely
  untested.
- **`crates/cubism-iceberg/tests/concurrency.rs`** (modified): new test
  `retry_loop_resolves_a_write_lock_held_past_busy_timeout`
  (`concurrency.rs:434-436` for the attribute/signature), `#[ignore]`d,
  holding an external `BEGIN IMMEDIATE` for 7s (past `busy_timeout`'s 5s
  window) to force `with_immediate_tx`'s Rust-level retry loop, and
  asserting both success and a lower bound on `publish`'s own elapsed time
  (timed from inside the spawned task) to distinguish "the retry loop
  resolved it" from "the test's own timing let it through some other way."
  Its doc comment records the exact before/after error sequence this
  session observed. Module doc comment updated to reference the new test
  and its `#14`/bug-fix relationship, and to correct the earlier claim that
  it "closes" `#14`.

## What was actually verified

The new test, run explicitly (`cargo test -p cubism-iceberg --test
concurrency -- --ignored --exact
retry_loop_resolves_a_write_lock_held_past_busy_timeout`), proves: (1)
before the fix, holding a write lock past `busy_timeout`'s 5s window made
every `publish` call through `with_immediate_tx` fail permanently with an
unrelated, non-retryable "cannot start a transaction within a transaction"
error, instead of retrying — observed directly via the attempt/error
sequence (`SQLITE_BUSY, retryable=true` on attempt 1, then the transaction-
nesting error, `retryable=false`, on attempt 2), not reasoned from SQLite's
documentation; (2) after the fix, the same scenario succeeds, and does so
only after `publish` itself has blocked past the 5s `busy_timeout` mark
(asserted via a lower bound timed inside the spawned task, not around it),
confirming the Rust-level retry loop — not `busy_timeout` alone — is what
resolved the contention; (3) the fast suite (`cargo test -p cubism-iceberg
--test concurrency`, no `--ignored`) is unaffected — 4 passed, 1 ignored,
same as before this session.

It does **not** prove: full *CI* coverage of this path — the new test is
`#[ignore]`d and the step-4 battery does not pass `--ignored`, so `#14`'s
"zero CI coverage" framing is not closed by this session, only its "the
mechanism has never been shown to work" half; the `MAX_TX_ATTEMPTS` bound,
`backoff`'s exponential schedule beyond the single retry this test
exercises, or exhaustion behavior (every attempt failing) — this test
forces exactly one retry, not eight; the module doc comment's separately-
described COMMIT-`RESERVED`-to-`EXCLUSIVE`-escalation contention path
(blocked by a concurrent `.current()`/`.run_state()` reader outside a
transaction, a different scenario from this test's two-writers-on-one-lock
setup); or that the `return Err(err)` (exhausted-attempts/non-retryable)
exit from the same branch is actually poisoned the way the retry-and-
continue exit was observed to be — that symmetric fix was applied by
reasoning from the same root cause and the file's own established
`ROLLBACK`-before-return pattern used twice elsewhere in the function, not
independently exercised by a test this session.

## GitHub issues touched

- [#14](https://github.com/jeromebanks/cubism-rs/issues/14) — commented
  with the fix, the exact before/after error sequence, and an explicit note
  that the issue stays **open**: the new test is `#[ignore]`d, so the
  "zero CI coverage" half of the title is not closed, only the "does the
  mechanism actually work" half. Matches this repo's convention (see
  `docs/TIMESERIES_PHASE_14_HANDOFF.md`'s #18 note) of recording
  resolutions with an explicit comment/cross-link rather than closing via
  `gh issue close`.
- No new issues filed. `#17`, `#16`, and `#10` were read in full during
  triage and confirmed still not sized to a bounded slice (each says so in
  its own body) — not re-litigated or re-filed.

## Deferred / not done this session

1. **#17** (append-committed-but-not-recorded recovery) — unchanged; still
   needs a design decision before it can be sized into a bounded milestone.
2. **#16** (event-time window identification + recompute-equality proof) —
   unchanged; still needs a decision on which crate closes it.
3. **#10** (real object store + Iceberg maintenance) — unchanged; still not
   split into shaped, sizeable work per its own "Suggested next steps."
4. **Extending the roadmap to plan-Phase 5** — still deferred, unchanged
   from prior sessions.
5. **#11/#12** — unchanged; this session did not touch either scope.
6. **The `return Err(err)` branch's poisoning fix is unverified by a
   test** (see "What was actually verified" above) — reaching it needs
   `MAX_TX_ATTEMPTS` (8) exhausted under sustained contention, each attempt
   up to 5s; a test forcing that would run well past one bounded slice's
   time budget. Not filed as an issue — the fix itself is landed and
   low-risk (it mirrors the file's own established pattern), just not
   independently exercised.
7. **A per-window revision-listing API** — unchanged from Phase 14; no
   concrete consumer has asked for it yet.

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log`
for the exact hash — this doc deliberately doesn't hardcode it, the
convention adopted after `docs/TIMESERIES_PHASE_4_HANDOFF.md` needed a
follow-up commit to fix a self-referential hash). That commit contains:

- New: `docs/TIMESERIES_PHASE_15_HANDOFF.md` (this file).
- Modified: `crates/cubism-iceberg/src/durable_control.rs` (`ROLLBACK` fix
  on the failed-`BEGIN IMMEDIATE` branch, module doc correction),
  `crates/cubism-iceberg/tests/concurrency.rs` (new `#[ignore]`d test,
  module doc update), `docs/TIMESERIES_PHASE_14_HANDOFF.md` (added
  `**Superseded by:**` line).
- Untouched: `crates/cubism-core/`, `crates/cubism-cli/` (built and
  clippy-checked, not modified), `crates/cubism-iceberg/src/coordinator.rs`,
  `crates/cubism-iceberg/src/control.rs`, `crates/cubism-iceberg/src/correction.rs`,
  `docs/TIMESERIES_ROADMAP.md` (no milestone touched this session).

Also present, deliberately uncommitted per prior-session convention:
`.serena/` (local tooling state), `examples/web_analytics_demo/events.csv`
(generated demo output).

## Tests (33 passed + 1 ignored in `cubism-iceberg`; 170 passed / 2 ignored in workspace)

`cargo test -p cubism-iceberg` reports 33 passed, 1 ignored (6 suites): 13
unit (`--lib`, unchanged) + 6 Phase-3 integration (`--test phase3`,
unchanged) + 7 durability integration (`--test durability`, unchanged) + 4
passed/1 ignored concurrency integration (`--test concurrency`, +1 ignored
this session) + 3 coordinator integration (`--test coordinator`,
unchanged). All figures confirmed by running each suite in isolation as
well as the full `cargo test -p cubism-iceberg` run, not derived by
subtraction. The new test does not add to the "passed" count in the default
run because it is `#[ignore]`d; it is separately confirmed passing via
`--ignored --exact` (see "Verification performed" below).
`cargo test --workspace --exclude cubism-py` reports 170 passed, 2 ignored
(22 suites) — passed count unchanged from Phase 14's 170 (the new test adds
to "ignored," not "passed," in the default run), ignored count +1 from
Phase 14's 1 (the pre-existing `crates/cubism-timeseries-bench/src/lib.rs:1660`
ignored test plus this session's new one — confirmed via `rtk proxy grep
-rn "#\[ignore" crates/`, not assumed). `cargo test -p cubism-core` reports
92 passed (4 suites), unchanged from Phase 14 — confirmed by running it
directly.

## Verification performed

```text
cargo test -p cubism-iceberg                                              # 33 passed, 1 ignored (6 suites)
cargo test -p cubism-iceberg --lib                                        # 13 passed
cargo test -p cubism-iceberg --test phase3                                # 6 passed
cargo test -p cubism-iceberg --test coordinator                           # 3 passed
cargo test -p cubism-iceberg --test concurrency                           # 4 passed, 1 ignored
cargo test -p cubism-iceberg --test durability                            # 7 passed
cargo test -p cubism-iceberg --test concurrency -- --ignored --exact \
  retry_loop_resolves_a_write_lock_held_past_busy_timeout                 # 1 passed (~7.1s, run twice for stability)
cargo clippy -p cubism-iceberg --all-targets --no-deps -- -D warnings     # clean
cargo build -p cubism-cli                                                 # clean
cargo clippy -p cubism-cli --all-targets --no-deps -- -D warnings        # clean
cargo test --workspace --exclude cubism-py                                # 170 passed, 2 ignored (22 suites; +1 ignored from Phase 14's 1)
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings  # clean
cargo test -p cubism-core                                                 # 92 passed (4 suites), unchanged from Phase 14
```

## Primary files

- [`../crates/cubism-iceberg/src/durable_control.rs`](../crates/cubism-iceberg/src/durable_control.rs)
  (`with_immediate_tx`'s `ROLLBACK` fix on the failed-`BEGIN IMMEDIATE`
  branch, module doc correction paragraph at lines 63-80)
- [`../crates/cubism-iceberg/tests/concurrency.rs`](../crates/cubism-iceberg/tests/concurrency.rs)
  (new `retry_loop_resolves_a_write_lock_held_past_busy_timeout` test,
  module doc update)
- [`TIMESERIES_PHASE_14_HANDOFF.md`](TIMESERIES_PHASE_14_HANDOFF.md) (prior
  handoff, superseded by this one)
- GitHub issue [`#14`](https://github.com/jeromebanks/cubism-rs/issues/14)
  (retry-loop CI coverage — addressed this session, left open; see the
  issue comment for the full before/after evidence)
- GitHub issue [`#17`](https://github.com/jeromebanks/cubism-rs/issues/17)
  (append-committed-but-not-recorded recovery — still needs a design
  decision, read in full this session but not picked)
- GitHub issue [`#16`](https://github.com/jeromebanks/cubism-rs/issues/16)
  (event-time window identification — still needs a crate decision, read in
  full this session but not picked)
- GitHub issue [`#10`](https://github.com/jeromebanks/cubism-rs/issues/10)
  (object store + Iceberg maintenance tracking issue — read in full this
  session, confirmed not yet split into shaped work)
