# Time-Series Phase 36 Handoff

Date: 2026-08-28

Branch: `feature/timeseries-phase-0a` @ `78982f6` — **the only outstanding
branch.** PR #24 into `main`, 96 commits, 142 files, `MERGEABLE`.

**Superseded by:** (none yet — this is the latest handoff)

## How this lands on `main`

This is the first thing to know, and it is simpler than the Phase 35
handoff made it sound.

`origin/main` (`961b116`, "dogfooding", 2026-07-13) is a **strict ancestor**
of this branch — verified with `git merge-base --is-ancestor`. `main` has
no commits this branch lacks. So:

- PR #24 merges to `main` cleanly. No rebase, no conflicts.
- There is **no base-branch decision to make.** Phase 35's "First decision,
  before anything else" agonized over whether to target `main` or
  `feature/timeseries-phase-0a`; that dilemma was an artifact of there
  being two branches, and there is now one.

The consolidation happened this session: `feature/timeseries-correction-engine`
fast-forwarded into `feature/timeseries-phase-0a` (it was a strict ancestor,
so the ref moved with no merge commit), and both it and
`feature/timeseries-demo-multi-xunit` (already merged via PR #23) were
deleted locally and on the remote. Nothing was lost — both were fully
contained.

**Consequence worth noting:** PR #24 now closes **#7, #9, #13, #15, and
#16** on merge. Phase 35 said #9 and #16 were "closed"; they were done but
sat on a separate branch, so #24 would not have closed them. It will now.

## Does anything block the merge?

**No open issue does.** This was checked against the tracker, not assumed:

- **#43 (CI)** — its own exit criteria reads "PR #24 **or** a representative
  multi-crate branch passes the full gate." CI does not have to be
  validated on #24, and GHA is far easier to iterate on once it is on the
  default branch.
- **#3 (rustfmt non-reproducible)** and **#14 (retry-loop coverage)** belong
  with the CI work, not ahead of it.
- Phase 35's "Blocks the merge" list is **stale**: the repo owner's own PR
  #24 comment (2026-08-28 19:23Z) moved CI to #43 and #7/#13/#15
  reconciliation to #38. Read that comment before re-deriving the list.

The remaining gate is human: #24 has `reviewDecision: ""` and no status
checks. Two reviews were run this session (below) and one High finding from
them is **not** fixed — see "The one open question" .

## What this session did

### Consolidated the branches (above), then landed working files (`7d85587`)

Four files had been carried uncommitted across many sessions as a "standing
convention" recorded in every handoff. **That convention is retired** — it
meant real content lived on one laptop and was invisible to the PR.

- `docs/CODEX_TELEMETRY_ROADMAP.md` (748 lines) — now tracked.
- `examples/web_analytics_demo/events.csv` (2,535 rows) — the demo dataset
  the multi-XUnit dashboard charts. **PR #23 merged the dashboard without
  the data it reads.**
- `.claude/skills/timeseries-slice/SKILL.md` — step 0 roadmap snapshot,
  step 1 pre-implementation announcement, step 9 delta.
- `.gitignore` — `.serena/` added (tool state, not project content).

### Fixed the partial-correction guarantee (`78982f6`)

`CorrectionEngine::correct` **documented a resumability guarantee it did
not deliver.** `outcome` was a local accumulator and every per-window
failure propagated with `?`, discarding it. A mid-run failure returned the
bare error, so a caller could not know which windows had already been
republished; a retry restarted, re-correcting corrected windows and burning
a revision on each.

This mattered more than an ordinary bug: Phase 35 names this exact semantic
"the semantic a reviewer must agree with, not discover," and the
per-window-commit design (no cross-commit transaction exists in this stack)
is defensible *only* because the corrected prefix is reportable.

The fix is one new `CorrectError::Partial { corrected, skipped_unpublished,
source }`. The alternatives and why they lose on complexity are recorded in
the commit message — the load-bearing one is that returning `Ok(outcome)`
with a `failed` field removes `?`'s forcing function, so a caller who
ignores the field believes a failed run succeeded.

Progress is reported on the **error** path deliberately. `CorrectionOutcome`
comes back only when every window succeeded.

Also fixed in the same commit: `WindowRevision::new(observed_current.get()
+ 1)` had an unchecked `+ 1` mapped to the wrong error variant. Now
`checked_add` plus `RevisionOverflow`.

**The test induces a real mid-run failure, not a mock.** Run IDs are
deterministic (`{prefix}-{window}-r{revision}`), so pre-claiming the second
window's run ID with a mismatched `expected_rows` makes its `claim_run`
fail with `RunConflict` while the first window lands. It asserts the prefix
is both *reported* and *real* (2026-04-06 at revision 2, the failed window
still at 1). Mutation-checked: reverting the fix fails it with exactly the
pre-fix behaviour, a bare `Persist(RunConflict)` carrying no progress. No
`MUTANT` markers remain.

## Two reviews were run on #24

An advisor review and an independent Codex review
(`codex resume 01a04a64-842a-7762-abf7-be7964846f0e`). **Codex's verdict was
"request changes before merge."** Its two High findings were the partial-run
defect (fixed above) and #50 (not fixed).

Codex confirmed the partial-progress defect was **broader than first
stated** — five per-window `?` paths could discard progress, not two.

What both reviews **cleared**, so the next session need not re-derive it:

- CAS-anchor ordering (`observed_current` read before the rebuild).
- Half-open bucket math, including the `end - 1` adjustment.
- Pre-epoch bucket math (`div_euclid`/`rem_euclid`).
- V1/V2 payload-offset enforcement — a V2 blob mislabeled V1 keeps four
  extra bytes and is caught by the exact-length check. The "same payload
  offsets" invariant the doc asserts in prose is genuinely enforced.
- The recompute-equality proof is non-tautological for correction
  orchestration (separate warehouses, different publication protocols, real
  vacuity guards). It is **not** an independent oracle for aggregation
  semantics — both sides call the same `build_temporal`.

## The one open question before merge

**#50 — the canonical `WindowId` encoding is not injective.** Verified
empirically this session, not read off a review:

```text
PROBE bucket 0us   -> 1970-01-01T00:00:00Z
PROBE bucket 500ms -> 1970-01-01T00:00:00Z
PROBE collide = true
```

`window_id_for` formats the non-day case as `%Y-%m-%dT%H:%M:%SZ`, dropping
fractional seconds, while `cubism-core` accepts `us`/`µs`/`ms` resolution
units. `WindowId` is the publication key, so two buckets sharing one means a
correction can target the wrong window, and the per-window CAS loses its
scoping.

Not reachable through the demo (daily buckets), so it is latent rather than
live — but `windows.rs` declares this encoding **canonical**, and a
sub-second warehouse would be silently wrong.

**Recommended fix, deliberately minimal:** emit microsecond precision only
when the bucket start has a sub-second remainder. Every id current
warehouses hold is day-form or whole-second-form, so nothing changes for
them and `day_form_matches_the_existing_demo_convention` still holds. No
migration. Encoding signed Unix microseconds outright is the alternative,
but it breaks that pin.

The owner was asked whether to fix this before merging and chose to commit
the partial-correction fix and file the rest. **So the merge decision on #50
is still open** — it is a latent High in a module the Phase 35 handoff
itself flagged as most worth a reviewer's disagreement.

## Issues filed this session

| # | Sev | What |
|---|---|---|
| #50 | High | `WindowId` not injective for sub-second resolutions (above) |
| #51 | Med | `affected_windows` count calc `span / width + 1` unchecked; can panic or silently return zero windows |
| #52 | Low | V2 version-byte corruption reports "unsupported version", defeating #9's stated rationale; the existing test asserts the wrong behaviour |
| #53 | Low | `AverageState::decode` accepts `count == 0` with non-zero `sum`; `merge` then folds the orphaned sum in |
| #54 | Low | Recompute-equality proof never compares registry contents, though its fixture exists to exercise exactly that |
| #55 | Cosmetic | Checksum error message has ~22 literal spaces from lost line continuations |
| #56 | Debt | `correct`'s 8 positional params should become a request struct **before** #40's CLI hardens the call shape |
| #57 | Open Q | `LatenessPolicy` never consulted; intent undocumented — reclassified by Codex as a #40 admission-layer gap, not a `correct` defect |

## Verification performed

```text
cargo test --workspace --exclude cubism-py   # 243 passed, 2 ignored, 27 suites (was 242)
cargo clippy --workspace --exclude cubism-py --all-targets --no-deps -- -D warnings   # clean
cargo test -p cubism-correct                 # 9 passed (was 8)
```

The 242-baseline was reproduced independently before any change, so Phase
35's "229 tests pass" style claim is no longer a single-laptop assertion for
this branch.

`examples/web_analytics_demo/events.csv` becoming tracked does not affect
tests — it is referenced only in a shell-script comment (checked).

## Next session, ordered

1. **Decide #50 before merging** (or decide explicitly to merge without it).
   It is ~20 lines plus tests.
2. **Merge PR #24.** Clean fast-forward; closes #7, #9, #13, #15, #16.
3. **#3 then #43** on a branch off `main` — pin the toolchain so "fmt clean"
   is reproducible, *then* write the CI workflow that depends on it. Note
   #14's title presumes a CI that has never existed; re-scope it in that
   light.
4. **Repo hygiene, cheaper now than later** (none are tickets):
   `crates/cubism-iceberg-spike` is a default workspace member with no
   dependents and would build in CI; `TIMESERIES_FINISH_PROMPT.md` sits at
   the repo root; two broken `](../../PLAN.md)` links in
   `TIMESERIES_IMPLEMENTATION_PLAN.md:36` and `TIMESERIES_FEASIBILITY.md:7`;
   38 of 61 files in `docs/` are handoffs and want a `docs/handoffs/`
   subdirectory; `spark-adapter/` needs an explicit ships-or-archived call.
5. **Answer why `--exclude cubism-py` exists.** #43 names Python bindings as
   a required check, so this is now direct input rather than trivia.
6. **#40 — the correction CLI**, starting with #56 (the request struct).
   Then `--dry-run` over the existing `plan()`, then execute. The CLI must
   *surface* `CorrectError::Partial` — which windows landed, which did not,
   what to re-run. Hiding it converts a deliberate design decision into a
   silent partial failure.
7. Then #8 (DataFusion `TableProvider`), Phase 6 (rolling comparisons), and
   the production substrate (#10, #11, #12).

## Primary files

- [`../crates/cubism-correct/src/engine.rs`](../crates/cubism-correct/src/engine.rs)
  — `CorrectError::Partial`, per-window commit semantics
- [`../crates/cubism-correct/src/windows.rs`](../crates/cubism-correct/src/windows.rs)
  — the canonical encoding; **#50 lives here**
- [`../crates/cubism-correct/tests/recompute_equality.rs`](../crates/cubism-correct/tests/recompute_equality.rs)
  — the equality proof plus the new partial-progress test
- [`../crates/cubism-core/src/aggregate_state.rs`](../crates/cubism-core/src/aggregate_state.rs)
  — V2 framing; #52, #53, #55 live here
- [`TIMESERIES_PHASE_35_HANDOFF.md`](TIMESERIES_PHASE_35_HANDOFF.md) — prior
  handoff. Its "First decision" and "Blocks the merge" sections are both
  superseded by this file.

## A note on handoffs

Phase 35 opened by deferring the base-branch decision to the owner. The
owner's response was that the handoff should have explained how to get
everything merged instead. The question was answerable from the repo — one
`git merge-base --is-ancestor` call — and the "blockers" it listed had
already been re-tracked by the owner in a PR comment the handoff predated.

So: **lead with the merge path.** Check branch ancestry and re-read the PR's
own comments before writing. Reserve questions for what the repo cannot
answer — like #50's risk/benefit call above.
