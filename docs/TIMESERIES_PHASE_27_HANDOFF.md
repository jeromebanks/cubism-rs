# Time-Series Phase 27 Handoff

Date: 2026-08-18

Branch: `feature/timeseries-phase-0a`

Status: **Landed Milestone 12b** (`docs/TIMESERIES_ROADMAP.md`'s "POC
Milestones" section) — a real `/api/series` request/response pair
captured against a live `cubism serve` process, showing `avg_revenue`
change across Milestone 12a's revision-1 -> revision-2 correction, both
responses `is_exact: true`. (Naming note, carried forward from every
prior handoff in this series: this file's number is a *session-slice*
number, not a `docs/TIMESERIES_IMPLEMENTATION_PLAN.md` phase number —
this slice does POC-milestone work, not plan-phase work.)

`docs/TIMESERIES_PHASE_26_HANDOFF.md`'s deferred list named Milestone 12b
as the natural next slice (item 2), with dependency Milestone 12a already
`Done`. Advisor review (before any code) confirmed the pick and, since
orientation had already surfaced two seam questions the roadmap's
original 12b text glossed over, recommended resolving both empirically
before writing the script rather than reasoning about them:

1. **`cubism serve`'s required positional `cube_path` argument.** The
   roadmap's 12b text wrote the invocation as `cubism serve --spec ..
   --warehouse ..`, but `crates/cubism-cli/src/main.rs`'s `serve` always
   calls `CubeStore::from_path(cube_path)` first, unconditionally —
   confirmed by reading the function — regardless of whether `--spec`/
   `--warehouse` are also given. `/api/series`'s own handler
   (`crates/cubism-serve/src/series.rs`) never touches the loaded
   `CubeStore`. Advisor's recommendation (not just asserted, checked):
   don't depend on the *static* demo's `web_analytics_cube.parquet` (it's
   untracked, present only because this repo happened to have run that
   demo before — would silently fail from a genuinely clean checkout).
   Instead the script writes its own throwaway one-row placeholder
   parquet (`xunit` column, one `/G` row) via a `pyarrow` heredoc into
   `.temporal_build/`.
2. **Whether the demo needs one server process or two (restart between
   revisions).** Tested empirically before writing the script: with a
   `cubism serve --spec .. --warehouse ..` process already running against
   revision 1, a second, independent `iceberg-build` invocation published
   revision 2 while the server stayed up, and the very next `/api/series`
   request against that same server returned `published: [{"revision":
   2, ...}]` and the corrected value — no restart needed. This decided
   the script's shape: one server, two builds
   interleaved around two queries.

Advisor also flagged: use the single-window request (`window_id:
"2026-04-07"` alone), not a wider multi-day range — a 3-day range at `1d`
resolution collapses to one merged point (Milestone 11's own test
demonstrates this), which would blur exactly the before/after delta this
milestone exists to show.

**Superseded by:** `docs/TIMESERIES_PHASE_28_HANDOFF.md`, which picked up
this doc's deferred item 2 (Milestone 13) and landed it — closing the
roadmap's tracked milestone list at 18/18.

## What this session built

**`examples/web_analytics_demo/query_temporal_demo.sh`** (new) — a
standalone script, not `site/` wiring (the roadmap's original 12b text
offered either; a script proved cheaper). Does not require
`build_temporal_demo.sh` to have run first, and builds only what this
query needs: window `2026-04-07`, both revisions — not the two ordinary
days `build_temporal_demo.sh` also builds for its own demonstration. Both
scripts share the `.temporal_build/` output directory; each does its own
`rm -rf` at the top (same ephemeral-output convention Milestone 12a
established), so the last one run "wins" as that directory's current
state.

Sequence: generate the same seeded temporal event stream Milestone 12a
uses -> write the placeholder `cube_path` parquet -> `temporal-build` +
`iceberg-build` window `2026-04-07` at revision 1 from the initial
(late-events-held-back) stream -> start `cubism serve` in the background,
poll until it accepts connections -> POST `/api/series` and capture the
response ("before") -> `temporal-build` + `iceberg-build` the same window
at revision 2 from the full stream, against the same running server ->
POST `/api/series` again and capture the response ("after") -> kill the
server -> print a short before/after summary parsed from both captured
JSON files.

**`examples/web_analytics_demo/README.md`** — new "Query view (Milestone
12b)" section: the script invocation, the two captured raw JSON
responses verbatim, and a "what this does and does not prove" note
(narrower than a real client: no concurrent-request race coverage, no
resolution other than `1d`, no selector other than `/G` — same style of
narrowing Milestone 11 itself used).

**`docs/TIMESERIES_ROADMAP.md`** — Milestone 12b marked `Done`, its entry
rewritten to describe what was actually built (the script, the empirical
restart-vs-live finding, the single-window request shape) rather than the
original speculative text, with the "Done when" criterion's evidence
filled in inline.

## What was actually verified

`query_temporal_demo.sh`, run twice (once from the repo root, once from
`/tmp` to confirm cwd-independence — same check 12a's own advisor
follow-up applied to `build_temporal_demo.sh`), produced identical
request/response pairs both times:

```text
== /api/series BEFORE the correction (window 2026-04-07 at revision 1) ==
{"cube":"northstar_web_analytics_temporal","measure":"avg_revenue","points":[{"bucket_end":1775606400000000,"bucket_start":1775520000000000,"is_exact":true,"missing":[],"published":[{"revision":1,"window_id":"2026-04-07"}],"value":0.0}],"source_resolution":"1d"}

== /api/series AFTER the correction (window 2026-04-07 at revision 2, same server, no restart) ==
{"cube":"northstar_web_analytics_temporal","measure":"avg_revenue","points":[{"bucket_end":1775606400000000,"bucket_start":1775520000000000,"is_exact":true,"missing":[],"published":[{"revision":2,"window_id":"2026-04-07"}],"value":1.6638655462184875}],"source_resolution":"1d"}
```

`avg_revenue` for `/G` moves from `0.0` to `1.6638655462184875`. The
zero on revision 1 isn't an artifact — this seed's 6 late-arriving
`signup_completed` events happen to be *every* revenue-bearing event that
landed on window `2026-04-07`, so revision 1 (which holds them back) has
literally no revenue rows in view. Both responses report `is_exact: true`
and the correct real revision in `published`, and the second query was
answered by the same server process that answered the first — proving
the revision bump is visible over a real HTTP request, not just as
`control_runs`/`control_publications` rows (which is all Milestone 12a
itself proved).

**What this does not prove:** anything about a request racing a
concurrent publish (`crates/cubism-serve/src/series.rs`'s own module doc
comment already documents the read-after-plan non-atomicity this would
exercise — not touched here); any resolution other than `1d`; any
selector other than the global `/G` rollup; anything about
`LatenessPolicy`/`CorrectionPlan` (same scope narrowing Milestone 12a's
own entry already states — this session's build path is identical to
12a's in that respect).

This also satisfies `docs/TIMESERIES_PHASE_26_HANDOFF.md`'s deferred item
4 ("a real CLI end-to-end smoke test for `cubism serve --spec ..
--warehouse ..`") — that handoff predicted it would naturally fold into
Milestone 12b, which is exactly what happened here.

Since this slice touches no `.rs` file (same as Phase 26), the standard
test-count claims are **unchanged**: `cargo test -p cubism-iceberg`
reports 35 passed, 1 ignored (6 suites); `cargo test --workspace
--exclude cubism-py` reports 211 passed, 2 ignored (24 suites) — both
re-run and re-verified this session (not copied from Phase 26's numbers),
identical because nothing under any crate's `src/` or `tests/` changed.
`rustfmt` was not run this session — no `.rs` file was touched.

## GitHub issues touched

None. No new scope-narrowing beyond what's already documented inline
(the same `LatenessPolicy`/`CorrectionPlan` caveat Milestone 12a already
states, plus this session's own narrower ones listed above, all inline in
the README/roadmap rather than filed, following this series' existing
precedent). `#20` remains open and untouched, still the highest-priority
item on the deferred list below.

## Deferred / not done this session

1. **`#20`** (a delayed correction replay can silently undo an intentional
   rollback) — unchanged, still the most consequential open item; needs a
   design decision, not a mechanical fix.
2. **Milestone 13** (architecture/implementation documentation) — now
   unblocked: both 12a and 12b are `Done`. Natural next slice per the
   roadmap's own "should be written after Milestones 12a and 12b land"
   note.
3. **The TOCTOU gap** between `CoveragePlan`'s publication snapshot and
   `read_window`'s own re-resolution of `current` — unchanged, documented
   in `series.rs`'s module doc comment, not touched this session (no
   `.rs` file changed). This session's script exercises the *not*-racing
   case only.
