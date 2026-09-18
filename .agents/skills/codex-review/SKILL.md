---
name: codex-review
description: Run the independent Codex review that produces a merge-gate review receipt for a pull request head. Called by work-slice step 8; not invoked directly.
---

# Independent Codex review of a pull request head

This owns one step: a fresh agent reviews a pushed head, and its verdict becomes
a SHA-bound receipt the merge gate honours. `.agents/skills/work-slice/SKILL.md`
step 8 delegates here; nothing else should reimplement the invocation.

There is no `.claude/skills/codex-review/` pointer. This is not user-invocable —
`work-slice` calls it. Harness-neutral: a Codex, Claude, or other session
following this file gets the same receipt. Only the optional pass in the last
section depends on a Claude Code plugin being installed, and it is optional
precisely so nothing is lost without it.

## What the caller supplies

| Value | Meaning |
| --- | --- |
| `$PR` | the pull request number |
| `$N` | the issue the PR closes |
| `$HEAD_SHA` | the head the review must read, from `gh pr view "$PR" --json headRefOid -q .headRefOid` |
| round | which attempt this is, and the verdicts of previous rounds |

## The independence rule

The reviewer must be a different agent session from the implementer, and the
implementing session must never author the verdict. It runs the command and
derives the verdict from the run's own output — that is not the same thing as
writing it.

The merge gate compares the receipt's `--reviewer` against the head commit's
`Agent-Session:` trailer. Both come from the runs themselves, never from
judgement.

## 1. Run the review

```bash
REPORT="${TMPDIR:-/tmp}/review-pr$PR.md"
codex exec --sandbox read-only --skip-git-repo-check "<prompt from section 2>" \
  < /dev/null > "$REPORT" 2> "$REPORT.err" \
  || { echo "review did not run; see $REPORT.err"; exit 1; }
test -s "$REPORT" || { echo "empty report; see $REPORT.err"; exit 1; }
```

All three redirections matter.

`< /dev/null` is required, not tidiness. `codex exec` appends stdin to the
positional prompt, so a caller that leaves stdin open rather than closing it or
attaching a terminal blocks indefinitely on
`Reading additional input from stdin...` and produces nothing.

The two streams go to separate files. `codex exec` writes progress,
configuration and token counts to stderr, so folding it into stdout corrupts the
report the receipt records; discarding it turns a failed run into a silent pass.
Stdout alone is exactly the review.

Those two guards are not decoration. Without them a review that never ran is
indistinguishable from one that passed.

## 2. The prompt

The reviewer fetches its own material. Anything this session pastes is a claim,
not evidence.

```text
You are an independent reviewer for pull request #$PR in <repo>. You are NOT the
agent that wrote this code. Review it on its merits; do not assume any claim in
the PR body or in a prior review is true.

First, confirm you are reviewing the right commit. Run `git rev-parse HEAD`. It
must equal $HEAD_SHA. If it does not, stop immediately and reply with only:
VERDICT: fail

DO NOT run the repository's quality gate, cargo build, cargo test, or any other
compilation. They take many minutes and CI results are already on the PR. You
may run `python3 scripts/tests/test_sdlc.py`, which is stdlib-only and takes
about two seconds.

Obtain the material yourself:
  git diff origin/main...HEAD
  git log origin/main..HEAD
  gh issue view $N
  gh pr view $PR --comments

<round context: which round this is, and each previous round's commit and
verdict. Prior receipts are PR comments; tell the reviewer to read them.>

Check at least:
<numbered, specific to the slice's acceptance criteria>

Report numbered findings. Each must cite file:line and state concretely what
goes wrong. If you find nothing material, say so explicitly.

Then end your response with exactly one final line, and nothing after it:
VERDICT: pass
or
VERDICT: fail
```

Findings come first; the verdict is the last line. `review-receipt` does not
parse it — section 3 does, which is why the contract has to be unambiguous.

Two authoring rules, both learned from real rounds:

- **Forbid the build explicitly.** A reviewer given no constraint may spend its
  entire run compiling the workspace and return nothing.
- **Ask whether a change weakens coverage, not merely whether it is correct.**
  "Do these tests still fail if the guard is removed?" gets a mutation test.
  "Are these tests correct?" gets an opinion.

## 3. Derive identity and verdict from the run

Never from this session's judgement. An empty identity string is still non-empty
enough for the gate to accept, so check the parts, not the result:

