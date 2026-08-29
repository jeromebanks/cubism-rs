package cubism.bench

import io.github.rctcwyvrn.blake3.{Blake3 => Blake3Hasher}
import org.apache.spark.sql.functions._
import org.apache.spark.sql.SparkSession

import java.io.{File, PrintWriter}
import java.nio.charset.StandardCharsets.UTF_8
import java.nio.file.{Files, Paths}
import java.nio.{ByteBuffer, ByteOrder}

// Top-level (not method-local) case classes: Spark's Encoder derivation via
// `spark.implicits._` needs a proper TypeTag, which method-local case
// classes don't reliably get (a known Scala/Spark gotcha -- "Unable to
// find encoder for type" even though the class is a perfectly ordinary
// case class).
private final case class SourceRow(bucketStartUs: Long, region: String, device: String, amount: Long, entityId: String)
private final case class ExplodedRow(
    bucketStartUs: Long,
    contentId: Array[Byte],
    canonicalXunit: Array[Byte],
    amount: Long,
    entityId: String
)

/** Phase 0B remaining-work item 1: the local Spark adapter. Reads the same
  * shared source Parquet the Rust harness consumes, runs an equivalent
  * aggregation, and computes the same BLAKE3 semantic digest -- see
  * ~/.claude/plans/flickering-popping-harp.md Phase 4/4b and
  * docs/TIMESERIES_PHASE_0B_HARNESS.md.
  *
  * Minimal-scope adapter: does not write any of the 4 candidate Parquet
  * layouts the Rust harness writes -- see the plan's "explicitly out of
  * scope" section.
  */
object SparkAggregate {

  private val SemanticDigestDomain: Array[Byte] =
    "cubism-phase0b-semantic-v1".getBytes(UTF_8)

  final case class Args(
      input: String,
      outputDir: String,
      partitions: Int = 4,
      memoryLimitMb: Option[Int] = None
  )

  private def parseArgs(argv: Array[String]): Args = {
    var input: Option[String] = None
    var outputDir: Option[String] = None
    var partitions = 4
    var memoryLimitMb: Option[Int] = None
    var i = 0
    while (i < argv.length) {
      argv(i) match {
        case "--input"           => input = Some(argv(i + 1)); i += 2
        case "--output-dir"      => outputDir = Some(argv(i + 1)); i += 2
        case "--partitions"      => partitions = argv(i + 1).toInt; i += 2
        case "--memory-limit-mb" => memoryLimitMb = Some(argv(i + 1).toInt); i += 2
        case other                => throw new IllegalArgumentException(s"unknown flag: $other")
      }
    }
    Args(
      input.getOrElse(throw new IllegalArgumentException("--input is required")),
      outputDir.getOrElse(throw new IllegalArgumentException("--output-dir is required")),
      partitions,
      memoryLimitMb
    )
  }

  private def writeLongBE(hasher: Blake3Hasher, value: Long): Unit = {
    val buf = ByteBuffer.allocate(8).order(ByteOrder.BIG_ENDIAN)
    buf.putLong(value)
    hasher.update(buf.array())
  }

  /** Unsigned lexicographic byte compare, matching Rust's `[u8;32]` /
    * `BTreeMap` ordering. Spark's own `BinaryType` Catalyst ordering
    * should already sort this way (masks each byte with & 0xff), but this
    * function is used by the monotonicity check below to verify that
    * assumption at run time rather than trust it silently.
    */
  private def compareUnsignedBytes(a: Array[Byte], b: Array[Byte]): Int = {
    val len = math.min(a.length, b.length)
    var i = 0
    while (i < len) {
      val cmp = (a(i) & 0xff) - (b(i) & 0xff)
      if (cmp != 0) return cmp
      i += 1
    }
    a.length - b.length
  }

