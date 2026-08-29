package cubism.bench

import io.github.rctcwyvrn.blake3.Blake3
import org.scalatest.funsuite.AnyFunSuite

/** Phase 0 hard gate, second hash function: does the `io.github.rctcwyvrn`
  * BLAKE3 Java port reproduce the official BLAKE3 test vectors, and by
  * extension Rust's `blake3` crate (`crates/cubism-core/src/encoding.rs`,
  * `canonical_xunit_content_id`)? Verified against BLAKE3's own published
  * test vectors (github.com/BLAKE3-team/BLAKE3/test_vectors/test_vectors.json,
  * fetched and cross-checked programmatically, not hand-transcribed), plus
  * an independent cross-check against the `blake3` PyPI package (a
  * different implementation from both the official vectors and the Rust
  * crate under test).
  *
  * Vectors up to length 63 exercise only a single BLAKE3 chunk (chunk size
  * = 1024 bytes) via the leaf-hashing path. Length 1024 is exactly one full
  * chunk (root-chunk-finalized, still no tree merge); length 1025 is the
  * smallest input requiring two chunks and genuinely exercises the
  * parent/tree-merge hashing path -- a leaf-only check would miss a
  * merge-logic bug entirely.
  */
class Blake3FidelitySpec extends AnyFunSuite {

  // Per the official test vectors' _comment: input is a repeating sequence
  // of 251 bytes, 0,1,2,...,249,250,0,1,...
  private def testInput(len: Int): Array[Byte] =
    Array.tabulate(len)(i => (i % 251).toByte)

  private def hexOf(bytes: Array[Byte]): String =
    bytes.map(b => f"${b & 0xff}%02x").mkString

  private def blake3Hex(input: Array[Byte]): String = {
    val h = Blake3.newInstance()
    h.update(input)
    h.hexdigest() // default 32-byte output
  }

  test("empty input matches the official BLAKE3(input_len=0) test vector") {
    assert(blake3Hex(testInput(0)) == "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262")
  }

  test("official test vector, input_len=1 (single chunk, leaf path)") {
    assert(blake3Hex(testInput(1)) == "2d3adedff11b61f14c886e35afa036736dcd87a74d27b5c1510225d0f592e213")
  }

  test("official test vector, input_len=63 (single chunk, leaf path)") {
    assert(blake3Hex(testInput(63)) == "e9bc37a594daad83be9470df7f7b3798297c3d834ce80ba85d6e207627b7db7b")
  }

  test("official test vector, input_len=1024 (exact one-chunk boundary, no merge yet)") {
    assert(blake3Hex(testInput(1024)) == "42214739f095a406f3fc83deb889744ac00df831c10daa55189b5d121c855af7")
  }

  test("official test vector, input_len=1025 (two chunks, exercises tree-merge path)") {
    assert(blake3Hex(testInput(1025)) == "d00278ae47eb27b34faecf67b4fe263f82d5412916c1ffd97c8cb7fb814b8444")
  }

  test("plain 32 raw bytes round-trip matches hex digest (digest() API, not just hexdigest())") {
    val h = Blake3.newInstance()
    h.update("cubism".getBytes(java.nio.charset.StandardCharsets.UTF_8))
    assert(hexOf(h.digest()) == h.hexdigest())
    assert(h.digest().length == 32)
  }
}
