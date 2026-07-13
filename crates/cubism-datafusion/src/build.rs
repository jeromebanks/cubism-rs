//! End-to-end cube build: spec + registered source table → aggregated cube.
//!
//! The build compiles the spec into one SQL statement over the source:
//!
//! ```sql
//! WITH __cubism_input AS (
//!   SELECT CAST(<level expr> AS VARCHAR) AS __l0, ..., <measure input> AS __m0, ...
//!   FROM <source>
//! ),
//! __cubism_exploded AS (
//!   SELECT unnest(cubism_xunit_keys(__l0, ...)) AS __xunit_key, __m0, ...
//!   FROM __cubism_input
//! ),
//! __cubism_cells AS (
//!   SELECT __xunit_key, SUM(__m0) AS "pageviews", ... FROM __cubism_exploded
//!   GROUP BY __xunit_key
//! )
//! SELECT cubism_xunit_str(__xunit_key) AS xunit, "pageviews", ...
//! FROM __cubism_cells
//! ```
//!
//! Level expressions and measure inputs are raw SQL from the spec, evaluated
//! by DataFusion — that's the design: the spec's `expr` fields are the
//! engine's expression language.

use crate::udf::{cube_udfs, SharedDictionary};
use cubism_core::{AggKind, CubeSpec};
use datafusion::common::{plan_err, Result};
use datafusion::dataframe::DataFrame;
use datafusion::execution::context::SessionContext;

/// Compile and run a cube build over `source` (an already-registered table
/// or a subquery). Returns the lazy DataFrame (collect/stream/write it) and
/// the string dictionary used by this build.
pub async fn build_cube(
    ctx: &SessionContext,
    spec: &CubeSpec,
    source: &str,
) -> Result<(DataFrame, SharedDictionary)> {
    spec.validate().map_err(|e| datafusion::common::DataFusionError::Plan(e.to_string()))?;

    let (keys_udf, decode_udf, dict) = cube_udfs(spec);
    ctx.register_udf(keys_udf);
    ctx.register_udf(decode_udf);
    let (udafs, presenters) = crate::udaf::sketch_udfs();
    for udaf in udafs {
        ctx.register_udaf(udaf);
    }
    for udf in presenters {
        ctx.register_udf(udf);
    }

    let sql = cube_sql(spec, source)?;
    let df = ctx.sql(&sql).await?;
    Ok((df, dict))
}

