#!/usr/bin/env bash
# Idempotent installer + verifier for Apache Spark on macOS, scoped to what
# cubism's Phase 0B benchmark needs: a working `spark-submit` on PATH, its
# required JVM, sbt for building the Scala KMV adapter (the adapter itself
# is implemented in Scala, not PySpark -- see docs/TIMESERIES_PHASE_0B_
# HARNESS.md remaining-work item 1 and ~/.claude/plans/flickering-popping-
# harp.md for why), and a matched pyspark venv kept around for ad hoc
# prototyping/data exploration only. Does NOT implement the adapter itself.
#
# Safe to re-run: every step checks current state before changing anything.
set -euo pipefail

MARKER_BEGIN="# >>> cubism spark-setup >>>"
MARKER_END="# <<< cubism spark-setup <<<"
PYSPARK_VENV="${PYSPARK_VENV:-$HOME/.cubism-spark-venv}"
CARGO_TARGET_DIR_DEFAULT="/tmp/cubism-target-phase0b"
SBT_SCRATCH_DIR="${SBT_SCRATCH_DIR:-$(mktemp -d /tmp/cubism-sbt-verify.XXXXXX)}"

log()  { printf '\n== %s ==\n' "$1"; }
ok()   { printf 'OK: %s\n' "$1"; }
warn() { printf 'WARN: %s\n' "$1"; }
fail() { printf 'FAIL: %s\n' "$1"; exit 1; }

# --- 1. platform ------------------------------------------------------------
log "Platform check"
[[ "$(uname -s)" == "Darwin" ]] || fail "this script is macOS-only (Homebrew formula names, keg-only paths)"
ok "macOS ($(uname -m))"

# --- 2. Homebrew present -----------------------------------------------------
log "Homebrew check"
if ! command -v brew >/dev/null 2>&1; then
  fail "Homebrew not found. Install it yourself first (https://brew.sh) -- this script won't do a system-wide curl|bash install on your behalf."
fi
ok "brew at $(command -v brew)"

# --- 3. resolve the JDK apache-spark actually depends on ---------------------
log "Resolving required JDK"
JDK_FORMULA="$(brew info --json=v2 apache-spark 2>/dev/null \
  | python3 -c "import json,sys; d=json.load(sys.stdin); deps=d['formulae'][0]['dependencies']; print(next((x for x in deps if x.startswith('openjdk')), ''))" \
  2>/dev/null || true)"
if [[ -z "$JDK_FORMULA" ]]; then
  JDK_FORMULA="openjdk@21"
  warn "could not read apache-spark's JDK dependency from brew metadata; falling back to $JDK_FORMULA (correct as of this writing, but re-check if Spark's formula has since bumped it)"
else
  ok "apache-spark currently depends on $JDK_FORMULA"
fi

# --- 4. install JDK + Spark (brew install is a no-op if already current) ----
log "Installing $JDK_FORMULA"
if brew list --formula "$JDK_FORMULA" >/dev/null 2>&1; then
  ok "$JDK_FORMULA already installed ($(brew list --formula --versions "$JDK_FORMULA"))"
else
  brew install "$JDK_FORMULA"
fi

log "Installing apache-spark"
if brew list --formula apache-spark >/dev/null 2>&1; then
  ok "apache-spark already installed ($(brew list --formula --versions apache-spark))"
else
  brew install apache-spark
fi

# --- 5. resolve JAVA_HOME (JDK formula is keg-only, so not on PATH) ---------
log "Resolving JAVA_HOME"
JDK_PREFIX="$(brew --prefix "$JDK_FORMULA")"
JAVA_HOME_RESOLVED="$JDK_PREFIX/libexec/openjdk.jdk/Contents/Home"
[[ -x "$JAVA_HOME_RESOLVED/bin/java" ]] || fail "expected java at $JAVA_HOME_RESOLVED/bin/java, not found -- brew layout may have changed"
export JAVA_HOME="$JAVA_HOME_RESOLVED"
export PATH="$JAVA_HOME/bin:$PATH"
ok "JAVA_HOME=$JAVA_HOME"
"$JAVA_HOME/bin/java" -version

