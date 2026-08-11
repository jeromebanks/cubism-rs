package cubism.bench

import com.dynatrace.hash4j.hashing.Hashing
import org.scalatest.funsuite.AnyFunSuite

import java.nio.charset.StandardCharsets.UTF_8

/** Phase 0 hard gate: does hash4j's XXH3-64(seed 0) reproduce Rust's
  * `xxhash_rust::xxh3::xxh3_64` bit-for-bit? Nothing past this test may be
  * built on until it passes -- see docs/TIMESERIES_PHASE_0B_HARNESS.md
  * remaining-work item 1 and ~/.claude/plans/flickering-popping-harp.md.
  *
  * Expected values below are cross-validated two ways, not just transcribed
  * from one source:
  *   1. Decoded from cubism-core's own pinned golden KMV fixture
  *      (crates/cubism-core/src/sketch/kmv.rs, `golden_format_v1`): k=8,
  *      insert "alpha"/"beta"/"gamma", serialized hex
  *      4b4d56010800000003000000f6299d6fbff7700041f6df977ffffa2
  *      85aab25f6b50369be decodes to three little-endian u64 hashes:
  *      "gamma"=0x0070f7bf6f9d29f6, "beta"=0x28faff7f97dff641,
  *      "alpha"=0xbe6903b5f625ab5a.
  *   2. Independently recomputed via Python's `xxhash` package (a C
  *      extension wrapping the reference xxHash implementation, not the
  *      Rust port) -- `xxhash.xxh3_64(s.encode("utf-8"), seed=0)` -- which
  *      reproduced all three values exactly and additionally supplied the
  *      width-representative fixtures below (no other oracle exists for
  *      those specific strings).
  */
class Xxh3FidelitySpec extends AnyFunSuite {

  private val hasher = Hashing.xxh3_64() // seed 0 by construction

  private def hashOf(s: String): Long = hasher.hashBytesToLong(s.getBytes(UTF_8))

  test("golden KMV fixture strings hash exactly (short path, <=16 bytes)") {
    assert(hashOf("alpha") == 0xbe6903b5f625ab5aL)
    assert(hashOf("beta") == 0x28faff7f97dff641L)
    assert(hashOf("gamma") == 0x0070f7bf6f9d29f6L)
  }

  test("this benchmark's actual string widths hash exactly") {
    // device_%d (8 bytes), region_%05d (12 bytes), entity_%06d (13 bytes)
    // -- crates/cubism-timeseries-bench/src/lib.rs's formatted dimension
    // values. Still all inside XXH3's <=16-byte short path like the golden
    // fixture above, since nothing in this benchmark's data exceeds 16
    // bytes -- this does NOT prove XXH3's medium/long code paths.
    assert(hashOf("device_0") == 0xe71e48aa3c86ae7cL)
    assert(hashOf("region_00000") == 0x63c05a63e190f935L)
    assert(hashOf("entity_000000") == 0x6ee38befcd0740b9L)
  }

  test("unsigned ordering of the golden hashes matches storage order") {
    // KmvSketch stores hashes ascending UNSIGNED. "alpha"'s hash has its
    // high bit set (0xbe... as the top byte), so a naive signed Long sort
    // would misorder it first instead of last -- this is the trap
    // documented in the implementation plan's Phase 2. java.lang.Long has
    // no unsigned type, so every comparison must go through
    // Long.compareUnsigned explicitly.
    val gamma = hashOf("gamma")
    val beta = hashOf("beta")
    val alpha = hashOf("alpha")

    assert(java.lang.Long.compareUnsigned(gamma, beta) < 0)
    assert(java.lang.Long.compareUnsigned(beta, alpha) < 0)

    // The naive signed comparison gives the WRONG answer for the last pair
    // (alpha's hash is negative as a signed Long) -- asserted here as a
    // documented trap, not a bug to fix in this test.
    assert(beta > alpha, "sanity check: signed comparison misorders alpha before beta, as expected")
  }
}
