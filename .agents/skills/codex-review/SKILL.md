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
| `$HEAD_SHA` | the head the review must read; section 5 owns the one capture procedure, via `codex-await-head` |
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
```

The empty-report guard moved into section 3's `codex-envelope` call, which
reads `$REPORT` anyway; there is no need to check it twice.

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
  python3 scripts/sdlc.py findings --pr $PR --kind codex

<round context: which round this is, and each previous round's commit and
verdict. Prior receipts are PR comments; tell the reviewer to read them.>

The last command prints every structured finding recorded so far for this PR,
keyed by its exact id. When a finding it lists is still present in the code,
reuse that id verbatim — a renumbered id is not recognized as the same
finding and leaves the old one open forever. Mint a new id only for a finding
that is genuinely new. Close a prior id explicitly (never by omitting it) by
reporting it again with "disposition": "fixed" or "resolved" and non-empty
"evidence". You do not need to repeat an id whose status is unchanged.

Check at least:
<numbered, specific to the slice's acceptance criteria>

Report numbered findings. Each must cite file:line and state concretely what
goes wrong. If you find nothing material, say so explicitly.

Optionally, before the final line, report every finding this round is
opening, closing, or otherwise touching as one compact-JSON line:

FINDINGS: [{"id":"$N-1","blocking":true,"disposition":"open"}]

Each entry is `{"id": str, "blocking": bool, "disposition":
"open"|"fixed"|"resolved", "evidence": str}` (`evidence` required once
`disposition` is not `"open"`). This line, when present, must be the last
line before the final VERDICT line below — nothing after it but that line.
Write it exactly as shown, at the very start of the line: no leading
whitespace, no `-`/`*` list marker, no blockquote `>`, and not wrapped in
backticks or a code fence. A malformed or misplaced attempt is rejected
outright and forces this round to record as a `fail` with no findings, which
loses whatever it was trying to report. Omit it entirely when there is
nothing to report; an explicit `FINDINGS: []` behaves identically to
omitting it, and an ordinary prose sentence that happens to start with the
word "findings" (for example, restating in prose that you found nothing
material) is not mistaken for this line *unless it also continues with a
literal `[`* — so do not write a sentence of that shape either.

Refer to a prior round's finding by its id only. If you need to discuss what
an earlier round reported, describe it in prose (mentioning it mid-sentence
is fine); never reproduce a prior round's `FINDINGS:` line as its own line —
in a blockquote, a code block, or otherwise starting a line by itself — even
as a quote. A round that reports its own well-formed `FINDINGS:` line right
before `VERDICT:` is unaffected by such a quote appearing earlier — the
correctly placed line is recognized first. But a round with nothing new of
its own to report, that reproduces an old `FINDINGS:` line as a standalone
line anywhere in its report, has no correctly placed line of its own to be
recognized first: the quoted one is then the only line that looks like an
attempt, and it forces this round to a `fail` for no reason.

Never answer `VERDICT: pass` while any finding marked `blocking` — this
round's or a prior round's still-open one — remains `open`. Close it in this
same FINDINGS line first, or answer `fail`.

Then end your response with exactly one final line, and nothing after it:
VERDICT: pass
or
VERDICT: fail
```

Findings come first; the verdict is the last line. `review-receipt` does not
parse either — section 3 does, which is why both contracts have to be
unambiguous.

Two authoring rules, both learned from real rounds:

- **Forbid the build explicitly.** A reviewer given no constraint may spend its
  entire run compiling the workspace and return nothing.
- **Ask whether a change weakens coverage, not merely whether it is correct.**
  "Do these tests still fail if the guard is removed?" gets a mutation test.
  "Are these tests correct?" gets an opinion.

## 3. Derive identity, verdict, and findings from the run

Never from this session's judgement. An empty identity string is still non-empty
enough for the gate to accept, so check the parts, not the result. `sdlc.py`'s
`codex-envelope` command owns this parsing — see "What is ours and what is
OpenAI's" below for why it, not this skill file, is what a fresh session edits
after a `codex exec` upgrade:

```bash
REVIEWER=$(rtk proxy python3 scripts/sdlc.py codex-envelope --field reviewer \
  --report "$REPORT" --err "$REPORT.err") || exit 1
[ -n "$REVIEWER" ] || { echo "cannot identify the reviewer; refusing to record"; exit 1; }

VERDICT=$(rtk proxy python3 scripts/sdlc.py codex-envelope --field verdict \
  --report "$REPORT" --err "$REPORT.err") || exit 1
[ -n "$VERDICT" ] || { echo "report does not end with a verdict line; refusing to record"; exit 1; }

if ! FINDINGS_JSON=$(rtk proxy python3 scripts/sdlc.py codex-envelope --field findings \
  --report "$REPORT" --err "$REPORT.err"); then
  echo "report's FINDINGS line is malformed; recording this round as a forced fail with no findings" >&2
  VERDICT=fail
  FINDINGS_JSON='[]'
fi
```

`rtk proxy` matters here exactly as it does in section 5: the value captured
is read back verbatim by later steps, not summarized for a human to read.
`$(...)` only ever captures stdout, so `codex-envelope`'s own diagnostic —
printed to stderr by `sdlc.py`'s top-level error handler, exactly like every
other command in this CLI — still reaches the operator directly instead of
being folded into `$REVIEWER`/`$VERDICT`/`$FINDINGS_JSON` (section 1's "the
two streams go to separate files" rule applies here too: merging them would
let a stray stderr line, not just an error, end up recorded in the receipt on
an otherwise successful run). The trailing `[ -n ... ]` guard on the first two
is only a last-resort net against an empty success that should be
structurally impossible; `$FINDINGS_JSON` has no equivalent guard because
`codex-envelope` never prints an empty string for this field — a report with
nothing structured prints the literal `[]`, and any actual failure exits
non-zero, already caught by `|| exit 1`.

`codex-envelope --field verdict` matches the **last non-empty line**, not any
line that looks like a verdict. Searching the whole report accepts one that
says `VERDICT: pass` and then keeps talking, which is a malformed report whose
real conclusion is unknown. The contract in section 2 says the verdict is the
final line; this is where that is enforced, so a reviewer that ignores the
contract fails closed instead of having a verdict guessed for it.

`--field findings` reads the optional `FINDINGS: <json>` line the same way:
it must be written *exactly* as `FINDINGS: <json>` — no leading whitespace,
list marker, or code formatting — and be the last non-blank line *before*
that trailing verdict line, or there is no structured findings line at all
(`FINDINGS_JSON` becomes `[]`). Anything that even loosely resembles an
attempt at one and gets the contract wrong — indented, in backticks, as a
list item, missing the space, after `VERDICT:`, not immediately preceding
it, malformed JSON, or a payload `parse_findings` itself rejects — fails
closed rather than being silently read as "none," because a markdown-writing
model is likely to produce exactly these near-misses, and any of them could
otherwise drop a blocking finding invisibly. A reviewer that renumbers a
still-open id instead of reusing it is not caught here: the new id and the
old one are indistinguishable strings to this parser. Section 2's
instructions are the only defense against that today; the `findings` read
command exists so a reviewer has no reason to guess.

A malformed `FINDINGS:` line is not swallowed once `$VERDICT` has already
been derived successfully: the block above forces `VERDICT=fail` and
`FINDINGS_JSON='[]'` rather than exiting, because the alternative — aborting
after the verdict is already known — would either lose a real verdict (a
`fail` the gate needs to see) or tempt a re-run that discards it. This does
not come free: whatever structured findings that malformed line was trying
to report are discarded along with it, not recovered some other way — they
survive only as free-form prose in the recorded report body, exactly like
before this mechanism existed. The forced `fail` blocks only the *current*
head; nothing stops a later fix commit from passing cleanly without those
ids ever having been structurally recorded. It is still the right call,
because the round is recorded and the head is blocked rather than merging on
an unverifiable report — a report whose own findings contract is broken is
not one whose `pass` should be trusted either, since both come from the same
untrusted text — but it is a real degradation to pre-#116 behavior for that
one round, not a lossless recovery.

Both the identity/verdict parsers and the findings parser are byte-literal
matches for the `sed`/`awk` they replace, or (for findings) mirror that same
discipline for a line that never had a bash equivalent — splitting on `\n`
alone and treating only a run of spaces/tabs as blank, never
`str.splitlines()`/`str.strip()`'s wider notion of whitespace — so a CRLF
report or a stray form-feed line fails exactly as it always has.
`scripts/tests/test_sdlc.py` pins the fixture table this was verified against,
including an always-on test that runs this very code block (with a
`rtk`-shimmed `PATH`) against those fixtures, so a future edit to this section
that quietly stops calling `codex-envelope` is itself a caught regression, not
just a hoped-for convention.

## 4. Record the receipt

```bash
rtk python3 scripts/sdlc.py review-receipt --pr "$PR" --kind codex \
  --verdict "$VERDICT" --reviewer "$REVIEWER" --body-file "$REPORT" \
  --expect-sha "$HEAD_SHA" --findings "$FINDINGS_JSON"
```

`--expect-sha` is required. It compares against the head GitHub reports at
recording time and refuses — non-zero, posting nothing — when they differ. A
refusal means the head moved during the review: the report describes a commit
that is no longer this branch's head, and the review must be run again.

`--findings "$FINDINGS_JSON"` is always passed, even when section 3 derived
`[]` — `review-receipt` already treats an empty list identically to the flag
being omitted (`review_marker` writes no `findings` key either way), so this
is never a behavior change for a round with nothing structured to report.

**Record every round when you obtain it, not when convenient.** A `fail` is a
veto the gate honours and the next round's reviewer reads. An unrecorded verdict
is indistinguishable from a review that never happened — a later reviewer told
"round 2 passed" will correctly refuse to rely on a receipt it cannot find.

## 5. After the head changes

This section owns **every** way the head moves -- a fix commit, a rebase onto
the default branch, an amend, anything. `work-slice` defers here rather than
repeating it, because two copies drifted once already.

Push, then **re-capture `$HEAD_SHA` before running this skill again**. GitHub's
API can report the pre-push head for several seconds, so a single read can
return the SHA you just replaced. `sdlc.py`'s `codex-await-head` retries a
fresh read against local `HEAD` until they agree, up to five attempts with a
delay between, and fails closed if they never do:

```bash
HEAD_SHA=$(rtk proxy python3 scripts/sdlc.py codex-await-head --pr "$PR" \
  --local-head "$(rtk proxy git rev-parse HEAD)") || exit 1
```

Skipping the retry — capturing once, immediately after the push — is the
specific way this goes wrong: the reviewer is handed a stale SHA, refuses to
review a head that does not match, and the result looks like a review failure
rather than a race. And skipping the re-capture entirely leaves the next
reviewer comparing the new head against the old SHA and stopping, which looks
like a review failure and is not one — and if the reviewer does not catch it,
`--expect-sha` refuses the receipt.

This is also how `$HEAD_SHA` is produced in the first place. `work-slice` step 7
creates the pull request and then calls this section for the capture rather than
reading `headRefOid` once, because the same race applies to a freshly created
PR. There is one capture procedure, here, used every time the value is needed.

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
COMPANION=$(python3 - <<'PY' 2>/dev/null
import json, pathlib
cfg = pathlib.Path.home() / ".claude/plugins/installed_plugins.json"
try:
    plugins = json.loads(cfg.read_text()).get("plugins") or {}
except (OSError, ValueError):
    raise SystemExit
for name, entries in plugins.items():
    if name.split("@", 1)[0] != "codex":
        continue
    for entry in entries:
        path = pathlib.Path(entry.get("installPath", "")) / "scripts/codex-companion.mjs"
        if path.is_file():
            print(path)
            raise SystemExit
PY
)
if [ -n "$COMPANION" ]; then
  node "$COMPANION" adversarial-review --wait --scope branch --base origin/main
else
  echo "supplementary adversarial pass: skipped, Codex plugin not found"
fi
```

**Resolve from `installed_plugins.json`, never from the filesystem.** That file's
`installPath` is what the harness actually loaded; everything else is a guess.
Two earlier attempts here were both wrong, and the second looked right:

- A version sort over the cache directories. Plugin directories are not
  guaranteed semver -- several installed here are git SHAs -- and a lexical sort
  is wrong even for semver, ordering `1.0.10` before `1.0.9`.
- Newest file by `ls -t`. That is modification time, not installation time. A
  cache that preserves timestamps, or any later touch, selects a stale version
  whose subcommands may differ from the one in use.

The non-empty test still matters, but existence only proves a file is there, not
that it is the active plugin. Reading `installPath` proves both.

The block degrades to one printed line when the config is absent, unparseable,
or lists no `codex` plugin, and `python3` is already required by this
repository's tooling.

## What is ours and what is OpenAI's

Ours, and safe to change: the prompt, the `VERDICT: pass|fail` contract, the
`FINDINGS: <json>` contract, the stream handling, `scripts/sdlc.py`'s
`codex-envelope`/`codex-await-head`/`findings` commands and the functions
behind them (`parse_codex_identity`, `parse_codex_verdict`,
`parse_codex_findings`, `require_nonempty_report`, `await_matching_head`),
`review-receipt`, `--expect-sha`, and `--findings`.

OpenAI's, and subject to drift on upgrade: `codex exec` flags and its stderr
format — `model:` and `session id:` are scraped by `parse_codex_identity` and a
rename breaks identity derivation; `codex-companion.mjs` subcommands; the
`disable-model-invocation` frontmatter.

These are **two independently versioned things, and they break different
steps**:

- Upgrading the **Codex CLI** (`codex exec`) can change the stderr labels that
  `parse_codex_identity` scrapes. That breaks identity derivation and makes
  receipts unrecordable -- the required path. A plugin upgrade cannot validate
  this, and a CLI upgrade happens without touching the plugin at all.
- Upgrading the **Claude Code plugin** can change `codex-companion.mjs`
  subcommands or the `disable-model-invocation` frontmatter. That affects only
  the optional pass in the previous section.

Two test layers guard identity/verdict/findings derivation, and they catch
different things. `python3 scripts/tests/test_sdlc.py` runs a fixture table
(CRLF, a stray form-feed line, an empty-then-non-empty `model:` match, a
misplaced or malformed `FINDINGS:` line, and more) against
`parse_codex_identity`/`parse_codex_verdict`/`parse_codex_findings` directly,
and a structural companion test that executes sections 3 and 4's actual
fenced code blocks (with `rtk` shimmed on `PATH`) against those same
fixtures — so a future edit that quietly stops calling `codex-envelope` or
drops `--findings`, not just a changed label, fails a test every run, with no
`codex` binary required. `python3 -m unittest
scripts.tests.test_sdlc.CodexReviewEnvelopeTests -v` additionally exercises
`parse_codex_identity` against a real `codex exec` run's stderr, when `codex`
is on `PATH` outside a Codex sandbox — a CLI upgrade that renames either label
fails that command with the same "cannot identify the reviewer" error this
section raises, instead of surfacing it mid-slice as an apparent Codex
malfunction.

So: after a CLI upgrade, run that command (outside a Codex sandbox); it is the
only layer that needs a real `codex` binary. After a plugin upgrade, re-check
the companion's subcommand list. Doing only the second is the easy mistake,
because the plugin is the thing that looks like a dependency.
