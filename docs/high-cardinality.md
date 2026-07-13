# High-cardinality dimensions

*What happens when a dimension turns out to contain a UUID, a raw
timestamp, or free-form text — and what to do about it.*

This bit the legacy library in production, so it gets its own doc.

## Why it explodes

Cubism's lattice is generated over dimension **shapes** (which dimensions
at which levels), but cells are minted per distinct **value combination**
observed in the data. A dimension with `V` distinct values contributes
roughly `V × (shapes that include it)` cells. For `country` (~200 values)
that's the point of the tool. For `request_id` (one value per row) it means
the "aggregation" has one input row per output cell — you've built a very
expensive copy of the raw table, through three compounding failure modes:

1. **Cell explosion.** Every UUID mints cells for every shape that includes
   its dimension — crossed with every other dimension's values under
   `max_dimensions`. The GROUP BY hash table, the output cube, and every
   per-cell sketch scale with it.
2. **Dictionary growth.** The build's string dictionary interns every
   distinct level value. 100M distinct UUIDs = 100M interned strings held
   in memory for the whole build.
3. **Memo starvation.** The explode UDF memoizes lattice keys per distinct
   level-value tuple, on the (normally safe) assumption that tuples repeat.
   A high-cardinality dimension drives the hit rate to zero — every row
   pays full lattice generation, and the memo itself becomes deadweight
   memory. (The memo is size-capped for exactly this reason; see below.)

Note what does **not** help: `max_dimensions` and the other filter rules
bound the *shape* count, not the *value* count. A UUID dimension explodes
even at `max_dimensions: 1`.

## The guardrail: `maxDictionaryEntries`

Since v0.1, every build enforces a dictionary budget (default **1,000,000**
distinct interned strings, configurable per spec):

```yaml
maxDictionaryEntries: 250000   # optional; default 1000000
```

When a build crosses it, it fails fast — minutes of runaway memory replaced
by an immediate, explanatory error:

```
cube dictionary exceeded maxDictionaryEntries (1000000) — a dimension is
likely high-cardinality (a UUID, raw timestamp, or free-form text). ...
```

Distinct strings across *all* dimensions of a sane cube — geographies,
categories, product names, weeks — number in the thousands. If you
legitimately need more (say, a `repo` dimension across a giant org), raise
the limit deliberately; the point is that a UUID column can no longer take
the build down silently. The dictionary count is also printed on every
successful CLI run (`… (135 dictionary entries)`) — glance at it when
adding a dimension.

Internally, the same episode is why the explode memo is capped
(`MEMO_MAX_BUCKETS` in `udf.rs`): past the cap, rows compute directly
instead of growing the memo, so pathological inputs degrade smoothly
instead of quadratically.

## Modeling your way out

The guardrail tells you something is wrong; these patterns fix it. The rule
of thumb:

> **Dimensions are for slicing; identity belongs in measures.**

If you'd never put a value in a WHERE clause by hand (`request_id = '3f2a…'`),
it isn't a dimension.

### Identity columns (UUIDs, user IDs, session IDs) → sketch measures

You almost never want *cells per ID* — you want *per cell: how many IDs,
which IDs matter, which IDs to sample*. Those are exactly the sketch
measures, all bounded-size regardless of cardinality:

```yaml
measures:
  - name: sessions            # how many distinct?
    agg: count_distinct
    input: session_id
  - name: costliest_sessions  # which ones matter?
    agg: top_k
    input: session_id
    by: cost_usd
  - name: exemplar_sessions   # give me a few to inspect
    agg: reservoir_sample
    input: session_id
```

This is how the agent-trace demos handle `session_id` (200k distinct) —
and via KMV set algebra you still get overlap/affinity *between* ID
populations without ever making the ID a dimension.

### Timestamps → bucketed hierarchy levels

A raw timestamp is a unique value per row; time analysis wants buckets.
Level `expr` fields are SQL, so bucket at build time:

```yaml
dimensions:
  - name: time
    levels:
      - name: week
        expr: "date_trunc('week', ts)"
      - name: day
        expr: "date_trunc('day', ts)"
```

(The demos precompute `week`/`day` strings in the importer — same idea,
either side of the spec boundary is fine.)

### Numeric / long-tail values → CASE bands, prefixes

```yaml
dimensions:
  - name: latency_band
    levels:
      - name: band
        expr: "CASE WHEN duration_ms < 100 THEN 'fast'
                    WHEN duration_ms < 1000 THEN 'ok'
                    ELSE 'slow' END"
  - name: error
    levels:
      # first path segment, not the full URL
      - name: endpoint
        expr: "split_part(path, '/', 2)"
```

Free-form text (error messages, URLs, user agents) always needs one of
these; the raw column is effectively an ID.

### Legitimately large dimensions

A real dimension with 50k–500k values (SKUs, repos in a monorepo org)
works — it's just costly. Contain it with rules and hierarchy:

```yaml
filterRules:
  - type: max_dimensions
    n: 2
  - type: not_together        # don't cross the big dim with the other big one
    dims: [sku, time]
dimensions:
  - name: sku
    levels: [category, sku]  # rollup level = cheap slicing, leaf = rare drill
```

and raise `maxDictionaryEntries` explicitly so the intent is recorded in
the spec.

## Roadmap (not yet implemented)

- **Per-dimension budgets with attribution** — `maxValues` on a dimension,
  and error messages naming the offending dimension (the global budget
  can't tell you *which* dimension blew it; today you find it with
  `SELECT count(distinct col)` per level column, or by bisecting the spec).
- **Top-N + `__other__` rollup** — keep a dimension's N heaviest values as
  real cells and fold the tail into one `__other__` value, sketch-guided.
  This is the "right" fix for Zipfian dimensions like `endpoint`.
- **`cubism validate --against <data>`** — a sampling pass that estimates
  per-level cardinality and warns before you build.
