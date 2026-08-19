#!/usr/bin/env bash
# Queries the time-series web-analytics demo over real HTTP
# (`docs/TIMESERIES_ROADMAP.md` Milestone 12b): captures a `/api/series`
# request/response pair against window 2026-04-07 before and after its
# revision-2 correction is published, proving Milestone 12a's revision
# bump (visible so far only as `control_runs`/`control_publications`
# rows) is also visible through the query path a real client would use.
#
# Builds only what this query needs: window 2026-04-07, both revisions --
# unlike `build_temporal_demo.sh` (Milestone 12a), which also builds the
# two ordinary days for its own demonstration. This script does not
# require that one to have run first. Both scripts share the same
# `.temporal_build/` output directory; each does its own `rm -rf` at the
# top (same ephemeral-output convention 12a established), so the last one
# run "wins" as the current state of that directory.
#
# `cubism serve` takes a required positional `cube_path` argument even
# when only `--spec`/`--warehouse` (the /api/series routes) are wanted --
# `CubeStore::from_path` still requires a real parquet with an 'xunit'
# column, though nothing in `/api/series`'s own path
# (`crates/cubism-serve/src/series.rs`) ever reads it. Rather than depend
# on the *static* demo's `web_analytics_cube.parquet` (untracked, only
# present if that demo has been built locally -- would silently fail from
# a clean checkout), this script writes its own throwaway one-row
# placeholder parquet into `.temporal_build/`.
#
# One server process serves both captures -- confirmed empirically before
# writing this script (not assumed): a `cubism serve` process holds its
# catalog/publication-store handles open and re-resolves `current` per
# request, so a revision published by a second, independent CLI
# invocation while the server is already running is visible on the very
# next request, no restart needed.
set -euo pipefail

cd "$(dirname "$0")"
OUT=.temporal_build
BIN="cargo run --release -q -p cubism-cli --"
SPEC=web_analytics_temporal.yaml
PORT=8095
WINDOW=2026-04-07
NEXT_DAY=2026-04-08

rm -rf "$OUT"
mkdir -p "$OUT/warehouse"

echo "== generating temporal event stream =="
python3 generate_temporal_events.py \
  --output "$OUT/events_temporal.csv" \
  --initial-output "$OUT/events_temporal_initial.csv"

echo "== writing placeholder cube_path parquet (required by \`serve\`'s CLI, unused by /api/series) =="
python3 - "$OUT/placeholder_cube.parquet" <<'PY'
import sys
import pyarrow as pa
import pyarrow.parquet as pq

pq.write_table(pa.table({"xunit": ["/G"]}), sys.argv[1])
PY

build_window() {
  local input="$1" revision="$2" run_id="$3"
  echo "== temporal-build: $WINDOW (revision $revision, input $(basename "$input")) =="
  $BIN temporal-build "$SPEC" \
    --input "$input" \
    --window-start "${WINDOW}T00:00:00Z" --window-end "${NEXT_DAY}T00:00:00Z" \
    --states-output "$OUT/states_${WINDOW}_r${revision}.parquet" \
    --registry-output "$OUT/registry_${WINDOW}_r${revision}.parquet"
  echo "== iceberg-build: $WINDOW (revision $revision) =="
  $BIN iceberg-build "$SPEC" \
    --states-input "$OUT/states_${WINDOW}_r${revision}.parquet" \
    --registry-input "$OUT/registry_${WINDOW}_r${revision}.parquet" \
    --window-id "$WINDOW" --revision "$revision" --run-id "$run_id" \
    --warehouse "$OUT/warehouse" \
    --catalog-db "$OUT/catalog.sqlite" --control-db "$OUT/control.sqlite"
}

# Revision 1: the "initial" stream, with the late-arriving events held back.
build_window "$OUT/events_temporal_initial.csv" 1 "run-${WINDOW}-r1"

echo "== starting cubism serve on :$PORT =="
$BIN serve "$OUT/placeholder_cube.parquet" --port "$PORT" \
  --spec "$SPEC" --warehouse "$OUT/warehouse" \
  --catalog-db "$OUT/catalog.sqlite" --control-db "$OUT/control.sqlite" \
  >"$OUT/serve.log" 2>&1 &
SERVE_PID=$!
trap 'kill "$SERVE_PID" 2>/dev/null; wait "$SERVE_PID" 2>/dev/null || true' EXIT

echo "== waiting for the server to accept connections =="
for _ in $(seq 1 50); do
  if ! kill -0 "$SERVE_PID" 2>/dev/null; then
    echo "server process exited before becoming ready -- see $OUT/serve.log" >&2
    cat "$OUT/serve.log" >&2
    exit 1
  fi
  if curl -s -o /dev/null "http://127.0.0.1:$PORT/"; then
    break
  fi
  sleep 0.2
done

REQUEST=$(cat <<JSON
{"selector":"/G","measure":"avg_revenue","start":1775520000000000,"end":1775606400000000,"resolution":"1d","exact":false,"gap_policy":"missing","windows":[{"window_id":"$WINDOW","bucket_start":1775520000000000}]}
JSON
)

echo
echo "== /api/series BEFORE the correction (window $WINDOW at revision 1) =="
curl -s -X POST "http://127.0.0.1:$PORT/api/series" -H "Content-Type: application/json" -d "$REQUEST" \
  | tee "$OUT/response_before.json"
echo

# Revision 2: the full stream, late events included -- the correction.
# Same running server; no restart between this and the query above.
build_window "$OUT/events_temporal.csv" 2 "run-${WINDOW}-r2"

echo
echo "== /api/series AFTER the correction (window $WINDOW at revision 2, same server, no restart) =="
curl -s -X POST "http://127.0.0.1:$PORT/api/series" -H "Content-Type: application/json" -d "$REQUEST" \
  | tee "$OUT/response_after.json"
echo

kill "$SERVE_PID" 2>/dev/null; wait "$SERVE_PID" 2>/dev/null || true
trap - EXIT

echo
python3 - "$OUT/response_before.json" "$OUT/response_after.json" <<'PY'
import json
import sys

before = json.load(open(sys.argv[1]))["points"][0]
after = json.load(open(sys.argv[2]))["points"][0]
print("== summary ==")
print(f"revision 1: value={before['value']} is_exact={before['is_exact']} published={before['published']}")
print(f"revision 2: value={after['value']} is_exact={after['is_exact']} published={after['published']}")
PY
