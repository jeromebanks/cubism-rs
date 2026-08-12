#!/usr/bin/env python3
"""Phase 0B process-level matrix runner with an RSS safety watchdog.

Remaining-work item 3 from docs/TIMESERIES_PHASE_0B_HARNESS.md: run the
`cubism-timeseries-bench` release binary as a subprocess, warm-up + >=5
measured runs per config, recording peak RSS/CPU/wall time/startup and
per-run environment snapshots.

This is also the safety mechanism gating item 4's scale-up (see
docs/TIMESERIES_PHASE_0B_NOTEBOOK.md): the harness's own in-process
metrics cannot see true OS-level peak RSS and cannot stop itself before
the 16GB host runs out of memory. This script samples the child's RSS on
an interval and kills it before that happens, recording why.

Deliberately stdlib-only (no psutil, no extra deps) so it runs anywhere
this repo's Rust toolchain does.

Usage:
    python3 matrix_runner.py \\
        --bench-bin /tmp/cubism-target-phase0b/release/cubism-timeseries-bench \\
        --source-dir /tmp/phase0b/sources \\
        --output-root /tmp/phase0b/matrix \\
        --config rows=10000000,occupancy=sparse,partitions=4,memory-limit-mb=2048 \\
        --measured-runs 5 --rss-limit-mb 12000 --min-free-gb 10
"""

from __future__ import annotations

import argparse
import functools
import json
import os
import shutil
import subprocess
import sys
import time
from dataclasses import dataclass, field
from pathlib import Path

# Python fully-buffers stdout when it isn't a terminal (e.g. redirected to a
# background-task log file), so progress prints otherwise sit unflushed for
# minutes at a time -- discovered when tailing a live 25M-row run's log
# showed nothing despite the process being on its 5th of 6 stages. The
# watchdog's actual kill decision runs in-process and was never affected by
# this, but external monitoring of progress was blind without it.
print = functools.partial(print, flush=True)


def env_snapshot() -> dict:
    """Same commands as the manual notebook entries, so tool output and
    hand-written entries stay directly comparable."""

    def run(cmd: list[str]) -> str:
        try:
            return subprocess.run(
                cmd, capture_output=True, text=True, timeout=10
            ).stdout.strip()
        except Exception as exc:  # noqa: BLE001 - snapshot must not abort the run
            return f"<error: {exc}>"

    return {
        "timestamp": time.strftime("%Y-%m-%dT%H:%M:%S%z"),
        "memory_pressure": run(["memory_pressure", "-Q"]),
        "vm_stat": run(["vm_stat"]),
        "swapusage": run(["sysctl", "vm.swapusage"]),
        "loadavg": list(os.getloadavg()),
        "disk_root": run(["df", "-h", "/"]),
    }


def free_gb(path: Path) -> float | None:
    """None means the path's volume couldn't be statted (e.g. disconnected),
    not zero -- callers must not treat that the same as "full disk"."""
    try:
        usage = shutil.disk_usage(path)
        return usage.free / (1024**3)
    except OSError:
        return None


def mount_ready(path: Path, retries: int = 6, backoff_s: float = 5.0) -> bool:
    """Verify `path`'s volume actually accepts writes right now, retrying
    with backoff. External USB drives can drop and remount mid-run (see
    docs/TIMESERIES_PHASE_0B_NOTEBOOK.md Entry 5) -- a dropped mount fails
    every syscall against it (even os.path.exists) for a window around the
    remount, then recovers on its own. Checking + waiting here, instead of
    reacting to whatever exception each caller happens to hit, keeps that
    handling in one place."""
    path.mkdir(parents=True, exist_ok=True)
    sentinel = path / ".mount_ready_check"
    for attempt in range(1, retries + 1):
        try:
            sentinel.write_text(str(time.time()))
            sentinel.read_text()
            sentinel.unlink()
            return True
        except OSError as exc:
            wait = backoff_s * attempt
            print(
                f"  mount check failed for {path} (attempt {attempt}/{retries}): "
                f"{exc}; waiting {wait:.0f}s"
            )
            time.sleep(wait)
    return False


def sample_rss_kb(pid: int) -> int | None:
    try:
        out = subprocess.run(
            ["ps", "-o", "rss=", "-p", str(pid)], capture_output=True, text=True
        )
        val = out.stdout.strip()
        return int(val) if val else None
    except Exception:  # noqa: BLE001
        return None


