"""Generate a small deterministic event stream for the Cubism web demo."""

import argparse
import csv
import random
from datetime import datetime, timedelta, timezone


FIELDS = [
    "event_id", "timestamp", "week", "day", "event_name", "page_section",
    "page_path", "source", "campaign", "device_type", "country", "plan",
    "user_id", "session_id", "revenue",
]


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("--output", required=True)
    parser.add_argument("--seed", type=int, default=42)
    parser.add_argument("--users", type=int, default=420)
    args = parser.parse_args()
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
    events = []
    start = datetime(2026, 4, 6, tzinfo=timezone.utc)
    event_no = 0

    for user_no in range(args.users):
        user_id = f"user_{user_no:04d}"
        source, campaign, _ = rng.choices(sources, weights=[x[2] for x in sources])[0]
        device = rng.choices(devices, weights=[0.56, 0.38, 0.06])[0]
        country = rng.choice(countries)
        plan = rng.choices(plans, weights=[0.68, 0.25, 0.07])[0]
        session_count = 1 + (rng.random() < 0.22) + (rng.random() < 0.07)
        for session_no in range(session_count):
            session_id = f"session_{user_no:04d}_{session_no}"
            day_offset = rng.randrange(28)
            current = start + timedelta(days=day_offset, minutes=rng.randrange(1440))
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

            if visited_pricing and rng.random() < 0.42:
                event_no += 1
                converted = rng.random() < (0.45 if plan != "free" else 0.16)
                events.append({
                    "event_id": f"evt_{event_no:06d}", "timestamp": current.isoformat(),
                    "week": current.strftime("%G-W%V"), "day": current.strftime("%a"),
                    "event_name": "signup_completed" if converted else "signup_started",
                    "page_section": "account", "page_path": "/signup",
                    "source": source, "campaign": campaign, "device_type": device,
                    "country": country, "plan": plan, "user_id": user_id,
                    "session_id": session_id, "revenue": 99 if converted and plan == "pro" else (249 if converted and plan == "team" else 0),
                })

    with open(args.output, "w", newline="") as handle:
        writer = csv.DictWriter(handle, fieldnames=FIELDS)
        writer.writeheader()
        writer.writerows(events)
    print(f"wrote {len(events):,} events for {args.users:,} users to {args.output}")


if __name__ == "__main__":
    main()
