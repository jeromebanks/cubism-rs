#!/usr/bin/env bash
# Builds the time-series web-analytics demo (`docs/TIMESERIES_ROADMAP.md`
# Milestone 12a): generates a temporal event stream, then drives
# `cubism temporal-build` + `cubism iceberg-build` (durable mode) once per
# day over a contiguous day range, publishing ONE day twice — revision 1
# from the initial event set, revision 2 after a late-arriving correction —
# so that day's window_id ends up with two published revisions.
#
# This demonstrates a revision bump driven by re-running the build over a
# fuller event set. It does NOT exercise `LatenessPolicy` or
# `CorrectionPlan` — nothing in this script or in `iceberg-build` consults
# `temporal.allowedLateness`; the "late" event is simply absent from the
# first build's input and present in the second's. See the roadmap's
# Milestone 12a entry for why that distinction is called out explicitly.
#
# The day range, the corrected day, and the generator's shape all come
# from `demo_env.sh`, shared with the other two demo scripts, so widening
# the demo is a one-line change there rather than an edit to a hardcoded
# list of dates in three places.
#
# All output (generated CSVs, the Iceberg warehouse, the sqlite catalog
# and control-store databases) is written under `.temporal_build/`, which
# is gitignored — same "generated, deliberately uncommitted" convention as
# `events.csv` for the static demo.
set -euo pipefail

cd "$(dirname "$0")"
OUT=.temporal_build
BIN="cargo run --release -q -p cubism-cli --"
SPEC=web_analytics_temporal.yaml

# START_DATE / DAYS / USERS / LATE_OFFSET / day_at() / LATE_DAY
. ./demo_env.sh

rm -rf "$OUT"
mkdir -p "$OUT/warehouse"

echo "== generating temporal event stream ($DAYS days from $START_DATE, $USERS users) =="
python3 generate_temporal_events.py \
  --output "$OUT/events_temporal.csv" \
  --initial-output "$OUT/events_temporal_initial.csv" \
  --days "$DAYS" --users "$USERS" --late-day-offset "$LATE_OFFSET" \
  --start-date "$START_DATE"

build_window() {
  local day="$1" next_day="$2" input="$3" revision="$4" run_id="$5"
  echo "== temporal-build: $day (revision $revision, input $(basename "$input")) =="
  $BIN temporal-build "$SPEC" \
    --input "$input" \
    --window-start "${day}T00:00:00Z" --window-end "${next_day}T00:00:00Z" \
    --states-output "$OUT/states_${day}_r${revision}.parquet" \
    --registry-output "$OUT/registry_${day}_r${revision}.parquet"
  echo "== iceberg-build: $day (revision $revision) =="
  $BIN iceberg-build "$SPEC" \
    --states-input "$OUT/states_${day}_r${revision}.parquet" \
    --registry-input "$OUT/registry_${day}_r${revision}.parquet" \
    --window-id "$day" --revision "$revision" --run-id "$run_id" \
    --warehouse "$OUT/warehouse" \
    --catalog-db "$OUT/catalog.sqlite" --control-db "$OUT/control.sqlite"
}

for ((i = 0; i < DAYS; i++)); do
  day=$(day_at "$i")
  next_day=$(day_at "$((i + 1))")
  if [ "$i" = "$LATE_OFFSET" ]; then
    # The late window: revision 1 from the initial (pre-correction)
    # stream, revision 2 from the full stream once the late events have
    # "arrived". Every other day has nothing to correct and is built once.
    build_window "$day" "$next_day" "$OUT/events_temporal_initial.csv" 1 "run-${day}-r1"
    build_window "$day" "$next_day" "$OUT/events_temporal.csv" 2 "run-${day}-r2"
  else
    build_window "$day" "$next_day" "$OUT/events_temporal.csv" 1 "run-${day}-r1"
  fi
done

echo
echo "== control_runs rows for window $LATE_DAY (revision bump) =="
sqlite3 -header -column "$OUT/control.sqlite" \
  "SELECT run_id, window_id, revision, status FROM control_runs WHERE window_id = '$LATE_DAY' ORDER BY revision;"

echo
echo "== control_publications current pointer for window $LATE_DAY =="
sqlite3 -header -column "$OUT/control.sqlite" \
  "SELECT cube_id, window_id, revision, run_id FROM control_publications WHERE window_id = '$LATE_DAY';"

echo
echo "built $DAYS day(s): $(day_at 0) .. $(day_at $((DAYS - 1)))  (corrected day: $LATE_DAY, revision 2)"