  def main(argv: Array[String]): Unit = {
    val startWall = System.nanoTime()
    val args = parseArgs(argv)

    val builder = SparkSession.builder().appName("cubism-spark-aggregate")
    // Only default the master if spark-submit didn't already set one --
    // lets `--master` on the spark-submit command line win. Falling back
    // to local[partitions] here is purely a convenience for `sbt run`
    // during development.
    if (System.getProperty("spark.master") == null) {
      builder.master(s"local[${args.partitions}]")
    }
    val spark = builder
      .config("spark.sql.shuffle.partitions", args.partitions.toString)
      .getOrCreate()
    spark.sparkContext.setLogLevel("WARN")
    import spark.implicits._

    val startupMs = (System.nanoTime() - startWall) / 1000000L

    val inputBytes = new File(args.input).length()

    val source = spark.read.parquet(args.input)
    // unix_micros isn't exposed as a `functions.unix_micros` Scala method
    // in this Spark version (only exists as the underlying Catalyst
    // expression / SQL function), so it's invoked via `expr`. Verified
    // against Rust's own BASE_BUCKET_START_US constant
    // (crates/cubism-timeseries-bench/src/lib.rs) before relying on it here
    // -- CAST(... AS BIGINT) on a TimestampType column silently returns
    // epoch SECONDS instead, which unix_micros avoids.
    val withMicros = source
      .withColumn("bucket_start_us", expr("unix_micros(bucket_start)"))
      .select(
        col("bucket_start_us").as("bucketStartUs"),
        col("region"),
        col("device"),
        col("amount"),
        col("entity_id").as("entityId")
      )

    // --- explode each row into its 4 fixed XUnit cells --------------------
    val exploded = withMicros
      .as[SourceRow]
      .flatMap { r =>
        Cxu.allCells(r.device, r.region).map { cxuBytes =>
          ExplodedRow(r.bucketStartUs, Cxu.contentId(cxuBytes), cxuBytes, r.amount, r.entityId)
        }
      }

    // groupBy on a BinaryType `contentId` column -- Catalyst's value-based
    // equality/grouping, NOT an RDD keyed on a raw Array[Byte] (which
    // would silently group by reference identity instead of content).
    val kmvUdaf = udaf(new KmvAggregator(Kmv.DefaultK))

    val aggStart = System.nanoTime()
    val aggregated = exploded
      .groupBy(col("bucketStartUs"), col("contentId"))
      .agg(
        first(col("canonicalXunit")).as("canonicalXunit"),
        sum(col("amount")).cast("long").as("sumState"),
        count(lit(1)).as("countState"),
        kmvUdaf(col("entityId")).as("kmvState")
      )
      .cache()
    // Spark is lazy -- force materialization here so aggregateMs actually
    // times the groupBy/shuffle stage instead of ~0ms of DataFrame
    // construction. The cache means the digest fold below reuses this
    // result instead of recomputing the whole aggregation from scratch.
    val cellCount = aggregated.count()
    val aggregateMs = (System.nanoTime() - aggStart) / 1000000L

    // --- driver-side streaming digest fold, NOT collect() ------------------
    // Total cell counts at benchmark scale (15.1M @ 10M rows, 28.8M @ 25M
    // rows per docs/TIMESERIES_PHASE_0B_HARNESS.md) are far too large to
    // collect() to the driver. orderBy's range partitioning guarantees
    // partition-index order equals global row order, so toLocalIterator
    // (one partition at a time) visits rows in the required strict
    // ascending order without materializing the whole result at once.
    val digestStart = System.nanoTime()
    val sorted = aggregated.orderBy(col("bucketStartUs"), col("contentId"))

    val hasher = Blake3Hasher.newInstance()
    hasher.update(SemanticDigestDomain)

    var previous: Option[(Long, Array[Byte])] = None
    val it = sorted.toLocalIterator()
    while (it.hasNext) {
      val row = it.next()
      val bucketUs = row.getAs[Long]("bucketStartUs")
      val contentId = row.getAs[Array[Byte]]("contentId")
      val canonicalXunit = row.getAs[Array[Byte]]("canonicalXunit")
      val sumState = row.getAs[Long]("sumState")
      val countState = row.getAs[Long]("countState")
      val kmvState = row.getAs[Array[Byte]]("kmvState")

      // Catches an orderBy/AQE ordering bug as a loud failure instead of a
      // silently wrong digest -- mirrors Rust's own check_monotonic.
      previous.foreach { case (prevBucket, prevContentId) =>
        val cmp =
          if (bucketUs != prevBucket) bucketUs.compareTo(prevBucket)
          else compareUnsignedBytes(contentId, prevContentId)
        require(cmp > 0, s"monotonicity violated at bucket=$bucketUs (prev bucket=$prevBucket) -- orderBy/AQE bug")
      }
      previous = Some((bucketUs, contentId))

      // Mirrors digest_update (crates/cubism-timeseries-bench/src/lib.rs)
      // field-for-field: bucket i64 BE, 32 raw content-id bytes, u64-BE
      // length-prefixed canonical_xunit, sum i64 BE, count u64 BE, u64-BE
      // length-prefixed kmv bytes.
      writeLongBE(hasher, bucketUs)
      hasher.update(contentId)
      writeLongBE(hasher, canonicalXunit.length.toLong)
      hasher.update(canonicalXunit)
      writeLongBE(hasher, sumState)
      writeLongBE(hasher, countState)
      writeLongBE(hasher, kmvState.length.toLong)
      hasher.update(kmvState)
    }
    val semanticDigest = hasher.hexdigest()
    val digestMs = (System.nanoTime() - digestStart) / 1000000L

    val totalMs = (System.nanoTime() - startWall) / 1000000L

    Files.createDirectories(Paths.get(args.outputDir))
    val metrics = RunMetrics(
      harnessSchemaVersion = 3,
      engine = "spark-scala",
      engineVersion = spark.version,
      input = args.input,
      inputBytes = inputBytes,
      outputDir = args.outputDir,
      sparkMaster = spark.sparkContext.master,
      targetPartitions = args.partitions,
      memoryLimitNote = args.memoryLimitMb
        .map(mb => s"informational only, not enforced by this process -- pass --driver-memory/--executor-memory to spark-submit for ${mb}m")
        .getOrElse("not specified"),
      aggregateMs = aggregateMs,
      digestMs = digestMs,
      startupMs = startupMs,
      totalMs = totalMs,
      cellCount = cellCount,
      semanticDigestBlake3 = semanticDigest,
      sparkConfig = Map(
        "spark.sql.adaptive.enabled" -> spark.conf.get("spark.sql.adaptive.enabled", "true"),
        "spark.sql.shuffle.partitions" -> spark.conf.get("spark.sql.shuffle.partitions")
      )
    )

    val runJsonPath = Paths.get(args.outputDir, "run.json")
    val writer = new PrintWriter(runJsonPath.toFile, "UTF-8")
    try writer.write(metrics.toJson)
    finally writer.close()

    println(metrics.toJson)
    spark.stop()
  }
}
