# Cubism web analytics demo

This is a small Google Analytics / Quantcast-style demo for Cubism.

It models a fictional product site, **Northstar**, with acquisition,
behavior, device, geography, and conversion events. Cubism turns those events
into a multidimensional cube and serves an interactive dashboard.

## Run it

From the `cubism/` directory:

```bash
# 1. Generate deterministic web events.
python3 examples/web_analytics_demo/generate_events.py \
  --output examples/web_analytics_demo/events.csv

# 2. Validate and build the Cubism cube.
cargo run --release -p cubism-cli -- validate \
  examples/web_analytics_demo/web_analytics.yaml
cargo run --release -p cubism-cli -- run \
  examples/web_analytics_demo/web_analytics.yaml \
  --input examples/web_analytics_demo/events.csv \
  --output examples/web_analytics_demo/web_analytics_cube.parquet \
  --show 10

# 3. Serve the cube and open http://127.0.0.1:8090.
cargo run --release -p cubism-cli -- serve \
  examples/web_analytics_demo/web_analytics_cube.parquet --port 8090
```

Open `site/index.html` separately to see the small product site that the
events represent. Its event names and columns match the generated data.

## What to demo

The dashboard gives you:

- total events, users, sessions, and revenue from the `/G` rollup;
- slices by acquisition source, device, country, page, or event;
- cell inspection with distinct-count sketches and top pages;
- ad-hoc audience overlap, such as `pricing` visitors ∩ `signup_completed`
  sessions, even though that pair was never materialized as a cube cell.

The last item is the Cubism-specific story: the dashboard computes the
intersection from stored KMV sketches at query time.