@dataclass
class RunResult:
    label: str
    cmd: list[str]
    exit_code: int | None = None
    wall_s: float = 0.0
    peak_rss_mb: float = 0.0
    killed: bool = False
    kill_reason: str | None = None
    run_json: dict | None = None
    log_path: str = ""
    # A run that never produced run.json despite the process exiting 0, or
    # that raised while starting/writing, distinct from a clean success and
    # from an intentional RSS-limit kill -- see run_one_attempt / retry loop.
    failed: bool = False
    fail_reason: str | None = None
    attempts: int = 1


def run_with_watchdog(
    cmd: list[str],
    label: str,
    log_path: Path,
    rss_limit_mb: float,
    poll_interval_s: float,
) -> RunResult:
    result = RunResult(label=label, cmd=cmd, log_path=str(log_path))
    limit_kb = rss_limit_mb * 1024
    start = time.monotonic()
    with open(log_path, "w") as log_file:
        proc = subprocess.Popen(cmd, stdout=log_file, stderr=subprocess.STDOUT)
        try:
            while True:
                ret = proc.poll()
                rss_kb = sample_rss_kb(proc.pid)
                if rss_kb is not None:
                    result.peak_rss_mb = max(result.peak_rss_mb, rss_kb / 1024)
                    if rss_kb > limit_kb:
                        proc.kill()
                        proc.wait()
                        result.killed = True
                        result.kill_reason = (
                            f"RSS {rss_kb / 1024:.0f}MB exceeded "
                            f"--rss-limit-mb {rss_limit_mb:.0f}MB"
                        )
                        break
                if ret is not None:
                    result.exit_code = ret
                    break
                time.sleep(poll_interval_s)
        except KeyboardInterrupt:
            proc.kill()
            proc.wait()
            result.killed = True
            result.kill_reason = "interrupted (Ctrl-C)"
            raise
    result.wall_s = time.monotonic() - start
    if result.exit_code is None and not result.killed:
        result.exit_code = proc.returncode
    return result


def parse_config(spec: str) -> dict:
    cfg: dict[str, str] = {}
    for part in spec.split(","):
        key, _, value = part.partition("=")
        cfg[key.strip()] = value.strip()
    required = {"rows", "occupancy", "partitions", "memory-limit-mb"}
    missing = required - cfg.keys()
    if missing:
        raise ValueError(f"config '{spec}' missing keys: {sorted(missing)}")
    return cfg


def ensure_source(
    bench_bin: Path,
    source_dir: Path,
    rows: str,
    occupancy: str,
    seed: str,
    batch_rows: str,
    min_free_gb: float,
    max_retries: int,
    retry_backoff_s: float,
) -> Path:
    if not mount_ready(source_dir):
        raise SystemExit(f"{source_dir}'s volume never became ready for writes")
    source_path = source_dir / f"{occupancy}-{rows}-seed{seed}.parquet"
    if source_path.exists():
        print(f"  source already exists, reusing: {source_path}")
        return source_path
    cmd = [
        str(bench_bin),
        "generate",
        "--output",
        str(source_path),
        "--rows",
        rows,
        "--occupancy",
        occupancy,
        "--seed",
        seed,
        "--batch-rows",
        batch_rows,
    ]
    log_path = source_dir / f"{occupancy}-{rows}-seed{seed}.generate.log"

    for attempt in range(1, max_retries + 2):
        free = free_gb(source_dir)
        if free is None or free < min_free_gb:
            raise SystemExit(
                f"refusing to generate: {free!r}GB free on {source_dir}, "
                f"below --min-free-gb {min_free_gb}"
            )
        print(f"  generating source (attempt {attempt}): {' '.join(cmd)}")
        try:
            result = run_with_watchdog(
                cmd, "generate", log_path, rss_limit_mb=8000, poll_interval_s=0.5
            )
            if result.killed:
                raise RuntimeError(f"generation killed: {result.kill_reason}")
            if result.exit_code != 0:
                raise RuntimeError(f"generation exited {result.exit_code}")
            if not source_path.exists():
                raise RuntimeError("generation exited 0 but source file is missing")
            return source_path
        except (OSError, RuntimeError) as exc:
            source_path.unlink(missing_ok=True)
            if attempt > max_retries:
                raise SystemExit(
                    f"source generation failed after {attempt} attempts: {exc}"
                ) from exc
            wait = retry_backoff_s * attempt
            print(f"  generation attempt {attempt} failed ({exc}); retrying in {wait:.0f}s")
            mount_ready(source_dir)
            time.sleep(wait)
    raise AssertionError("unreachable")


