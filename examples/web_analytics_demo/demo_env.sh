# Shared shape of the time-series demo, sourced by build_temporal_demo.sh,
# query_temporal_demo.sh and run_temporal_demo.sh.
#
# These four numbers define the generated dataset. They live here rather
# than in each script because the scripts must agree: they share the
# `.temporal_build/` output directory, and `query_temporal_demo.sh`'s whole
# point is querying *the corrected window* before and after its correction
# — if it computed a different corrected day than the generator held events
# back from, its before/after pair would be two identical responses and the
# script would silently prove nothing.
#
# Override any of them from the environment to reshape the demo, e.g.
#   DAYS=30 USERS=9000 ./build_temporal_demo.sh
# Keep `generate_temporal_events.py`'s own argparse defaults in step with
# these; the scripts pass them explicitly rather than relying on that.

START_DATE=${START_DATE:-2026-04-06}
DAYS=${DAYS:-14}
USERS=${USERS:-4200}
# 0-indexed day within the range that receives the late-arriving
# correction, i.e. the one window published twice (revision 1 then 2).
LATE_OFFSET=${LATE_OFFSET:-7}

# `date` differs between BSD (macOS) and GNU; probe rather than assume.
day_at() {
  local offset="$1"
  if date -v +1d +%Y-%m-%d >/dev/null 2>&1; then
    date -j -v "+${offset}d" -f %Y-%m-%d "$START_DATE" +%Y-%m-%d
  else
    date -u -d "$START_DATE + ${offset} day" +%Y-%m-%d
  fi
}

# The window every script means when it says "the corrected window".
LATE_DAY=$(day_at "$LATE_OFFSET")
LATE_NEXT_DAY=$(day_at "$((LATE_OFFSET + 1))")
