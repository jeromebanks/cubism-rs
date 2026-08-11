// Scala/Spark benchmark adapter for cubism Phase 0B remaining-work item 1
// ("the local Spark adapter"). See docs/TIMESERIES_PHASE_0B_HARNESS.md and
// ~/.claude/plans/flickering-popping-harp.md for the full design.
//
// Not part of the Cargo workspace -- this is a standalone sbt project.

ThisBuild / scalaVersion := "2.13.18" // matches Spark 4.2.0's bundled Scala
ThisBuild / version := "0.1.0"

lazy val root = (project in file("."))
  .settings(
    name := "cubism-spark-adapter",
    libraryDependencies ++= Seq(
      // Spark itself is supplied by `spark-submit` at runtime -- "provided"
      // keeps the built jar thin and avoids bundling a second Spark copy.
      "org.apache.spark" %% "spark-core" % "4.2.0" % "provided",
      "org.apache.spark" %% "spark-sql" % "4.2.0" % "provided",
      // XXH3-64(seed 0): must byte-match cubism-core's xxhash_rust::xxh3::xxh3_64.
      "com.dynatrace.hash4j" % "hash4j" % "0.30.0",
      // BLAKE3-256: must byte-match cubism-core's blake3 crate. Direct port
      // of the Rust reference implementation, not a from-scratch rewrite.
      "io.github.rctcwyvrn" % "blake3" % "1.3",
      "org.scalatest" %% "scalatest" % "3.2.19" % Test
    )
  )