def write_watchdog_sidecar(path: Path, result: RunResult) -> None:
    path.write_text(
        json.dumps(
            {
                "label": result.label,
                "cmd": result.cmd,
                "exit_code": result.exit_code,
                "wall_s": result.wall_s,
                "peak_rss_mb": result.peak_rss_mb,
                "killed": result.killed,
                "kill_reason": result.kill_reason,
                "log_path": result.log_path,
                "failed": result.failed,
                "fail_reason": result.fail_reason,
                "attempts": result.attempts,
            },
            indent=2,
        )
    )


def load_watchdog_sidecar(path: Path) -> RunResult:
    saved = json.loads(path.read_text())
    return RunResult(
        label=saved["label"],
        cmd=saved["cmd"],
        exit_code=saved["exit_code"],
        wall_s=saved["wall_s"],
        peak_rss_mb=saved["peak_rss_mb"],
        killed=saved["killed"],
        kill_reason=saved["kill_reason"],
        log_path=saved["log_path"],
        failed=saved.get("failed", False),
        fail_reason=saved.get("fail_reason"),
        attempts=saved.get("attempts", 1),
    )


def run_one_with_retries(
    cmd: list[str],
    label: str,
    run_dir: Path,
    log_path: Path,
    rss_limit_mb: float,
    poll_interval_s: float,
    max_retries: int,
    retry_backoff_s: float,
) -> RunResult:
    """Retries transient failures (I/O errors from a flaky external drive,
    a run that exits 0 but never wrote run.json) by discarding the partial
    output directory and trying again -- each `rust` subcommand invocation
    is a clean, deterministic function of its input file, so a from-scratch
    retry is always safe. An RSS-limit kill is NOT retried: that's the
    watchdog doing its job, not a transient fault, so it's returned
    immediately for the caller to act on."""
    last_exc: Exception | None = None
    for attempt in range(1, max_retries + 2):
        if run_dir.exists():
            shutil.rmtree(run_dir, ignore_errors=True)
        log_path.unlink(missing_ok=True)
        if not mount_ready(run_dir.parent):
            last_exc = RuntimeError(f"{run_dir.parent}'s volume never became ready")
            break
        try:
            result = run_with_watchdog(cmd, label, log_path, rss_limit_mb, poll_interval_s)
        except OSError as exc:
            last_exc = exc
            print(f"  {label}: attempt {attempt} raised {exc!r}")
            if attempt <= max_retries:
                wait = retry_backoff_s * attempt
                print(f"  {label}: retrying in {wait:.0f}s")
                time.sleep(wait)
            continue
        result.attempts = attempt
        if result.killed:
            return result  # deliberate RSS kill: not a transient fault, don't retry
        run_json_path = run_dir / "run.json"
        if result.exit_code == 0 and run_json_path.exists():
            result.run_json = json.loads(run_json_path.read_text())
            return result
        last_exc = RuntimeError(
            f"exit={result.exit_code}, run.json present={run_json_path.exists()}"
        )
        print(f"  {label}: attempt {attempt} did not produce a valid result ({last_exc})")
        if attempt <= max_retries:
            wait = retry_backoff_s * attempt
            print(f"  {label}: retrying in {wait:.0f}s")
            time.sleep(wait)

    failed_result = RunResult(
        label=label,
        cmd=cmd,
        log_path=str(log_path),
        failed=True,
        fail_reason=str(last_exc),
        attempts=max_retries + 1,
    )
    return failed_result