# --- 6. verify spark-submit is on PATH (brew symlinks it; not keg-only) -----
log "Verifying spark-submit"
if ! command -v spark-submit >/dev/null 2>&1; then
  # brew's own bin dir may not be on this non-interactive shell's PATH yet
  export PATH="$(brew --prefix)/bin:$PATH"
fi
command -v spark-submit >/dev/null 2>&1 || fail "spark-submit still not on PATH after brew install apache-spark; check 'brew --prefix apache-spark'"
SPARK_VERSION_LINE="$(spark-submit --version 2>&1 | grep -m1 -E 'version [0-9]' || true)"
SPARK_VERSION="$(printf '%s' "$SPARK_VERSION_LINE" | grep -oE '[0-9]+\.[0-9]+\.[0-9]+' | head -1)"
[[ -n "$SPARK_VERSION" ]] || fail "spark-submit --version did not report a parseable version"
ok "spark-submit $SPARK_VERSION at $(command -v spark-submit)"

# --- 7. real job smoke test, not just a version banner ----------------------
log "Running SparkPi smoke test (confirms the JVM actually executes a job)"
SPARK_PREFIX="$(brew --prefix apache-spark)"
RUN_EXAMPLE="$SPARK_PREFIX/bin/run-example"
[[ -x "$RUN_EXAMPLE" ]] || fail "run-example not found at $RUN_EXAMPLE"
SPARK_PI_OUT="$("$RUN_EXAMPLE" SparkPi 10 2>&1)" || { printf '%s\n' "$SPARK_PI_OUT"; fail "SparkPi job failed -- see output above (common cause: JAVA_HOME mismatch)"; }
printf '%s\n' "$SPARK_PI_OUT" | grep -m1 "Pi is roughly" || { printf '%s\n' "$SPARK_PI_OUT"; fail "SparkPi ran but produced no result line"; }
ok "local Spark job executed successfully"

# --- 8. sbt + a real build (the actual KMV adapter is Scala, not PySpark) ---
log "Installing sbt"
if brew list --formula sbt >/dev/null 2>&1; then
  ok "sbt already installed ($(brew list --formula --versions sbt))"
else
  brew install sbt
fi
command -v sbt >/dev/null 2>&1 || export PATH="$(brew --prefix)/bin:$PATH"
command -v sbt >/dev/null 2>&1 || fail "sbt still not on PATH after brew install sbt"

log "Verifying sbt with a real build (not just --version)"
# Matches Spark 4.2.0's bundled Scala (scala-library-2.13.18.jar in Spark's
# jars dir) -- the actual spark-adapter/ project must pin the same version.
SCALA_VERSION_TARGET="2.13.18"
mkdir -p "$SBT_SCRATCH_DIR/project" "$SBT_SCRATCH_DIR/src/main/scala"
cat > "$SBT_SCRATCH_DIR/build.sbt" <<EOF
scalaVersion := "$SCALA_VERSION_TARGET"
name := "cubism-sbt-verify"
EOF
printf 'sbt.version=1.10.7\n' > "$SBT_SCRATCH_DIR/project/build.properties"
cat > "$SBT_SCRATCH_DIR/src/main/scala/Hello.scala" <<'EOF'
object Hello { def main(args: Array[String]): Unit = println("cubism-sbt-verify OK") }
EOF
if ! ( cd "$SBT_SCRATCH_DIR" && sbt -batch package > "$SBT_SCRATCH_DIR/sbt.log" 2>&1 ); then
  tail -n 40 "$SBT_SCRATCH_DIR/sbt.log"
  fail "sbt package failed in scratch project (log above, full log at $SBT_SCRATCH_DIR/sbt.log) -- global sbt $(sbt --version 2>/dev/null | tail -1 || true) may not resolve Scala $SCALA_VERSION_TARGET cleanly; spark-adapter/'s own project/build.properties can pin a different sbt launcher version if so"
fi
JAR_COUNT="$(find "$SBT_SCRATCH_DIR/target" -name '*.jar' 2>/dev/null | wc -l | tr -d ' ')"
[[ "$JAR_COUNT" -gt 0 ]] || fail "sbt package reported success but produced no jar under $SBT_SCRATCH_DIR/target"
ok "sbt package produced a real jar for Scala $SCALA_VERSION_TARGET ($JAR_COUNT jar(s))"
rm -rf "$SBT_SCRATCH_DIR"

