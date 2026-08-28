"""Generate a small deterministic event stream for the time-series demo
(`docs/TIMESERIES_ROADMAP.md` Milestone 12a), with a late-arriving
correction for one window.

Separate from `generate_events.py` (the original static-cube demo's
generator, which stays untouched — its `events.csv` output and the
`web_analytics.yaml` v1 spec keep working exactly as documented in
`README.md`). This script's dataset only exists to drive the durable
`temporal-build` / `iceberg-build` CLI path against a new `cubism/v2alpha1`
spec (`web_analytics_temporal.yaml`), which `apiVersion: v1` cannot parse.

Sized so the dashboard has an actual *series* to draw: two weeks of days
at ~300 users/day, which keeps the per-day `temporal-build` +
`iceberg-build` pair a seconds-long transcript while giving every charted
XUnit enough daily volume to move. Each user is assigned to exactly one
day, so `--users` divided by `--days` is the daily cohort size.

Writes two CSVs from the same generated event set:
  - `--output`: the full stream, including the late-arriving events.
  - `--initial-output`: the same stream with the late-arriving events
    removed — this is what the "first" build (revision 1) sees. Rebuilding
    the late window from `--output` afterwards (revision 2) is what
    simulates the correction; nothing in this script or the build itself
    consults `LatenessPolicy`/`CorrectionPlan` (see the roadmap entry for
    why that distinction matters).
"""

import argparse
import csv
from datetime import datetime, timedelta, timezone


FIELDS = [
    "event_id", "timestamp", "week", "day", "event_name", "page_section",
    "page_path", "source", "campaign", "device_type", "country", "plan",
    "user_id", "session_id", "revenue",
]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", required=True)
    parser.add_argument("--initial-output", required=True)
    parser.add_argument("--seed", type=int, default=7)
    parser.add_argument("--users", type=int, default=4200)
    parser.add_argument("--days", type=int, default=14)
    parser.add_argument(
        "--late-day-offset", type=int, default=7,
        help="0-indexed day (within --days) that receives a late correction",
    )
    parser.add_argument("--late-count", type=int, default=6)
    parser.add_argument(
        "--start-date", default="2026-04-06",
        help="UTC date of day 0 (YYYY-MM-DD). Must match the build scripts' "
             "START_DATE: they derive window boundaries from it, and a "
             "mismatch builds windows over a range this stream has no "
             "events in — empty aggregates, no error.",
    )
    args = parser.parse_args()

    import random
    rng = random.Random(args.seed)

    sources = [("organic", "none", 0.34), ("google", "spring_search", 0.24),
               ("linkedin", "founders", 0.18), ("newsletter", "april_launch", 0.14),
               ("direct", "none", 0.10)]
    devices = ["desktop", "mobile", "tablet"]
    countries = ["US", "GB", "DE", "CA", "AU"]
    plans = ["free", "pro", "team"]
    pages = [
        ("marketing", "/", 0.24), ("marketing", "/pricing", 0.16),
        ("product", "/product", 0.18), ("product", "/integrations", 0.12),
        ("learn", "/docs/getting-started", 0.18), ("learn", "/blog/warehouse", 0.12),
    ]

    start = datetime.strptime(args.start_date, "%Y-%m-%d").replace(tzinfo=timezone.utc)
    late_day_start = start + timedelta(days=args.late_day_offset)
    events = []
    late_candidates = []
    event_no = 0

    for user_no in range(args.users):
        user_id = f"user_{user_no:04d}"
        source, campaign, _ = rng.choices(sources, weights=[x[2] for x in sources])[0]
        device = rng.choices(devices, weights=[0.56, 0.38, 0.06])[0]
        country = rng.choice(countries)
        plan = rng.choices(plans, weights=[0.68, 0.25, 0.07])[0]
        day_offset = rng.randrange(args.days)
        current = start + timedelta(days=day_offset, minutes=rng.randrange(1440))
        session_id = f"session_{user_no:04d}_0"
        page_count = rng.randint(2, 7)
        visited_pricing = False
        for _ in range(page_count):
            section, path, _ = rng.choices(pages, weights=[x[2] for x in pages])[0]
            if path == "/pricing":
                visited_pricing = True
            event_no += 1
            events.append({
                "event_id": f"evt_{event_no:06d}",
                "timestamp": current.isoformat(),
                "week": current.strftime("%G-W%V"),
                "day": current.strftime("%a"),
                "event_name": "page_view",
                "page_section": section,
                "page_path": path,
                "source": source,
                "campaign": campaign,
                "device_type": device,
                "country": country,
                "plan": plan,
                "user_id": user_id,
                "session_id": session_id,
                "revenue": 0,
            })
            current += timedelta(seconds=rng.randint(15, 240))

        if visited_pricing and rng.random() < 0.55:
            event_no += 1
            converted = rng.random() < (0.45 if plan != "free" else 0.16)
            revenue = 99 if converted and plan == "pro" else (249 if converted and plan == "team" else 0)
            record = {
                "event_id": f"evt_{event_no:06d}", "timestamp": current.isoformat(),
                "week": current.strftime("%G-W%V"), "day": current.strftime("%a"),
                "event_name": "signup_completed" if converted else "signup_started",
                "page_section": "account", "page_path": "/signup",
                "source": source, "campaign": campaign, "device_type": device,
                "country": country, "plan": plan, "user_id": user_id,
                "session_id": session_id, "revenue": revenue,
            }
            events.append(record)
            if converted and revenue > 0 and day_offset == args.late_day_offset:
                late_candidates.append(record)

    # Deterministic (seeded) choice of which signup_completed rows in the
    # target window simulate a late-arriving correction — these are the
    # only rows that differ between `--output` and `--initial-output`.
    rng.shuffle(late_candidates)
    late_events = late_candidates[: args.late_count]
    late_ids = {r["event_id"] for r in late_events}
    if len(late_events) < args.late_count:
        raise SystemExit(
            f"only found {len(late_events)} revenue-bearing signup_completed "
            f"event(s) on day offset {args.late_day_offset}, need "
            f"{args.late_count} — raise --users or --seed"
        )

    initial_events = [e for e in events if e["event_id"] not in late_ids]

    for path, rows in ((args.output, events), (args.initial_output, initial_events)):
        with open(path, "w", newline="") as handle:
            writer = csv.DictWriter(handle, fieldnames=FIELDS)
            writer.writeheader()
            writer.writerows(rows)

    print(
        f"wrote {len(events):,} events ({args.output}) and "
        f"{len(initial_events):,} events ({args.initial_output}) for "
        f"{args.users:,} users over {args.days} day(s); late window is "
        f"{late_day_start.date()} with {len(late_events):,} late-arriving "
        f"event(s): {sorted(late_ids)}"
    )


if __name__ == "__main__":
    main()
