package cubism.bench

import org.scalatest.funsuite.AnyFunSuite

/** Phase 2 acceptance criterion: reproduce cubism-core's own
  * `golden_format_v1` test (crates/cubism-core/src/sketch/kmv.rs) exactly.
  * k=8, insert "alpha"/"beta"/"gamma", serialize, compare hex.
  */
class KmvGoldenSpec extends AnyFunSuite {

  private def hexOf(bytes: Array[Byte]): String =
    bytes.map(b => f"${b & 0xff}%02x").mkString

  test("golden_format_v1: k=8, insert alpha/beta/gamma, exact byte match") {
    val sketch = new Kmv(8)
    for (s <- Seq("alpha", "beta", "gamma")) sketch.insertString(s)

    val expected = "4b4d56010800000003000000f6299d6fbff7700041f6df977ffffa285aab25f6b50369be"
    assert(hexOf(sketch.toBytes) == expected)
  }

  test("insert is idempotent for duplicate values") {
    val sketch = new Kmv(8)
    sketch.insertString("alpha")
    sketch.insertString("alpha")
    assert(sketch.len == 1)
  }

  test("truncates to k, keeping the k smallest unsigned hashes (cross-checked against fromHashes)") {
    val items = (0 until 20).map(i => s"item$i")
    val sketch = new Kmv(8)
    items.foreach(sketch.insertString)
    assert(sketch.len == 8)

    // Independent computation path: bulk sort/dedup/truncate over ALL 20
    // hashes should land on exactly the same 8 survivors as one-at-a-time
    // insertString calls -- this is both a truncation-correctness check
    // and an insertString-vs-fromHashes cross-check in one assertion.
    val allHashes = items.map(Kmv.hashString)
    val expected = Kmv.fromHashes(8, allHashes.iterator)
    assert(sketch.toBytes.sameElements(expected.toBytes))
  }

  test("merge is commutative and caps at min(k_a, k_b)") {
    val a = new Kmv(16)
    for (s <- Seq("alpha", "beta", "gamma", "delta")) a.insertString(s)
    val b = new Kmv(8)
    for (s <- Seq("beta", "gamma", "epsilon")) b.insertString(s)

    val ab = a.merge(b)
    val ba = b.merge(a)
    assert(ab.k == 8) // min(16, 8)
    assert(ab.toBytes.sameElements(ba.toBytes))
    assert(ab.len == 5) // alpha, beta, gamma, delta, epsilon (beta/gamma deduped)
  }

  test("merge is idempotent (self-merge)") {
    val a = new Kmv(8)
    for (s <- Seq("alpha", "beta", "gamma")) a.insertString(s)
    val merged = a.merge(a)
    assert(merged.toBytes.sameElements(a.toBytes))
  }

  test("fromHashes matches equivalent one-at-a-time insertString calls") {
    val bulk = Kmv.fromHashes(8, Seq("alpha", "beta", "gamma").iterator.map(Kmv.hashString))
    val incremental = new Kmv(8)
    for (s <- Seq("alpha", "beta", "gamma")) incremental.insertString(s)
    assert(bulk.toBytes.sameElements(incremental.toBytes))
  }
}
