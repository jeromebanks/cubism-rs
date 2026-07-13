# Cube spec reference (apiVersion v1)

The spec is the single declarative contract for a cube build. YAML and JSON
are equivalent; field names are camelCase at the top level. Unknown fields
are **rejected** (typo protection), and validation reports *every* problem
at once with human-readable messages.

```yaml
apiVersion: v1            # required, must be "v1"
name: web_events          # required, non-empty
dimensions: [...]         # required, at least one
filterRules: [...]        # optional, default []
measures: [...]           # required in practice (a cube with no measures fails at plan time)
includeGlobal: false      # optional, default false
maxDictionaryEntries: 1000000  # optional; build fails fast past this many distinct
                               # interned dimension values — the guardrail against
                               # UUID/timestamp dimensions (docs/high-cardinality.md)
```

## `dimensions`

```yaml
dimensions:
  - name: geo                    # required; appears in YPaths (/geo/...)
    levels: [country, city]      # hierarchy, outermost first
  - name: platform
    levels:
      - name: device             # long form:
        expr: device_type        #   expr = SQL expression (default: the level name as a column)
      - browser                  # short and long forms mix freely
  - name: gender                 # no levels => one implicit level named after the dimension
```

| Field | Rules |
|---|---|
| `name` | non-empty; must not contain `/`, `=`, `,`; unique across dimensions |
| `levels[].name` | same character rules; unique within the dimension |
| `levels[].expr` | any SQL expression the engine can evaluate over one row; defaults to the level name |

Semantics:

- Level values are stringified (`CAST(expr AS VARCHAR)`) — YPath values are
  strings by definition.
- A row contributes every *prefix* of the hierarchy it has values for, and
  the hierarchy **truncates at the first NULL**: `country=CZ, city=NULL`
  yields only `/geo/country=CZ`; `country=NULL` yields no geo YPath at all
  (the row still lands in cells of other dimensions and `/G`).
- Dimension order in the spec is irrelevant; the engine canonicalizes by
  sorting dimension names.

## `filterRules`

A list of tagged rules; a cell is kept iff **all** rules accept it (empty
list keeps everything). `includeGlobal`'s `/G` bypasses the rules.

```yaml
filterRules:
  - type: max_dimensions          # ≤ n dimensions — put one in every production spec;
    n: 3                          #   it also bounds lattice *generation*, not just output
  - type: min_dimensions
    n: 1
  - type: contains_dim            # only cells including this dimension
    dim: geo
  - type: contains_all_dims
    dims: [geo, gender]
  - type: top_level               # only the standalone /geo/... cell
    dim: geo
  - type: not_together            # never combine these dimensions
    dims: [geo, platform]
  - type: not_with_attribute      # don't combine `dim` with `other_dim` once it
    dim: gender                   #   has drilled down to attribute `attr`
    other_dim: geo
    attr: city
  - type: not_alone               # this dimension never appears as a 1-dim cell
    dim: gender
  - type: only_with               # alias of contains_dim (reads better in some specs)
    dim: geo
  - type: and                     # combinators nest arbitrarily
    rules: [...]
  - type: or
    rules: [...]
```

Validation: every `dim` referenced must be a declared dimension;
`max_dimensions` requires `n ≥ 1` (n=0 would exclude every cell).

## `measures`

```yaml
measures:
  - name: pvs                    # output column name; unique
    agg: sum                     # aggregation kind (table below)
    input: pv                    # SQL expression; required except for `count`
  - name: top_tools
    agg: top_k
    input: tool_name             # top_k only: input = the KEY...
    by: tokens                   # ...and `by` = the score expression (required)
```

| `agg` | Input type | Output column(s) | Notes |
|---|---|---|---|
| `sum`, `min`, `max`, `avg` | numeric expr | `{name}` | engine-native |
| `count` | — (no `input`) | `{name}` | `COUNT(*)` per cell |
| `count_distinct` | any (stringified) | `{name}` (float estimate) + `{name}__sketch` | KMV sketch, k=1024; ~3.1% typical error; blob supports ∪ / ∩ / Jaccard |
| `top_k` | key expr + `by` score expr | `{name}` (JSON `[{"key","score"},...]`) + `{name}__sketch` | sums scores per key; top 100; approximate on very long tails |
| `reservoir_sample` | any (stringified) | `{name}` (JSON array) + `{name}__sketch` | up to 64 exemplar *distinct* values, deterministic |
| `centroid` | `DOUBLE[]` column | `{name}` (`DOUBLE[]` mean) + `{name}__sketch` | element-wise mean; all vectors must share a dimension |
| `quantile` | — | — | reserved; fails at plan time ("not implemented") |

Validation: names unique; `input` required except for `count`; `by` required
for `top_k` and invalid for everything else.

The `__sketch` companion columns hold versioned binary buffers (formats in
`docs/sketches.md`). Store them: they are what makes cubes mergeable and
set-operable after the fact. Sketch sizes (k=1024, top-100, 64 exemplars)
are engine defaults in this release; per-measure overrides are planned.

## `includeGlobal`

When `true`, the global rollup cell `/G` is appended to every row's
explosion, **after** filter rules (it cannot be filtered out). `/G` is what
turns "share of total" questions into a single join.

## Output shape

One row per surviving cell:

```text
xunit: Utf8            -- canonical cell string, e.g. "/gender/gender=F,/geo/country=CZ"
<measure columns>      -- in spec order; sketch measures contribute value + blob
```

The `xunit` string is canonical across builds (normalized dimension order),
so cubes from different runs join/merge on it directly.

## Error examples

Validation collects everything wrong at once:

```text
invalid cube spec:
apiVersion 'v2' is not supported (expected 'v1')
duplicate dimension name 'geo'
filter rule references unknown dimension 'nope' (declared dimensions: geo)
max_dimensions n must be at least 1 (0 excludes every cell)
measure 'm': agg 'Sum' requires an 'input' column or expression
measure 'top_tools': top_k requires 'by' (the score expression; 'input' is the key)
```

A misspelled field fails at parse time with the offending name
(`unknown field 'dimenssions'`).