# --- 9. matched pyspark in an isolated venv (via uv) -------------------------
log "Setting up matched pyspark ($SPARK_VERSION) via uv -- prototyping only, not the adapter's language"
if ! command -v uv >/dev/null 2>&1; then
  warn "uv not found on PATH -- skipping pyspark venv setup. Install uv (https://docs.astral.sh/uv/) and rerun, or 'pip install pyspark==$SPARK_VERSION' manually."
else
  if [[ ! -x "$PYSPARK_VENV/bin/python" ]]; then
    uv venv "$PYSPARK_VENV"
  else
    ok "venv already exists at $PYSPARK_VENV"
  fi
  uv pip install --python "$PYSPARK_VENV/bin/python" "pyspark==$SPARK_VERSION"
  "$PYSPARK_VENV/bin/python" - <<PYEOF
import os
os.environ.setdefault("JAVA_HOME", "$JAVA_HOME")
from pyspark.sql import SparkSession
spark = SparkSession.builder.master("local[2]").appName("cubism-spark-setup-check").getOrCreate()
n = spark.range(5).count()
assert n == 5, f"expected 5, got {n}"
spark.stop()
print("pyspark local session OK, count()=", n)
PYEOF
  ok "pyspark $SPARK_VERSION verified in $PYSPARK_VENV"
fi

# --- 10. persist JAVA_HOME / PATH for future shells --------------------------
log "Shell profile"
CURRENT_SHELL="$(basename "${SHELL:-zsh}")"
case "$CURRENT_SHELL" in
  zsh)  RC_FILE="$HOME/.zshrc" ;;
  bash) RC_FILE="$HOME/.bash_profile" ;;
  *)    RC_FILE="" ;;
esac

if [[ -n "$RC_FILE" ]]; then
  if [[ -f "$RC_FILE" ]] && grep -qF "$MARKER_BEGIN" "$RC_FILE"; then
    ok "$RC_FILE already has a cubism spark-setup block (not duplicating)"
  else
    {
      printf '\n%s\n' "$MARKER_BEGIN"
      printf 'export JAVA_HOME="%s"\n' "$JAVA_HOME"
      printf 'export PATH="%s/bin:%s/bin:$PATH"\n' "$JAVA_HOME" "$SPARK_PREFIX"
      printf '%s\n' "$MARKER_END"
    } >> "$RC_FILE"
    ok "appended JAVA_HOME/PATH block to $RC_FILE (marked, remove between the $MARKER_BEGIN / $MARKER_END lines to undo)"
  fi
else
  warn "unrecognized \$SHELL ($CURRENT_SHELL) -- add these manually to your shell profile:"
  printf '  export JAVA_HOME="%s"\n' "$JAVA_HOME"
  printf '  export PATH="%s/bin:%s/bin:$PATH"\n' "$JAVA_HOME" "$SPARK_PREFIX"
fi

# --- 11. re-run the project's own preflight check -----------------------------
log "cubism-timeseries-bench preflight"
REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../../.." && pwd)"
if [[ -f "$REPO_ROOT/Cargo.toml" ]]; then
  ( cd "$REPO_ROOT" && CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$CARGO_TARGET_DIR_DEFAULT}" \
      cargo run -p cubism-timeseries-bench -- preflight )
  ok "harness preflight now passes"
else
  warn "could not find the cubism Cargo workspace at $REPO_ROOT -- run 'cargo run -p cubism-timeseries-bench -- preflight' manually to confirm"
fi

log "Done"
cat <<SUMMARY
JAVA_HOME:    $JAVA_HOME
spark-submit: $(command -v spark-submit) ($SPARK_VERSION)
sbt:          $(command -v sbt) (verified against Scala $SCALA_VERSION_TARGET, matching Spark's bundled Scala)
pyspark venv: $PYSPARK_VENV (prototyping only -- the KMV adapter itself is Scala, see spark-adapter/)
Open a new shell (or 'source $RC_FILE') to pick up JAVA_HOME/PATH in interactive sessions.
SUMMARY
