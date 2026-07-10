# Sketch internals: algorithms, error bounds, byte formats

Every sketch is a **mergeable buffer**: associative + commutative under
`merge`, serialized with a versioned header, stable across releases.
Blobs are persisted state — treat a format change like a database migration
(bump the version byte, keep a decode path). Golden-file tests in
`cubism-core` lock each format, *including the hash function*.

All integers little-endian. All hashes are **xxh3-64, seed 0**, over the
value's UTF-8 bytes (`sketch::kmv::hash_value`) — the hash is part of the
format contract: change it and every persisted sketch silently corrupts.

---

## KMV — distinct count + set algebra (`count_distinct`)

**Idea.** Hash every value to a uniform 64-bit point; keep only the `k`
smallest distinct hashes. If the k-th smallest hash is `m`, the set's hashes
have density ≈ `k / (m / 2⁶⁴)`, so distinct count ≈ `(k−1) / (m / 2⁶⁴)`
(the unbiased form). While fewer than `k` distinct values have been seen the
sketch is simply *exact* — it holds all their hashes.

**Merge** = sorted union of the two hash lists, truncated to
`min(k_left, k_right)`. Exactly associative/commutative/idempotent
(property-tested).

**Set operations** (the payoff):

```text
|A ∪ B| = estimate(merge(A, B))
|A ∩ B| = max(|A| + |B| − |A ∪ B|, 0)          (inclusion–exclusion)
J(A,B)  = |A ∩ B| / |A ∪ B|
```

**Error.** Relative standard error ≈ `1/√(k−2)`:

| k | typical error | blob size (full) |
|---|---|---|
| 256 | ~6.3% | ~2 KB |
| 1024 (default) | ~3.1% | ~8 KB |
| 4096 | ~1.6% | ~33 KB |

Intersection error compounds — inclusion–exclusion subtracts three noisy
estimates, so small intersections of large sets are the weak spot (relative
error grows as `|A∪B| / |A∩B|` shrinks the intersection). Jaccard values
land within a few points of truth in practice; treat ∩ estimates below ~1%
of the union as directional.

**Format v1** (`KMV`):

```text
"KMV"  version:u8=1  k:u32  n:u32  hash:u64 × n     (hashes sorted ascending, distinct)
```

Divergences from the legacy Scala implementation (deliberate, locked into
v1): xxh3 instead of MD5's first 8 bytes; unbiased `(k−1)/U` instead of the
biased `k/U`.

---

## TopK — heavy hitters by summed score (`top_k`)

**Idea.** A bounded map `key → Σscore`. Adds are exact while the number of
distinct keys fits within `capacity × 4` (the overflow headroom); beyond
that the lowest-scored keys are pruned. Presentation returns the top
`capacity` (default 100) entries, score-descending, key-ascending tiebreak.

**Merge** = sum matching keys over the union, re-prune. Exactly associative
until pruning has occurred; after pruning, results can depend on merge order
(the standard bounded-top-k trade-off — same caveat as any frequent-items
sketch). A key that stays in the tail everywhere but sums large globally can
be undercounted; keys that are large *anywhere* survive.

**Format v1** (`TPK`):

```text
"TPK"  version:u8=1  capacity:u32  n:u32  ( score:f64  key_len:u16  key:utf8 ) × n
```

entries serialized best-first. Presented as JSON:
`[{"key":"Bash","score":63809142.0}, ...]`.

Divergence from legacy `ArgMaxMap`: keys are deduplicated and scores summed
(the legacy kept a score-sorted list with possible duplicate keys and relied
on upstream pre-aggregation).

---

## ExemplarSample — deterministic distinct sample (`reservoir_sample`)

**Idea.** Keep the `capacity` (default 64) values whose hashes are smallest
— "bottom-k by hash". Because the hash is a pure function of the value, this
is a uniform sample over *distinct* values that is completely deterministic:
insertion order, batching, and partitioning cannot change the result.

**Merge** = sorted union by hash, truncate — identical mechanics to KMV,
so it is *exactly* associative and commutative (unlike a classic randomized
reservoir, whose merge cannot be made order-independent). That determinism
is why this implementation was chosen despite the `reservoir_sample` name.

Use it for exemplars: representative doc IDs / session IDs per cell — the
retrieval-unit primitive for RAG-over-cubes (fetch a cell's exemplars, let
an LLM summarize the slice).

**Format v1** (`SMP`):

```text
"SMP"  version:u8=1  capacity:u32  n:u32  ( hash:u64  len:u16  value:utf8 ) × n
```

entries sorted by hash ascending. Presented as a JSON array of values.

Caveats: it samples *distinct* values (frequency-blind — a value appearing
once and one appearing a million times are equally likely to be kept), and
values are stored verbatim, so sample IDs, not documents.

---

## Centroid — mean vector (`centroid`)

**Idea.** Store `(count, element-wise sums)`; present `sums / count`. The
first vector fixes the dimensionality; mismatched dimensions are a hard
error (fail loud beats silently averaging misaligned embeddings).

**Merge** = element-wise sum + count sum. Associative up to floating-point
rounding (f64 addition is not bit-exact under reassociation; differences are
at the 1e-15 level, irrelevant for embeddings).

Use per-cell centroids as the cell's *retrieval embedding*: semantic search
over cells = nearest-centroid search, then pull the cell's exemplar sample
for grounding.

**Format v1** (`CTR`):

```text
"CTR"  version:u8=1  count:u64  dim:u32  sum:f64 × dim
```

Presented as `DOUBLE[]` (the mean). Input must be a `DOUBLE[]`
(list-of-Float64) column; NULL elements inside a vector are treated as 0.0,
NULL vectors are skipped.

---

## Cross-cutting rules

1. **Blobs are the product.** Every sketch measure outputs
   `{name}` (presented) *and* `{name}__sketch` (the blob). Store the blob;
   re-present any time; merge across builds, windows, and cells.
2. **Merging different sizes** degrades to the smaller capacity
   (`min(k)`), never fabricates precision.
3. **Empty sketches are merge identities** — an empty cell merges as a
   no-op, so sparse partials are safe.
4. **Version discipline**: `from_bytes` rejects unknown magic/version and
   corrupt payloads (truncation, unsorted hashes, dimension mismatches)
   rather than guessing.

## Consuming blobs outside the engine

`cubism-core` is dependency-light (no DataFusion); any Rust service — or
Python via the bindings — can do query-time algebra on stored blobs:

```python
import cubism
bash = cubism.KmvSketch.from_bytes(cells["/tool/tool=Bash"]["sessions__sketch"])
err  = cubism.KmvSketch.from_bytes(cells["/outcome/outcome=error"]["sessions__sketch"])
bash.intersection_estimate(err)          # sessions that did both
cubism.topk_items(cells["/G"]["costliest_sessions__sketch"])
cubism.sample_values(cells["/G"]["exemplars__sketch"])
```