/// Generate the cube-build SQL for a spec (exposed for inspection/tests).
pub fn cube_sql(spec: &CubeSpec, source: &str) -> Result<String> {
    let mut level_selects = Vec::new();
    let mut level_args = Vec::new();
    for dim in spec.sorted_dimensions() {
        for level in dim.effective_levels() {
            let alias = format!("__l{}", level_args.len());
            level_selects.push(format!("CAST({} AS VARCHAR) AS {alias}", level.expression()));
            level_args.push(alias);
        }
    }

    // Each measure contributes: projections of its input expression(s) into
    // aliased columns (which the explode stage passes through), one aggregate
    // over those columns, and its final-select presentation. Sketch measures
    // emit two final columns: the presented value and the mergeable sketch
    // blob (the serving layer's set-ops and incremental merges consume the
    // blob).
    let mut measure_selects: Vec<String> = Vec::new();
    let mut passthrough: Vec<String> = Vec::new();
    let mut agg_selects = Vec::new();
    let mut final_cols = Vec::new();
    for (i, measure) in spec.measures.iter().enumerate() {
        let alias = format!("__m{i}");
        let name = &measure.name;
        let input = || measure.input.as_ref().unwrap();
        let sketch_finals = |presenter: &str| {
            format!("{presenter}(\"{name}__sketch\") AS \"{name}\", \"{name}__sketch\"")
        };
        match measure.agg {
            AggKind::Sum | AggKind::Min | AggKind::Max | AggKind::Avg => {
                let f = match measure.agg {
                    AggKind::Sum => "SUM",
                    AggKind::Min => "MIN",
                    AggKind::Max => "MAX",
                    _ => "AVG",
                };
                measure_selects.push(format!("{} AS {alias}", input()));
                passthrough.push(alias.clone());
                agg_selects.push(format!("{f}({alias}) AS \"{name}\""));
                final_cols.push(format!("\"{name}\""));
            }
            AggKind::Count => {
                agg_selects.push(format!("COUNT(*) AS \"{name}\""));
                final_cols.push(format!("\"{name}\""));
            }
            AggKind::CountDistinct => {
                measure_selects.push(format!("CAST({} AS VARCHAR) AS {alias}", input()));
                passthrough.push(alias.clone());
                agg_selects.push(format!("cubism_kmv_sketch({alias}) AS \"{name}__sketch\""));
                final_cols.push(sketch_finals("cubism_kmv_estimate"));
            }
            AggKind::TopK => {
                let by = measure.by.as_ref().unwrap();
                measure_selects.push(format!("CAST({} AS VARCHAR) AS {alias}k", input()));
                measure_selects.push(format!("CAST({by} AS DOUBLE) AS {alias}s"));
                passthrough.push(format!("{alias}k"));
                passthrough.push(format!("{alias}s"));
                agg_selects
                    .push(format!("cubism_topk_sketch({alias}k, {alias}s) AS \"{name}__sketch\""));
                final_cols.push(sketch_finals("cubism_topk_json"));
            }
            AggKind::ReservoirSample => {
                measure_selects.push(format!("CAST({} AS VARCHAR) AS {alias}", input()));
                passthrough.push(alias.clone());
                agg_selects.push(format!("cubism_sample_sketch({alias}) AS \"{name}__sketch\""));
                final_cols.push(sketch_finals("cubism_sample_json"));
            }
            AggKind::Centroid => {
                measure_selects.push(format!("{} AS {alias}", input()));
                passthrough.push(alias.clone());
                agg_selects
                    .push(format!("cubism_centroid_sketch({alias}) AS \"{name}__sketch\""));
                final_cols.push(sketch_finals("cubism_centroid_mean"));
            }
            AggKind::Quantile => {
                return plan_err!(
                    "measure '{name}': quantile is not implemented in the engine yet"
                );
            }
        }
    }
    if agg_selects.is_empty() {
        return plan_err!("cube spec has no measures");
    }

    let input_cols = level_selects.iter().chain(&measure_selects).cloned().collect::<Vec<_>>();
    Ok(format!(
        "WITH __cubism_input AS (\n  SELECT {input}\n  FROM {source}\n),\n\
         __cubism_exploded AS (\n  SELECT unnest(cubism_xunit_keys({args})) AS __xunit_key{measures}\n  FROM __cubism_input\n),\n\
         __cubism_cells AS (\n  SELECT __xunit_key, {aggs}\n  FROM __cubism_exploded\n  GROUP BY __xunit_key\n)\n\
         SELECT cubism_xunit_str(__xunit_key) AS xunit, {finals}\nFROM __cubism_cells",
        input = input_cols.join(", "),
        args = level_args.join(", "),
        measures = passthrough.iter().map(|a| format!(", {a}")).collect::<String>(),
        aggs = agg_selects.join(", "),
        finals = final_cols.join(", "),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use datafusion::arrow::array::{Int64Array, RecordBatch, StringArray};
    use datafusion::arrow::datatypes::{DataType, Field, Schema};
    use std::collections::HashMap;
    use std::sync::Arc;

    fn sample_batch() -> RecordBatch {
        let schema = Arc::new(Schema::new(vec![
            Field::new("country", DataType::Utf8, true),
            Field::new("city", DataType::Utf8, true),
            Field::new("gender", DataType::Utf8, true),
            Field::new("pv", DataType::Int64, false),
            Field::new("user_id", DataType::Utf8, false),
        ]));
        RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec![Some("CZ"), Some("CZ"), Some("US"), None])),
                Arc::new(StringArray::from(vec![Some("Prague"), Some("Brno"), None, None])),
                Arc::new(StringArray::from(vec![Some("F"), Some("M"), Some("F"), Some("F")])),
                Arc::new(Int64Array::from(vec![10, 20, 40, 80])),
                Arc::new(StringArray::from(vec!["u1", "u2", "u1", "u3"])),
            ],
        )
        .unwrap()
    }

    const SPEC: &str = r#"
apiVersion: v1
name: test_cube
dimensions:
  - name: geo
    levels: [country, city]
  - name: gender
measures:
  - name: pvs
    agg: sum
    input: pv
  - name: events
    agg: count
