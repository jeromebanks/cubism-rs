package cubism.bench

import com.dynatrace.hash4j.hashing.Hashing

import java.nio.charset.StandardCharsets.UTF_8
import java.nio.{ByteBuffer, ByteOrder}
import java.util.{Comparator, TreeSet => JTreeSet}
import scala.collection.mutable

/** Byte-identical Scala port of cubism-core's KMV sketch
  * (crates/cubism-core/src/sketch/kmv.rs). See
  * ~/.claude/plans/flickering-popping-harp.md Phase 2 for the full
  * contract; this file implements it verbatim -- do not "improve" the
  * algorithm without re-deriving byte-identity from the Rust source.
  *
  * Wire format, all integers little-endian:
  *   b"KMV" + version:u8=1 + k:u32-LE + n:u32-LE + n x u64-LE hashes
  *   (ascending sorted, distinct). Total length = 12 + 8n bytes exactly.
  *
  * Hashes sort ascending UNSIGNED -- the JVM has no unsigned 64-bit
  * primitive, so every comparison here goes through
  * java.lang.Long.compareUnsigned explicitly. See Xxh3FidelitySpec's
  * "unsigned ordering" test for why this matters concretely.
  */
final class Kmv private (val k: Int, private val hashes: mutable.ArrayBuffer[Long]) {

  def this(k: Int) = this(k, mutable.ArrayBuffer.empty[Long])

  require(k >= 8, s"k must be >= 8, got $k")

  def len: Int = hashes.length

  /** Binary-search insert, ascending unsigned; keep only if the insertion
    * position is < k, then truncate back to k. Mirrors kmv.rs's `insert`.
    */
  def insertHash(h: Long): Unit = {
    val pos = unsignedInsertionPoint(h)
    if (pos < hashes.length && hashes(pos) == h) return // already present, no-op
    if (pos < k) {
      hashes.insert(pos, h)
      if (hashes.length > k) hashes.remove(hashes.length - 1)
    }
  }

  def insert(bytes: Array[Byte]): Unit = insertHash(Kmv.hashValue(bytes))

  def insertString(s: String): Unit = insertHash(Kmv.hashString(s))

  /** First index whose stored value is >= h, unsigned comparison. */
  private def unsignedInsertionPoint(h: Long): Int = {
    var lo = 0
    var hi = hashes.length
    while (lo < hi) {
      val mid = (lo + hi) >>> 1
      if (java.lang.Long.compareUnsigned(hashes(mid), h) < 0) lo = mid + 1 else hi = mid
    }
    lo
  }

  /** Two-pointer sorted merge, capped at min(k_a, k_b), dedup on equal
    * hash. Mirrors kmv.rs's `merge`. Associative/commutative/idempotent --
    * this is the operation Spark's Aggregator combine step relies on.
    */
  def merge(other: Kmv): Kmv = {
    val k2 = math.min(this.k, other.k)
    val result = mutable.ArrayBuffer.empty[Long]
    var i = 0
    var j = 0
    val a = this.hashes
    val b = other.hashes
    while (result.length < k2 && (i < a.length || j < b.length)) {
      if (i < a.length && j < b.length) {
        val cmp = java.lang.Long.compareUnsigned(a(i), b(j))
        if (cmp < 0) { result += a(i); i += 1 }
        else if (cmp > 0) { result += b(j); j += 1 }
        else { result += a(i); i += 1; j += 1 } // equal hash: dedup
      } else if (i < a.length) { result += a(i); i += 1 }
      else { result += b(j); j += 1 }
    }
    new Kmv(k2, result)
  }

  /** Read-only snapshot of the current sorted-unsigned-distinct hash list,
    * for handing off to a serializable buffer type (e.g. Spark's
    * Aggregator BUF, which needs a plain Array[Long] it can encode). The
    * returned array is already sorted/deduped/truncated -- see
    * `Kmv.fromSortedHashes` for the paired reconstruction.
    */
  def hashesArray: Array[Long] = hashes.toArray

  def toBytes: Array[Byte] = {
    val n = hashes.length
    val buf = ByteBuffer.allocate(Kmv.HeaderLen + 8 * n).order(ByteOrder.LITTLE_ENDIAN)
    buf.put(Kmv.Magic)
    buf.put(Kmv.Version)
    buf.putInt(k)
    buf.putInt(n)
    var idx = 0
    while (idx < n) {
      buf.putLong(hashes(idx))
      idx += 1
    }
    buf.array()
  }
}

object Kmv {
  private val Magic: Array[Byte] = Array('K'.toByte, 'M'.toByte, 'V'.toByte)
  private val Version: Byte = 1
  private val HeaderLen = 3 + 1 + 4 + 4 // magic + version + k + n

  /** k fixed at 1024 for this benchmark (BENCH_SPEC / DEFAULT_SKETCH_SIZE
    * in the Rust harness); the type itself supports any k >= 8.
    */
  val DefaultK: Int = 1024

  private val hasher = Hashing.xxh3_64() // seed 0 by construction

  def hashValue(bytes: Array[Byte]): Long = hasher.hashBytesToLong(bytes)

  def hashString(s: String): Long = hashValue(s.getBytes(UTF_8))

  /** Bulk build: sort + dedup (unsigned) + truncate to k. Mirrors kmv.rs's
    * `from_hashes` -- the path Spark's own partial-aggregate combine
    * exercises when building a sketch from a whole group's worth of
    * distinct hashes at once rather than one insertHash call at a time.
    */
  /** Reconstructs a Kmv from a hash array that is ALREADY sorted-unsigned,
    * deduplicated, and truncated to k -- i.e. exactly what `hashesArray`
    * produces. Paired with that accessor to round-trip through a
    * serializable buffer (Spark's Aggregator BUF type) without re-sorting.
    * Passing an array that doesn't already satisfy this invariant produces
    * a corrupt sketch -- this is an internal reconstruction helper, not a
    * general-purpose constructor; use `fromHashes` for arbitrary input.
    */
  def fromSortedHashes(k: Int, sortedHashes: Array[Long]): Kmv =
    new Kmv(k, mutable.ArrayBuffer.from(sortedHashes))

  def fromHashes(k: Int, hs: IterableOnce[Long]): Kmv = {
    val unsignedCmp: Comparator[java.lang.Long] = (a, b) => java.lang.Long.compareUnsigned(a, b)
    val sorted = new JTreeSet[java.lang.Long](unsignedCmp)
    hs.iterator.foreach(h => sorted.add(h))
    val buf = mutable.ArrayBuffer.empty[Long]
    val it = sorted.iterator()
    while (it.hasNext && buf.length < k) buf += it.next()
    new Kmv(k, buf)
  }
}
