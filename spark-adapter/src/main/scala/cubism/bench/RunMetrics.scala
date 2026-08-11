package cubism.bench

/** run.json-equivalent for the Scala/Spark adapter, diffable against
  * Rust's `RustRunMetrics` (crates/cubism-timeseries-bench/src/lib.rs)
  * where the two overlap -- see
  * ~/.claude/plans/flickering-popping-harp.md Phase 4b for the field
  * mapping table and the explicit non-comparability caveats.
  *
  * No JSON library dependency -- these are a handful of scalar fields
  * plus one small string map, hand-serialized below rather than pulling
  * in circe/play-json/jackson for this.
  */
final case class RunMetrics(
    harnessSchemaVersion: Int,
    engine: String,
    engineVersion: String,
    input: String,
    inputBytes: Long,
    outputDir: String,
    sparkMaster: String,
    targetPartitions: Int,
    memoryLimitNote: String,
    aggregateMs: Long,
    digestMs: Long,
    startupMs: Long,
    totalMs: Long,
    cellCount: Long,
    semanticDigestBlake3: String,
    sparkConfig: Map[String, String]
) {

  def toJson: String = {
    val sb = new StringBuilder
    sb.append("{\n")
    def field(name: String, value: String, last: Boolean = false): Unit = {
      sb.append(s"""  "$name": $value""")
      sb.append(if (last) "\n" else ",\n")
    }
    def str(s: String): String = "\"" + RunMetrics.escape(s) + "\""

    field("harness_schema_version", harnessSchemaVersion.toString)
    field("engine", str(engine))
    field("engine_version", str(engineVersion))
    field("input", str(input))
    field("input_bytes", inputBytes.toString)
    field("output_dir", str(outputDir))
    field("spark_master", str(sparkMaster))
    field("target_partitions", targetPartitions.toString)
    field("memory_limit_note", str(memoryLimitNote))
    field("aggregate_ms", aggregateMs.toString)
    field("digest_ms", digestMs.toString)
    field("startup_ms", startupMs.toString)
    field("total_ms", totalMs.toString)
    field("cell_count", cellCount.toString)
    field("semantic_digest_blake3", str(semanticDigestBlake3))
    val configEntries = sparkConfig.toSeq
      .map { case (k, v) => s"""    ${str(k)}: ${str(v)}""" }
      .mkString(",\n")
    sb.append("""  "spark_config": {""")
    sb.append("\n")
    sb.append(configEntries)
    sb.append("\n  },\n")
    field(
      "comparability_caveat",
      str(
        "Rust's aggregate_ms includes writing all 4 candidate Parquet " +
          "layouts; this adapter writes none, so a raw aggregate_ms diff " +
          "overstates Spark's relative speed unless this is accounted for."
      ),
      last = true
    )
    sb.append("}\n")
    sb.toString()
  }
}

object RunMetrics {
  def escape(s: String): String =
    s.replace("\\", "\\\\").replace("\"", "\\\"").replace("\n", "\\n")
}
