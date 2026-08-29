package cubism.bench

import io.github.rctcwyvrn.blake3.Blake3

import java.io.{ByteArrayOutputStream, DataOutputStream}
import java.nio.charset.StandardCharsets.UTF_8

/** One YPath: a dimension name plus its ordered attribute name/value
  * pairs. Values are always CanonicalValue::String for this benchmark --
  * BENCH_SPEC's device/region dimensions are flat strings, so the fuller
  * CanonicalValue tag space (Null/Bool/I64/U64/F64/Bytes) from
  * cubism-core's encoding.rs is intentionally not implemented here; only
  * tag 6 (String) is ever emitted.
  */
final case class CxuYPath(dim: String, attributes: Seq[(String, String)])

/** Byte-identical Scala port of cubism-core's canonical XUnit ("CXU")
  * encoding (crates/cubism-core/src/encoding.rs, `encode_canonical_xunit`
  * + `canonical_xunit_content_id`). See
  * ~/.claude/plans/flickering-popping-harp.md Phase 3.
  *
  * Wire format, all integers BIG-endian (opposite of Kmv's little-endian
  * -- do not conflate the two):
  *   "CXU" + version:u8=1 + ypath_count:u32-BE
  *     for each ypath, sorted by dim name ascending:
  *       dim_name: u32-BE length + UTF-8 bytes
  *       attr_count:u32-BE
  *       for each attribute, original order:
  *         attr_name: u32-BE length + UTF-8 bytes
  *         value: 1-byte tag + payload (tag 6 = String: u32-BE len + UTF-8)
  *
  * Content ID = plain BLAKE3-256 of the encoded bytes, no domain
  * separation.
  *
  * Does NOT port the general XUnit/YPath/lattice/filter-rule engine --
  * BENCH_SPEC (crates/cubism-timeseries-bench/src/lib.rs) is fixed and
  * flat (dimensions: device, region; includeGlobal: true; neither
  * dimension has explicit `levels:`), so every row explodes into exactly
  * 4 fixed cells. This object hardcodes those 4 builders directly instead.
  */
object Cxu {
  private val Magic: Array[Byte] = Array('C'.toByte, 'X'.toByte, 'U'.toByte)
  private val Version: Int = 1
  private val StringTag: Int = 6

  private def writeLenPrefixed(dos: DataOutputStream, s: String): Unit = {
    val bytes = s.getBytes(UTF_8)
    dos.writeInt(bytes.length) // u32-BE (DataOutputStream.writeInt is always big-endian)
    dos.write(bytes)
  }

  /** ypaths must already be sorted by dim name ascending and contain no
    * duplicate dims -- this function does not sort or validate, matching
    * the "hardcode the fixed 4-variant lattice, don't port the general
    * engine" scoping decision. All 4 builders below satisfy this by
    * construction.
    */
  def encode(ypaths: Seq[CxuYPath]): Array[Byte] = {
    val baos = new ByteArrayOutputStream()
    val dos = new DataOutputStream(baos)
    dos.write(Magic)
    dos.writeByte(Version)
    dos.writeInt(ypaths.length)
    for (yp <- ypaths) {
      writeLenPrefixed(dos, yp.dim)
      dos.writeInt(yp.attributes.length)
      for ((name, value) <- yp.attributes) {
        writeLenPrefixed(dos, name)
        dos.writeByte(StringTag)
        writeLenPrefixed(dos, value)
      }
    }
    dos.flush()
    baos.toByteArray
  }

  def contentId(encoded: Array[Byte]): Array[Byte] = {
    val h = Blake3.newInstance()
    h.update(encoded)
    h.digest() // 32 bytes, default output length
  }

  def contentIdHex(encoded: Array[Byte]): String =
    contentId(encoded).map(b => f"${b & 0xff}%02x").mkString

  // --- BENCH_SPEC's fixed 4-cell lattice --------------------------------
  // dimensions: device, region (both flat -- attribute name == dim name,
  // per DimensionSpec::effective_levels() in crates/cubism-core/src/
  // spec.rs), includeGlobal: true. "device" < "region" alphabetically, so
  // device-then-region is already the canonical dim-sorted order.

  def globalCell(): Array[Byte] = encode(Seq.empty)

  def deviceCell(device: String): Array[Byte] =
    encode(Seq(CxuYPath("device", Seq(("device", device)))))

  def regionCell(region: String): Array[Byte] =
    encode(Seq(CxuYPath("region", Seq(("region", region)))))

  def deviceRegionCell(device: String, region: String): Array[Byte] =
    encode(
      Seq(
        CxuYPath("device", Seq(("device", device))),
        CxuYPath("region", Seq(("region", region)))
      )
    )

  /** All 4 fixed-lattice cells for one (device, region) row, matching
    * AGGREGATE_SQL's `cubism_xunit_keys(device, region)` explosion.
    */
  def allCells(device: String, region: String): Seq[Array[Byte]] =
    Seq(globalCell(), deviceCell(device), regionCell(region), deviceRegionCell(device, region))
}