```bash
MODEL=$(sed -n 's/^model: //p' "$REPORT.err" | head -1)
SESSION=$(sed -n 's/^session id: //p' "$REPORT.err" | head -1)
[ -n "$MODEL" ] && [ -n "$SESSION" ] \
  || { echo "cannot identify the reviewer; refusing to record"; exit 1; }
REVIEWER="Codex $MODEL / $SESSION"

VERDICT=$(grep -E '^VERDICT: (pass|fail)$' "$REPORT" | tail -1 | cut -d' ' -f2)
[ -n "$VERDICT" ] \
  || { echo "no verdict line in report; refusing to record"; exit 1; }
```

## 4. Record the receipt

```bash
rtk python3 scripts/sdlc.py review-receipt --pr "$PR" --kind codex \
  --verdict "$VERDICT" --reviewer "$REVIEWER" --body-file "$REPORT" \
  --expect-sha "$HEAD_SHA"
```

`--expect-sha` is required. It compares against the head GitHub reports at
recording time and refuses — non-zero, posting nothing — when they differ. A
refusal means the head moved during the review: the report describes a commit
that is no longer this branch's head, and the review must be run again.

**Record every round when you obtain it, not when convenient.** A `fail` is a
veto the gate honours and the next round's reviewer reads. An unrecorded verdict
is indistinguishable from a review that never happened — a later reviewer told
"round 2 passed" will correctly refuse to rely on a receipt it cannot find.

## 5. After a fix commit

Push it, then **re-capture `$HEAD_SHA` before running this skill again**:

```bash
HEAD_SHA=$(rtk proxy gh pr view "$PR" --json headRefOid -q .headRefOid)
```

Skipping that leaves the next reviewer comparing the new head against the old
SHA and stopping, which looks like a review failure and is not one — and if the
reviewer does not catch it, `--expect-sha` refuses the receipt.

GitHub's API can report the pre-push head for several seconds. Re-query until it
agrees with `git rev-parse HEAD` rather than recording against a stale read.

## Failure modes this step has actually hit

- **Reading `$REPORT` before the process exits.** It is empty until then, so a
  running review looks exactly like one that produced nothing. Wait for exit,
  then read. Misreading an in-flight report as final has produced both a false
  "the review did not run" and redundant re-runs.
- **The reviewer running the quality gate.** Section 2 forbids it. An
  unconstrained reviewer sharing the worktree can also collide with the
  implementer's own gate run over the shared cargo target directory.
- **A verdict obtained but not recorded.** See section 4.

## The optional adversarial pass

`adversarial-review` challenges design and assumptions — a different lens from
defect hunting. It is **advisory only** and never supplies `--verdict`: it has
no `VERDICT: pass|fail` contract, and its `--scope` options are
`auto|working-tree|branch`, none of which is "the pushed head SHA".

`/codex:review` and `/codex:adversarial-review` cannot be used here at all. Both
set `disable-model-invocation: true` — OpenAI's own frontmatter — making them
user-invocable only, and no automated step may depend on a human keystroke. The
underlying script has no such restriction.

Resolve the plugin by the companion file, never by a hardcoded version:

```bash
COMPANION=$(find "$HOME/.claude/plugins/cache/openai-codex/codex" \
  -maxdepth 3 -name codex-companion.mjs -type f 2>/dev/null | sort | tail -1)
if [ -n "$COMPANION" ] && [ -f "$COMPANION" ]; then
  node "$COMPANION" adversarial-review --wait --scope branch --base origin/main
else
  echo "supplementary adversarial pass: skipped, Codex plugin not found"
fi
```

Test the file, not the directory name: plugin version directories are not
guaranteed to be semver — some are git SHAs — so a version sort is unreliable,
and `find` locates the companion wherever it landed.

Use `find`, not a glob. Under zsh's default `nomatch`, an unmatched glob aborts
the whole command rather than expanding to nothing, and `2>/dev/null` on the
command does not suppress it because the shell expands before running anything.
`find` against a missing directory simply prints nothing. Both the non-empty and
the `-f` test are still needed, because an empty `COMPANION` would otherwise make
`-f ""` the thing that decides.

## What is ours and what is OpenAI's

Ours, and safe to change: the prompt, the `VERDICT: pass|fail` contract, the
stream handling, the identity derivation, `review-receipt` and `--expect-sha`.

OpenAI's, and subject to drift on upgrade: `codex exec` flags and its stderr
format — `model:` and `session id:` are scraped in section 3 and a rename breaks
identity derivation; `codex-companion.mjs` subcommands; the
`disable-model-invocation` frontmatter.

A session upgrading the plugin should re-check section 3's two `sed` patterns
and the companion's subcommand list before assuming this file still holds.