includeGlobal: true
"#;

    async fn run_cube(spec_yaml: &str) -> HashMap<String, (i64, i64)> {
        let spec = CubeSpec::from_yaml(spec_yaml).unwrap();
        let ctx = SessionContext::new();
        ctx.register_batch("events", sample_batch()).unwrap();
        let (df, _dict) = build_cube(&ctx, &spec, "events").await.unwrap();
        let batches = df.collect().await.unwrap();

        let mut cells = HashMap::new();
        for batch in &batches {
            let xunit = batch.column(0).as_any().downcast_ref::<StringArray>().unwrap();
            let pvs = batch.column(1).as_any().downcast_ref::<Int64Array>().unwrap();
            let events = batch.column(2).as_any().downcast_ref::<Int64Array>().unwrap();
            for i in 0..batch.num_rows() {
                cells.insert(xunit.value(i).to_string(), (pvs.value(i), events.value(i)));
            }
        }
        cells
    }

    #[tokio::test]
    async fn dictionary_guard_fails_fast_on_high_cardinality_dimension() {
        // user_id as a dimension: 3 distinct values but the cap allows far
        // fewer interned strings, standing in for the UUID-dimension case.
        let spec_yaml = r#"
apiVersion: v1
name: exploding
dimensions:
  - name: user_id
  - name: gender
measures:
  - name: events
    agg: count
maxDictionaryEntries: 3
includeGlobal: true
"#;
        let spec = CubeSpec::from_yaml(spec_yaml).unwrap();
        let ctx = SessionContext::new();
        ctx.register_batch("events", sample_batch()).unwrap();
        let (df, _dict) = build_cube(&ctx, &spec, "events").await.unwrap();
        let err = df.collect().await.unwrap_err().to_string();
        assert!(err.contains("maxDictionaryEntries"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn end_to_end_cube_matches_hand_computed_cells() {
        let cells = run_cube(SPEC).await;

        // Global rollup sees every row.
        assert_eq!(cells["/G"], (150, 4));
        // Country level: CZ rows 1+2, US row 3; row 4 has null country (no geo cell).
        assert_eq!(cells["/geo/country=CZ"], (30, 2));
        assert_eq!(cells["/geo/country=US"], (40, 1));
        // City level: US row has null city -> truncated at country.
        assert_eq!(cells["/geo/country=CZ/city=Prague"], (10, 1));
        assert!(!cells.keys().any(|k| k.contains("US") && k.contains("city")));
        // Flat dimension: gender F = rows 1,3,4.
        assert_eq!(cells["/gender/gender=F"], (130, 3));
        // Cross cells combine dimensions.
        assert_eq!(cells["/gender/gender=F,/geo/country=CZ"], (10, 1));
        assert_eq!(cells["/gender/gender=F,/geo/country=CZ/city=Prague"], (10, 1));

        // Distinct cells across all rows: geo {CZ, CZ/Prague, CZ/Brno, US} +
        // gender {F, M} + crosses {CZ+F, CZ/Prague+F, CZ+M, CZ/Brno+M, US+F}
        // + global = 12.
        assert_eq!(cells.len(), 12);
    }

    #[tokio::test]
    async fn filter_rules_prune_the_cube() {
        let spec_filtered = SPEC.replace(
            "measures:",
            "filterRules:\n  - type: top_level\n    dim: geo\nmeasures:",
        );
        let cells = run_cube(&spec_filtered).await;
        // Only single-dimension geo cells survive, plus the rule-bypassing global.
        assert_eq!(cells["/geo/country=CZ"], (30, 2));
        assert!(cells.contains_key("/G"));
        assert!(!cells.keys().any(|k| k.contains("gender")));
        assert_eq!(cells.len(), 5); // CZ, CZ/Prague, CZ/Brno, US, /G
    }

    #[tokio::test]
    async fn top_k_and_sample_measures_work_end_to_end() {
        let spec = CubeSpec::from_yaml(
            r#"
apiVersion: v1
name: topk_cube
dimensions:
  - name: gender
measures:
  - name: top_cities
    agg: top_k
    input: city
    by: pv
  - name: user_sample
    agg: reservoir_sample
    input: user_id
includeGlobal: true
"#,
        )
        .unwrap();
        let ctx = SessionContext::new();
        ctx.register_batch("events", sample_batch()).unwrap();
        let (df, _dict) = build_cube(&ctx, &spec, "events").await.unwrap();
        let batches = df.collect().await.unwrap();

        let mut cells: HashMap<String, (String, String)> = HashMap::new();
        for batch in &batches {
            let xunit = batch.column(0).as_any().downcast_ref::<StringArray>().unwrap();
            let topk = batch.column(1).as_any().downcast_ref::<StringArray>().unwrap();
            let sample = batch.column(3).as_any().downcast_ref::<StringArray>().unwrap();
            for i in 0..batch.num_rows() {
                cells.insert(
                    xunit.value(i).to_string(),
                    (topk.value(i).to_string(), sample.value(i).to_string()),
                );
            }
        }

        // Global: Prague pv=10, Brno pv=20 (rows 3,4 have null city — no key).
        let (topk, sample) = &cells["/G"];
        assert_eq!(topk, r#"[{"key":"Brno","score":20.0},{"key":"Prague","score":10.0}]"#);
        // Distinct users u1,u2,u3 all fit in the sample.
        let users: Vec<String> = serde_json::from_str(sample).unwrap();
        let mut sorted = users.clone();
        sorted.sort();
        assert_eq!(sorted, vec!["u1", "u2", "u3"]);
    }

    #[tokio::test]
    async fn centroid_measure_works_on_embedding_columns() {
        use datafusion::arrow::array::{Float64Builder, ListArray, ListBuilder};

        let mut emb = ListBuilder::new(Float64Builder::new());
        emb.values().append_slice(&[1.0, 0.0]);
        emb.append(true);
        emb.values().append_slice(&[0.0, 1.0]);
        emb.append(true);
        let emb: ListArray = emb.finish();

        let schema = Arc::new(Schema::new(vec![
            Field::new("channel", DataType::Utf8, false),
            Field::new(
                "embedding",
                DataType::List(Arc::new(Field::new("item", DataType::Float64, true))),
                true,
            ),
        ]));
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(StringArray::from(vec!["eng", "eng"])),
                Arc::new(emb),
            ],
        )
        .unwrap();

        let spec = CubeSpec::from_yaml(
            r#"
apiVersion: v1
name: gbrain_cube
dimensions:
  - name: channel
measures:
  - name: topic_centroid
    agg: centroid
    input: embedding
"#,
        )
        .unwrap();
        let ctx = SessionContext::new();
        ctx.register_batch("docs", batch).unwrap();
        let (df, _dict) = build_cube(&ctx, &spec, "docs").await.unwrap();
        let batches = df.collect().await.unwrap();

        let batch = &batches[0];
        assert_eq!(batch.num_rows(), 1); // one cell: /channel/channel=eng
        let means = batch.column(1).as_any().downcast_ref::<ListArray>().unwrap();
        let mean = means.value(0);
        let mean = mean
            .as_any()
            .downcast_ref::<datafusion::arrow::array::Float64Array>()
            .unwrap();
        assert_eq!((mean.value(0), mean.value(1)), (0.5, 0.5));
    }

    #[tokio::test]
    async fn unimplemented_aggs_report_planning_error() {
        let spec = CubeSpec::from_yaml(&SPEC.replace("agg: sum", "agg: quantile")).unwrap();
        let err = cube_sql(&spec, "events").unwrap_err().to_string();
        assert!(err.contains("not implemented"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn count_distinct_produces_exact_estimates_and_mergeable_sketches() {
        use cubism_core::sketch::KmvSketch;
        use datafusion::arrow::array::{BinaryArray, Float64Array};

        let spec = CubeSpec::from_yaml(&SPEC.replace(
            "measures:",
            "measures:\n  - name: reach\n    agg: count_distinct\n    input: user_id",
        ))
        .unwrap();
        let ctx = SessionContext::new();
        ctx.register_batch("events", sample_batch()).unwrap();
        let (df, _dict) = build_cube(&ctx, &spec, "events").await.unwrap();
        let batches = df.collect().await.unwrap();

        let mut reach = HashMap::new();
        let mut sketches: HashMap<String, KmvSketch> = HashMap::new();
        for batch in &batches {
            let xunit = batch.column(0).as_any().downcast_ref::<StringArray>().unwrap();
            let est = batch.column(1).as_any().downcast_ref::<Float64Array>().unwrap();
            let blob = batch.column(2).as_any().downcast_ref::<BinaryArray>().unwrap();
            for i in 0..batch.num_rows() {
                reach.insert(xunit.value(i).to_string(), est.value(i));
                sketches
                    .insert(xunit.value(i).to_string(), KmvSketch::from_bytes(blob.value(i)).unwrap());
            }
        }

        // Under-full sketches are exact: users are u1,u2,u1,u3.
        assert_eq!(reach["/G"], 3.0);
        assert_eq!(reach["/gender/gender=F"], 2.0); // u1, u3
        assert_eq!(reach["/gender/gender=M"], 1.0); // u2
        assert_eq!(reach["/geo/country=CZ"], 2.0); // u1, u2

        // The blobs are mergeable and set-operable at query time: F ∪ M = /G,
        // F ∩ CZ = {u1}.
        let f = &sketches["/gender/gender=F"];
        let m = &sketches["/gender/gender=M"];
        let cz = &sketches["/geo/country=CZ"];
        assert_eq!(f.merge(m).estimate(), 3.0);
        assert_eq!(f.intersection_estimate(cz), 1.0);
    }
}
