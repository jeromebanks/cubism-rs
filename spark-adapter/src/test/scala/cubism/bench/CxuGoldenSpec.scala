package cubism.bench

import org.scalatest.funsuite.AnyFunSuite

/** Phase 3 acceptance criterion. Two levels of check:
  *
  *   1. Reproduce cubism-core's own general-purpose golden fixture
  *      (crates/cubism-core/src/encoding.rs, test
  *      `canonical_golden_bytes_and_content_id_v1`) exactly -- both the
  *      encoded bytes AND the BLAKE3 content ID. This fixture uses
  *      dim="geo", attribute name="country" (name != dim, unlike this
  *      benchmark's flat dims), so it validates the general encoder logic
  *      independent of the BENCH_SPEC-specific 4-cell builders.
  *   2. Structural/self-consistency checks on the actual 4 fixed-lattice
  *      builders this benchmark uses (Cxu.allCells).
  */
class CxuGoldenSpec extends AnyFunSuite {

  private def hexOf(bytes: Array[Byte]): String =
    bytes.map(b => f"${b & 0xff}%02x").mkString

  test("golden fixture: geo/country=CZ encodes and hashes exactly") {
    val ypath = CxuYPath("geo", Seq(("country", "CZ")))
    val encoded = Cxu.encode(Seq(ypath))

    assert(
      hexOf(encoded) ==
        "43585501000000010000000367656f0000000100000007636f756e7472790600000002435a"
    )
    assert(
      Cxu.contentIdHex(encoded) ==
        "bed9c102177b3a386d76f139182452ae0f343c8760bf439d66aca37ad547fc4a"
    )
  }

  test("global cell encodes to ypath_count=0 with no trailing bytes") {
    val encoded = Cxu.globalCell()
    // "CXU" + version(1) + ypath_count:u32-BE=0 -- exactly 8 bytes, nothing more
    assert(hexOf(encoded) == "4358550100000000")
    assert(encoded.length == 8)
  }

  test("all 4 fixed-lattice cells for one row are distinct") {
    val cells = Cxu.allCells("device_0", "region_00000")
    assert(cells.length == 4)
    val hexes = cells.map(hexOf)
    assert(hexes.distinct.length == 4, "all 4 cells must encode to distinct bytes")
  }

  test("device+region cell's dim order is device-then-region (already dim-sorted)") {
    // "device" < "region" alphabetically -- deviceRegionCell hardcodes this
    // order directly rather than sorting at runtime. Confirm by decoding
    // the first ypath's dim name at byte offset 8 (after magic+version+
    // ypath_count) and checking it reads "device", not "region".
    val combined = Cxu.deviceRegionCell("device_0", "region_00000")
    val dimLen = ((combined(8) & 0xff) << 24) | ((combined(9) & 0xff) << 16) |
      ((combined(10) & 0xff) << 8) | (combined(11) & 0xff)
    assert(dimLen == "device".length)
    val dimName = new String(combined, 12, dimLen, java.nio.charset.StandardCharsets.UTF_8)
    assert(dimName == "device")
  }

  test("content IDs are 32 bytes and deterministic across repeated calls") {
    val a = Cxu.contentId(Cxu.deviceRegionCell("device_0", "region_00000"))
    val b = Cxu.contentId(Cxu.deviceRegionCell("device_0", "region_00000"))
    assert(a.length == 32)
    assert(a.sameElements(b))
  }
}
