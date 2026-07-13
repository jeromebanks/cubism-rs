//! End-to-end: build a real cube with cubism-datafusion, write it to
//! parquet, load it through the store, and exercise every endpoint over a
//! live listener.

use cubism_core::CubeSpec;
use cubism_datafusion::build_cube;
use cubism_datafusion::datafusion::arrow::array::{Int64Array, RecordBatch, StringArray};
use cubism_datafusion::datafusion::arrow::datatypes::{DataType, Field, Schema};
use cubism_datafusion::datafusion::dataframe::DataFrameWriteOptions;
use cubism_datafusion::datafusion::prelude::SessionContext;
use cubism_serve::CubeStore;
use serde_json::Value;
use std::sync::Arc;

const SPEC: &str = r#"
apiVersion: v1
name: serve_test
dimensions:
  - name: tool
  - name: outcome
filterRules:
  - type: max_dimensions
    n: 2
measures:
  - name: tokens
    agg: sum
    input: tokens
  - name: calls
    agg: count
  - name: sessions
    agg: count_distinct
    input: session_id
  - name: top_sessions
    agg: top_k
    input: session_id
    by: tokens
includeGlobal: true
"#;

async fn cube_file(dir: &tempfile::TempDir) -> String {
    let schema = Arc::new(Schema::new(vec![
        Field::new("tool", DataType::Utf8, false),
        Field::new("outcome", DataType::Utf8, false),
        Field::new("tokens", DataType::Int64, false),
        Field::new("session_id", DataType::Utf8, false),
    ]));
    // s1: Bash ok + Bash error; s2: Bash ok + Read ok; s3: Read error
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(StringArray::from(vec!["Bash", "Bash", "Bash", "Read", "Read"])),
            Arc::new(StringArray::from(vec!["ok", "error", "ok", "ok", "error"])),
            Arc::new(Int64Array::from(vec![10, 20, 30, 40, 50])),
            Arc::new(StringArray::from(vec!["s1", "s1", "s2", "s2", "s3"])),
        ],
    )
    .unwrap();

    let ctx = SessionContext::new();
    ctx.register_batch("events", batch).unwrap();
    let spec = CubeSpec::from_yaml(SPEC).unwrap();
    let (df, _dict) = build_cube(&ctx, &spec, "events").await.unwrap();
    let path = dir.path().join("cube.parquet").to_str().unwrap().to_string();
    df.write_parquet(&path, DataFrameWriteOptions::new(), None).await.unwrap();
    path
}

async fn get(base: &str, path: &str) -> (u16, Value) {
    // stdlib-free-of-clients: raw HTTP over tokio TcpStream keeps dev-deps lean
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = tokio::net::TcpStream::connect(base).await.unwrap();
    stream
        .write_all(format!("GET {path} HTTP/1.1\r\nHost: {base}\r\nConnection: close\r\n\r\n").as_bytes())
        .await
        .unwrap();
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).await.unwrap();
    let text = String::from_utf8(buf).unwrap();
    let status: u16 = text.split_whitespace().nth(1).unwrap().parse().unwrap();
    let body = text.split("\r\n\r\n").nth(1).unwrap_or("");
    (status, serde_json::from_str(body).unwrap_or(Value::Null))
}

#[tokio::test]
async fn api_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    let path = cube_file(&dir).await;

    let store = CubeStore::from_path(&path).unwrap();
    let app = cubism_serve::router(Arc::new(store));
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let base = listener.local_addr().unwrap().to_string();
    tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });

    // meta
    let (status, meta) = get(&base, "/api/meta").await;
    assert_eq!(status, 200);
    assert_eq!(meta["dimensions"], serde_json::json!(["outcome", "tool"]));
    assert_eq!(meta["has_global"], true);
    assert_eq!(meta["sketches"][0]["measure"], "sessions");
    assert_eq!(meta["sketches"][0]["kind"], "kmv");

    // slice: tools sorted by tokens desc -> Read (90) then Bash (60)
    let (status, cells) = get(&base, "/api/cells?dim=tool&sort=tokens").await;
    assert_eq!(status, 200);
    assert_eq!(cells["cells"][0]["label"], "Read");
    assert_eq!(cells["cells"][0]["measures"]["tokens"], 90);
    assert_eq!(cells["cells"][1]["measures"]["tokens"], 60);

    // one cell with decoded sketches: 3 distinct sessions in /G (under-full
    // KMV = exact), top_k items present
    let (status, g) = get(&base, "/api/cell?xunit=/G").await;
    assert_eq!(status, 200);
    assert_eq!(g["sketches"]["sessions"]["estimate"], 3.0);
    assert_eq!(g["sketches"]["top_sessions"]["items"][0][0], "s2");

    // set ops: sessions(Bash) = {s1,s2}, sessions(error) = {s1,s3};
    // exact while under-full: |A∩B| = 1, |A∪B| = 3
    let (status, ops) =
        get(&base, "/api/setops?a=/tool/tool=Bash&b=/outcome/outcome=error").await;
    assert_eq!(status, 200);
    assert_eq!(ops["measure"], "sessions");
    assert_eq!(ops["a"]["estimate"], 2.0);
    assert_eq!(ops["intersection"], 1.0);
    assert_eq!(ops["union"], 3.0);
    // lift = (1 * 3) / (2 * 2)
    assert_eq!(ops["lift"], 0.75);

    // errors are JSON with useful messages
    let (status, err) = get(&base, "/api/cell?xunit=/nope/nope=x").await;
    assert_eq!(status, 404);
    assert!(err["error"].as_str().unwrap().contains("no cell"));
    let (status, err) = get(&base, "/api/cells?dim=nope").await;
    assert_eq!(status, 404);
    assert!(err["error"].as_str().unwrap().contains("unknown dimension"));

    // the dashboard ships
    let (status, _) = get(&base, "/").await;
    assert_eq!(status, 200);
}
