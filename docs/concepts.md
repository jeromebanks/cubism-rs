# Concepts: the Cubism data model

Cubism answers one question well: *given a stream of events, what is the
value of every interesting aggregate over every interesting combination of
dimensions?* This doc defines the vocabulary the whole system is built on.

## YPath — one dimension's coordinate

A **YPath** names a position within a single dimension, possibly at depth in
a hierarchy:

```text
/geo/country=CZ                     one level deep
/geo/country=CZ/city=Prague         two levels deep
/tool/tool=Bash                     a flat dimension
```

The general shape is `/<dimension>/<attr>=<value>/<attr>=<value>/...` where
the attribute *sequence* encodes drill-down: `country=CZ/city=Prague` means
"Prague, within CZ" — not two independent facts. Values are escaped so they
can contain the structural characters `/`, `=`, `,`.

A row that has `country=CZ, city=Prague` belongs to **both**
`/geo/country=CZ` and `/geo/country=CZ/city=Prague` — every prefix of the
hierarchy. A row with a country but no city belongs only to the country
prefix (the hierarchy truncates at the first missing level).

## XUnit — one cell of the cube

An **XUnit** is a conjunction of YPaths, at most one per dimension — a
single cell of an OLAP cube:

```text
/gender/gender=F,/geo/country=CZ/city=Prague
```

reads as "female AND in Prague". The empty conjunction is the **global
rollup**, written `/G` — the cell every row belongs to.

XUnits are *normalized*: YPaths are sorted by dimension name, so a cell has
exactly one canonical string form. That string is globally stable — two
independent cube builds of overlapping data produce byte-identical strings
for the same cell, which is what makes cross-build merging possible (see
`docs/scaling.md`).

## The lattice — which cells exist

Each input row *explodes* into every cell it belongs to. With dimensions
`geo` (2 hierarchy levels), `platform` (2 levels), and `gender` (flat), one
fully-populated row belongs to

```text
(2 + 1) × (2 + 1) × (1 + 1) − 1 = 17 cells        (+1 with the global /G)
```

— each dimension contributes "absent" or one of its hierarchy depths, minus
the all-absent case. This is the **cube lattice**, and it grows as the
product of (depth+1) over dimensions: 6 hierarchical dimensions of depth 2
already mean 728 cells *per row*. Uncontrolled, that's the classic OLAP
explosion.

## Filter rules — pruning the lattice

**Filter rules** declare which cells are worth materializing. An XUnit is
kept only if **every** rule accepts it (rules are a conjunction; use
`or`/`and` combinators for anything fancier). The global `/G`, when
requested, bypasses the rules.

| Rule | Keeps a cell iff |
|---|---|
| `max_dimensions {n}` | it has ≤ n dimensions |
| `min_dimensions {n}` | it has ≥ n dimensions |
| `contains_dim {dim}` | it includes that dimension |
| `contains_all_dims {dims}` | it includes all of them |
| `top_level {dim}` | it is exactly the single-dimension cell of `dim` |
| `not_together {dims}` | it does *not* contain all of `dims` together |
| `not_with_attribute {dim, other_dim, attr}` | it doesn't combine `dim` with `other_dim` drilled to `attr` |
| `not_alone {dim}` | it isn't the standalone single-dimension cell of `dim` |
| `only_with {dim}` | it includes that dimension |
| `and {rules}` / `or {rules}` | combinators |

`max_dimensions` is special: the engine uses it to stop *generating*
combinations past the bound, not just to filter them afterward — it's the
rule that keeps wide specs tractable, and every production spec should have
one.

## Measures — what's computed per cell

A **measure** is a named aggregate evaluated for every cell. Two families:

- **Scalar measures** (`sum`, `count`, `min`, `max`, `avg`) — one number per
  cell, computed by the engine's native aggregation.
- **Sketch measures** (`count_distinct`, `top_k`, `reservoir_sample`,
  `centroid`) — per cell, a compact **mergeable buffer** plus a presented
  value. The buffer ("sketch blob") is the real product: two blobs combine
  associatively, so cells can be unioned/intersected/extended *after* the
  build, at query time, without touching raw data.

Why mergeability is the load-bearing property:

```text
sessions(Bash) ∩ sessions(error)
```

is answerable from the stored blobs of two cells that were never aggregated
together — a query pre-aggregation systems classically cannot serve. The
same property makes distributed builds (merge partial cubes) and streaming
increments (merge micro-batches into existing cells) fall out for free.

## The spec — the single contract

A cube is declared entirely by a YAML/JSON **spec**: dimensions (with
hierarchy levels and SQL extractor expressions), filter rules, measures, and
whether to include `/G`. The spec is the API between users, the engine, the
CLI, the Python bindings, and (eventually) the hosted service — see
`docs/spec-reference.md` for every field.

## Worked example

```yaml
apiVersion: v1
name: web_events
dimensions:
  - name: geo
    levels: [country, city]
  - name: gender
filterRules:
  - type: max_dimensions
    n: 2
measures:
  - name: reach
    agg: count_distinct
    input: user_id
includeGlobal: true
```

Input row `country=CZ, city=Prague, gender=F, user_id=u1` explodes into:

```text
/G
/geo/country=CZ
/geo/country=CZ/city=Prague
/gender/gender=F
/gender/gender=F,/geo/country=CZ
/gender/gender=F,/geo/country=CZ/city=Prague
```

and `u1` enters the `reach` KMV sketch of each. The output cube is one row
per surviving cell: `xunit`, `reach` (the estimate), `reach__sketch` (the
mergeable blob).

## Lineage

The model is a Rust rewrite of *Qubism*, a Scala/Spark library built for
audience analytics at Demandbase — YPath/XUnit/FilterRule and the mergeable
aggregator design are inherited; the string-key hot path, deprecated Spark
plumbing, and pre-lakehouse storage layers were deliberately left behind.
