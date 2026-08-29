package cubism.bench

import org.apache.spark.sql.expressions.Aggregator
import org.apache.spark.sql.{Encoder, Encoders}

/** Serializable buffer for KmvAggregator. Spark's `Aggregator[IN,BUF,OUT]`
  * requires an `Encoder[BUF]`; `Kmv` itself holds a private mutable
  * ArrayBuffer and isn't a case class, so it can't be encoded directly.
  * `hashes` is always already sorted-unsigned/deduped/truncated to `k` --
  * see `Kmv.hashesArray`/`Kmv.fromSortedHashes`, which this wraps.
  */
final case class KmvBuf(k: Int, hashes: Array[Long])

/** Spark `Aggregator` wrapping `Kmv`, structurally the same state/merge/
  * evaluate shape as Rust's own `KmvAccumulator`/`SketchGroupsAccumulator`
  * (crates/cubism-datafusion/src/udaf.rs) -- state and the final output
  * are both the identical serialized KMV v1 blob, matching the Rust
  * UDAF's own "state == evaluate == blob" design.
  */
final class KmvAggregator(k: Int = Kmv.DefaultK) extends Aggregator[String, KmvBuf, Array[Byte]] {

  override def zero: KmvBuf = KmvBuf(k, Array.emptyLongArray)

  override def reduce(buf: KmvBuf, entityId: String): KmvBuf = {
    val sketch = Kmv.fromSortedHashes(buf.k, buf.hashes)
    sketch.insertString(entityId)
    KmvBuf(sketch.k, sketch.hashesArray)
  }

  override def merge(b1: KmvBuf, b2: KmvBuf): KmvBuf = {
    val merged = Kmv.fromSortedHashes(b1.k, b1.hashes).merge(Kmv.fromSortedHashes(b2.k, b2.hashes))
    KmvBuf(merged.k, merged.hashesArray)
  }

  override def finish(buf: KmvBuf): Array[Byte] =
    Kmv.fromSortedHashes(buf.k, buf.hashes).toBytes

  override def bufferEncoder: Encoder[KmvBuf] = Encoders.product[KmvBuf]

  override def outputEncoder: Encoder[Array[Byte]] = Encoders.BINARY
}