4. **Concurrent-request coverage for `/api/series`** — this session
   proved sequential before/after visibility across one server process,
   not what happens if a request lands mid-publish. Not newly filed (see
   item 3 — same underlying gap, already tracked in the module doc
   comment, not yet a GitHub issue).
5. Every other item on `docs/TIMESERIES_PHASE_26_HANDOFF.md`'s deferred
   list (concurrent recovery of the same `run_id`, plan completion-
   criterion 774/the SQL-table-function decision gated on `#8`, `#16`,
   `#10`, `#9`, `#14`, `#4`, `#3`) — unchanged, none touched this session.

## Worktree state

**Committed and pushed** to `feature/timeseries-phase-0a` (see `git log`
for the exact hash — this doc deliberately doesn't hardcode it, per this
series' established convention).

- New: `examples/web_analytics_demo/query_temporal_demo.sh`,
  `docs/TIMESERIES_PHASE_27_HANDOFF.md` (this file).
- Modified: `docs/TIMESERIES_ROADMAP.md` (Milestone 12b marked `Done`,
  entry rewritten with what was actually built), `examples/web_analytics_demo/README.md`
  (new "Query view" section), `docs/TIMESERIES_PHASE_26_HANDOFF.md`
  (`Superseded by` line added), `docs/handoff_latest.md` (symlink
  retarget).
- Untouched: every crate under `crates/` — this slice changed no `.rs`
  file, confirmed via `git status` before staging and after commit.

Also present, deliberately uncommitted: `.serena/` (local tooling state,
prior-session convention); `examples/web_analytics_demo/events.csv` (the
*static* demo's own generated output, unrelated to this session,
prior-session convention); `examples/web_analytics_demo/.temporal_build/`
(regenerated by both `build_temporal_demo.sh` and `query_temporal_demo.sh`,
gitignored since Phase 26); `.claude/skills/timeseries-slice/SKILL.md`
(pre-existing uncommitted edits from before this session started —
unrelated to this slice's scope, left untouched, not staged).

## Tests (35 passed + 1 ignored in `cubism-iceberg`, unchanged; 211 passed / 2 ignored in workspace, unchanged)

Identical to Phase 26's counts across the board — this slice added one
new shell script and touched two docs, no `.rs` file. `cubism-cli` still
has no test harness of its own; its `cargo build`/`cargo clippy` legs are
clean, and this session additionally exercised it as a real end-user
would (`serve --spec .. --warehouse ..` against a live process, POSTed
twice around a real revision bump), which the standard battery alone
does not cover.

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
./examples/web_analytics_demo/query_temporal_demo.sh                       # real run, twice (repo root and /tmp) — identical output both times
```

Every `cargo` leg's full, non-truncated output was captured to a file
first and grepped for `test result:`/`error` afterward (not piped through
`tail` directly), per the skill's own step-4 note about a truncated
capture producing a wrong total before. Totals summed by hand from each
suite's own `test result:` line match Phase 26's totals exactly.

## Primary files

- [`../examples/web_analytics_demo/query_temporal_demo.sh`](../examples/web_analytics_demo/query_temporal_demo.sh)
  (new — the request/response capture script)
- [`../examples/web_analytics_demo/README.md`](../examples/web_analytics_demo/README.md)
  ("Query view" section)
- [`TIMESERIES_ROADMAP.md`](TIMESERIES_ROADMAP.md) (Milestone 12b marked
  `Done`)
- [`TIMESERIES_PHASE_26_HANDOFF.md`](TIMESERIES_PHASE_26_HANDOFF.md)
  (prior handoff, superseded by this one)
- GitHub issue [`#20`](https://github.com/jeromebanks/cubism-rs/issues/20)
  (untouched this session, still the top deferred item)
