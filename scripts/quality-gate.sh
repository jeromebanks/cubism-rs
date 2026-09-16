#!/usr/bin/env bash
# Reproduce the required CI jobs with terse output and retained failure logs.
set -uo pipefail

ROOT="$(rtk git rev-parse --show-toplevel)"
WORKTREE="${1:-$ROOT}"
LOG_DIR="$(rtk mktemp -d)"
FAILED=0

# Reuse one cache across issue worktrees; this workspace is too large to rebuild
# into a fresh target directory for every slice. Cargo's own lock safely
# serializes concurrent writers. Restricted runners can set SDLC_CARGO_TARGET_DIR
# to an isolated writable location without changing the workflow.
COMMON_GIT="$(rtk git rev-parse --path-format=absolute --git-common-dir)"
REPO_CACHE_ROOT="$(rtk proxy dirname "$COMMON_GIT")"
export CARGO_TARGET_DIR="${SDLC_CARGO_TARGET_DIR:-$REPO_CACHE_ROOT/target}"

run_check() {
  local name="$1"
  shift
  local log="$LOG_DIR/${name//[^a-zA-Z0-9]/_}.log"
  if "$@" >"$log" 2>&1; then
    echo "PASS: $name"
  else
    echo "FAIL: $name (log: $log)"
    rtk tail -n 50 "$log"
    FAILED=1
  fi
}

cd "$WORKTREE" || exit 2
run_check "rustfmt" rtk cargo fmt --all --check
run_check "clippy" rtk cargo clippy --workspace --all-targets -- -D warnings
run_check "workspace tests" rtk cargo test --workspace --all-targets
run_check "doctests" rtk cargo test --workspace --doc
run_check "rustdoc" rtk proxy env RUSTDOCFLAGS=-Dwarnings rtk cargo doc --no-deps --workspace
run_check "CLI build" rtk cargo build -p cubism-cli

if [ "$FAILED" -eq 0 ]; then
  CLI_BIN="$CARGO_TARGET_DIR/debug/cubism"
  for spec in examples/*.yaml examples/web_analytics_demo/*.yaml; do
    [ -e "$spec" ] || continue
    run_check "validate $spec" rtk proxy "$CLI_BIN" validate "$spec"
  done
fi

if command -v lychee >/dev/null 2>&1; then
  run_check "internal links" rtk lychee --offline --include-fragments README.md 'docs/**/*.md'
else
  echo "SKIP: internal links (lychee unavailable; CI remains authoritative)"
fi

if command -v maturin >/dev/null 2>&1; then
  run_check "Python binding build" rtk maturin build --manifest-path bindings/cubism-py/Cargo.toml --out "$LOG_DIR/python-dist"
else
  echo "SKIP: Python binding build (maturin unavailable; CI remains authoritative)"
fi

if [ "$FAILED" -eq 0 ]; then
  rtk proxy rm -rf "$LOG_DIR"
  echo "QUALITY_GATE=pass"
else
  echo "QUALITY_GATE=fail logs=$LOG_DIR"
fi
exit "$FAILED"