def run_config(
    bench_bin: Path,
    source_dir: Path,
    output_root: Path,
    cfg: dict,
    measured_runs: int,
    rss_limit_mb: float,
    poll_interval_s: float,
    min_free_gb: float,
    seed: str,
    batch_rows: str,
    max_retries: int,
    retry_backoff_s: float,
    inter_run_pause_s: float,
) -> dict:
    rows = cfg["rows"]
    occupancy = cfg["occupancy"]
    partitions = cfg["partitions"]
    memory_limit_mb = cfg["memory-limit-mb"]
    config_label = f"{occupancy}-{rows}rows-p{partitions}-mem{memory_limit_mb}mb"
    print(f"\n=== config: {config_label} ===")

    before = env_snapshot()
    print(f"  loadavg before: {before['loadavg']}")

    source_path = ensure_source(
        bench_bin,
        source_dir,
        rows,
        occupancy,
        seed,
        batch_rows,
        min_free_gb,
        max_retries,
        retry_backoff_s,
    )

    config_dir = output_root / config_label
    config_dir.mkdir(parents=True, exist_ok=True)

    runs: list[RunResult] = []
    labels = ["warmup"] + [f"run_{i:02d}" for i in range(1, measured_runs + 1)]
    for label in labels:
        run_dir = config_dir / label
        watchdog_path = config_dir / f"{label}.watchdog.json"
        if run_dir.exists() or watchdog_path.exists():
            if watchdog_path.exists():
                result = load_watchdog_sidecar(watchdog_path)
                run_json_path = run_dir / "run.json"
                if run_json_path.exists():
                    result.run_json = json.loads(run_json_path.read_text())
                runs.append(result)
                state = "FAILED" if result.failed else (
                    "KILLED" if result.killed else "ok"
                )
                print(
                    f"  {label}: reusing prior result from {watchdog_path} "
                    f"({state}, peak_rss={result.peak_rss_mb:.0f}MB, "
                    f"wall={result.wall_s:.1f}s)"
                )
            else:
                print(
                    f"  {label}: output dir exists but no watchdog sidecar "
                    f"({watchdog_path}) -- excluded from this run's stats, "
                    "pre-dates the sidecar fix"
                )
            continue

        free = free_gb(output_root)
        if free is None:
            print(f"  {label}: {output_root} not reachable, waiting for it")
            if not mount_ready(output_root):
                print(f"  ABORTING remaining runs for {config_label}: volume never came back")
                break
            free = free_gb(output_root)
        if free is None or free < min_free_gb:
            print(
                f"  ABORTING remaining runs for {config_label}: "
                f"{free!r}GB free, below --min-free-gb {min_free_gb}"
            )
            break

        if runs and inter_run_pause_s > 0:
            # Give the (possibly flaky external) drive a moment to settle
            # between sustained multi-GB writes rather than hammering it
            # back-to-back -- cheap mitigation for the disconnect this was
            # added after (see notebook Entry 5).
            time.sleep(inter_run_pause_s)

        cmd = [
            str(bench_bin),
            "rust",
            "--input",
            str(source_path),
            "--output-dir",
            str(run_dir),
            "--partitions",
            partitions,
            "--memory-limit-mb",
            memory_limit_mb,
        ]
        print(f"  {label}: {' '.join(cmd)}")
        log_path = config_dir / f"{label}.log"
        result = run_one_with_retries(
            cmd,
            label,
            run_dir,
            log_path,
            rss_limit_mb,
            poll_interval_s,
            max_retries,
            retry_backoff_s,
        )
        write_watchdog_sidecar(watchdog_path, result)
        runs.append(result)
        status = "FAILED" if result.failed else ("KILLED" if result.killed else f"exit={result.exit_code}")
        print(
            f"    -> {status}, attempts={result.attempts}, wall={result.wall_s:.1f}s, "
            f"peak_rss={result.peak_rss_mb:.0f}MB"
        )
        if result.killed:
            print(
                f"  ABORTING remaining runs for {config_label}: {result.kill_reason}"
            )
            break
        if result.failed:
            print(
                f"  ABORTING remaining runs for {config_label}: "
                f"gave up after {result.attempts} attempts ({result.fail_reason})"
            )
            break

    after = env_snapshot()
    measured = [
        r for r in runs if r.label != "warmup" and not r.killed and not r.failed
    ]
    summary = {
        "config": cfg,
        "config_label": config_label,
        "source_path": str(source_path),
        "env_before": before,
        "env_after": after,
        "runs": [
            {
                "label": r.label,
                "exit_code": r.exit_code,
                "wall_s": r.wall_s,
                "peak_rss_mb": r.peak_rss_mb,
                "killed": r.killed,
                "kill_reason": r.kill_reason,
                "failed": r.failed,
                "fail_reason": r.fail_reason,
                "attempts": r.attempts,
                "log_path": r.log_path,
                "run_json": r.run_json,
            }
            for r in runs
        ],
        "measured_run_count": len(measured),
        "any_killed": any(r.killed for r in runs),
        "any_failed": any(r.failed for r in runs),
    }
    if measured:
        rss_vals = sorted(r.peak_rss_mb for r in measured)
        wall_vals = sorted(r.wall_s for r in measured)
        mid = len(rss_vals) // 2
        summary["peak_rss_mb_median"] = rss_vals[mid]
        summary["peak_rss_mb_max"] = rss_vals[-1]
        summary["wall_s_median"] = wall_vals[mid]
        summary["wall_s_max"] = wall_vals[-1]
    return summary


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--bench-bin", required=True, type=Path)
    parser.add_argument("--source-dir", required=True, type=Path)
    parser.add_argument("--output-root", required=True, type=Path)
    parser.add_argument(
        "--config",
        action="append",
        required=True,
        help="rows=N,occupancy=sparse|dense,partitions=N,memory-limit-mb=N "
        "(repeat --config for multiple)",
    )
    parser.add_argument("--measured-runs", type=int, default=5)
    parser.add_argument(
        "--rss-limit-mb",
        type=float,
        default=12000,
        help="kill a run if its RSS exceeds this (default leaves ~4GB "
        "headroom on a 16GB host)",
    )
    parser.add_argument("--poll-interval-ms", type=float, default=500)
    parser.add_argument(
        "--min-free-gb",
        type=float,
        default=10,
        help="abort before starting a run if free disk on the relevant "
        "volume drops below this",
    )
    parser.add_argument("--seed", default="1")
    parser.add_argument("--batch-rows", default="65536")
    parser.add_argument(
        "--max-retries",
        type=int,
        default=2,
        help="retries for a transient failure (I/O error, missing run.json) "
        "before giving up on that run -- an RSS-limit kill is never retried",
    )
    parser.add_argument(
        "--retry-backoff-s",
        type=float,
        default=5.0,
        help="base backoff between retries, multiplied by attempt number",
    )
    parser.add_argument(
        "--inter-run-pause-s",
        type=float,
        default=3.0,
        help="pause between measured runs to give a flaky external drive "
        "a moment to settle instead of hammering it back-to-back",
    )
    args = parser.parse_args()

    if not args.bench_bin.exists():
        print(f"bench binary not found: {args.bench_bin}", file=sys.stderr)
        return 1

    args.output_root.mkdir(parents=True, exist_ok=True)
    configs = [parse_config(spec) for spec in args.config]

    all_summaries = []
    for cfg in configs:
        try:
            summary = run_config(
                bench_bin=args.bench_bin,
                source_dir=args.source_dir,
                output_root=args.output_root,
                cfg=cfg,
                measured_runs=args.measured_runs,
                rss_limit_mb=args.rss_limit_mb,
                poll_interval_s=args.poll_interval_ms / 1000,
                min_free_gb=args.min_free_gb,
                seed=args.seed,
                batch_rows=args.batch_rows,
                max_retries=args.max_retries,
                retry_backoff_s=args.retry_backoff_s,
                inter_run_pause_s=args.inter_run_pause_s,
            )
        except (SystemExit, OSError) as exc:
            # A config-level failure (e.g. source generation never recovered
            # from a dropped mount) shouldn't take down configs already
            # queued after it, or lose the results already written.
            print(f"  CONFIG FAILED: {cfg} -- {exc}", file=sys.stderr)
            all_summaries.append(
                {"config": cfg, "config_label": None, "config_error": str(exc)}
            )
            results_path = args.output_root / "matrix_results.json"
            results_path.write_text(json.dumps(all_summaries, indent=2))
            continue
        all_summaries.append(summary)
        results_path = args.output_root / "matrix_results.json"
        results_path.write_text(json.dumps(all_summaries, indent=2))
        print(f"  wrote {results_path}")
        if summary["any_killed"]:
            print(
                f"  STOPPING matrix: {summary['config_label']} hit the RSS "
                "limit; not proceeding to further configs automatically.",
                file=sys.stderr,
            )
            return 2

    print("\n=== summary ===")
    for s in all_summaries:
        if s.get("config_label") is None:
            print(f"{s['config']}: CONFIG FAILED -- {s.get('config_error')}")
            continue
        print(
            f"{s['config_label']}: "
            f"{s['measured_run_count']} measured runs, "
            f"peak_rss_median={s.get('peak_rss_mb_median', float('nan')):.0f}MB, "
            f"peak_rss_max={s.get('peak_rss_mb_max', float('nan')):.0f}MB, "
            f"wall_median={s.get('wall_s_median', float('nan')):.1f}s"
            + (" [FAILURES OCCURRED]" if s.get("any_failed") else "")
        )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
