"""Independent warm-cache process trials, including reading and output formatting."""
import argparse
import hashlib
import json
import os
import platform
import re
import statistics
import subprocess
import tempfile
import time
from pathlib import Path


OPERATIONS = ("inspect", "describe", "pearson", "frequencies", "histogram", "scatter", "boxplot", "one-sample-t", "paired-t", "independent-t", "regression", "reliability",
              "anova-one-way", "anova-factorial", "anova-repeated", "load", "save", "export", "edit", "groups", "cancel", "shutdown", "save-load")


HISTORICAL_INSPECT_SHA256 = "74bae312170fbb64af6091082cfb492104a88202f1cedd6c6d6b01b3b57565d2"

def parse_metrics(stderr):
    stages = {name: float(value) for name, value in
              re.findall(r"^stage_([a-z_]+)_ms=(\d+(?:\.\d+)?)$", stderr, re.MULTILINE)}
    retained = {name: int(value) for name, value in
                re.findall(r"^retained_([a-z_]+)_bytes=(\d+)$", stderr, re.MULTILINE)}
    return {"stage_ms": stages, "retained_bytes": retained}


def sha256(path):
    with path.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256")
    return digest.hexdigest()


def measure(binary, fixture, trials, operation):
    if trials < 5:
        raise ValueError("At least five trials are required.")
    if operation not in OPERATIONS:
        raise ValueError("Choose an operation from " + ", ".join(OPERATIONS) + ".")
    # Reading the fixture identity warms the cache, matching the preserved baseline.
    identity = {"path": str(fixture.resolve()), "bytes": fixture.stat().st_size,
                "sha256": sha256(fixture)}
    targets_applicable = operation == "inspect" and identity["sha256"] == HISTORICAL_INSPECT_SHA256
    records = []
    for _ in range(trials):
        with tempfile.TemporaryFile() as errors:
            start = time.perf_counter()
            process = subprocess.Popen([str(binary.resolve()), operation, str(fixture.resolve())],
                                       stdout=subprocess.DEVNULL, stderr=errors)
            if hasattr(os, "wait4"):
                _, status, usage = os.wait4(process.pid, 0)
                process.returncode = os.waitstatus_to_exitcode(status)
                divisor = 1024 * 1024 if platform.system() == "Darwin" else 1024
                resources = {"peak_rss_mib": usage.ru_maxrss / divisor,
                             "user_s": usage.ru_utime, "system_s": usage.ru_stime}
            else:
                process.wait()
                resources = {"peak_rss_mib": None, "user_s": None, "system_s": None}
            elapsed = (time.perf_counter() - start) * 1000
            errors.seek(0)
            stderr = errors.read(8192).decode("utf-8", errors="replace")
            retained = re.search(r"^retained_analysis_bytes=(\d+)$", stderr, re.MULTILINE)
            records.append({"wall_ms": elapsed, "exit": process.returncode,
                            "stderr": stderr, "error": stderr if process.returncode else "",
                            "retained_analysis_bytes": int(retained.group(1)) if retained else None,
                            **parse_metrics(stderr), **resources})
    successful = [record for record in records if record["exit"] == 0]
    median = statistics.median(record["wall_ms"] for record in successful) if successful else None
    rss = [record["peak_rss_mib"] for record in successful if record["peak_rss_mib"] is not None]
    peak = max(rss) if rss else None
    return {"operation": operation, "fixture": identity,
            "binary": {"path": str(binary.resolve()), "sha256": sha256(binary)},
            "platform": platform.platform(), "architecture": platform.machine(),
            "profile": "cargo release --no-default-features --example " + ("lifecycle_perf_probe" if operation in ("cancel", "shutdown", "save-load") else "perf_probe"),
            "file_cache": "Warm OS cache; fixture hash read before sequential independent trials",
            "output": "Application formats output; stdout discarded; stderr retained up to 8192 bytes",
            "rss": "wait4 per-process maximum; Darwin bytes, other Unix KiB; unavailable on Windows",
            "trials": records, "successful_trials": len(successful),
            "all_trials_succeeded": len(successful) == trials, "median_wall_ms": median,
            "max_peak_rss_mib": peak,
            "targets_applicable": targets_applicable,
            "targets": {"median_import_ms": 172.8565, "peak_rss_mib": 103.894} if targets_applicable else None,
            "targets_pass": targets_applicable and len(successful) == trials and
                peak is not None and peak <= 103.894 and median <= 172.8565 if targets_applicable else None}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--fixture", type=Path, required=True)
    parser.add_argument("--trials", type=int, default=5)
    parser.add_argument("--operation", choices=OPERATIONS, default="inspect")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("Output already exists; choose a new measurement file.")
    try:
        result = measure(args.binary, args.fixture, args.trials, args.operation)
    except (OSError, ValueError) as error:
        parser.error(str(error))
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("x") as destination:
        json.dump(result, destination, indent=2)
        destination.write("\n")
    print(json.dumps({key: result[key] for key in (
        "successful_trials", "median_wall_ms", "max_peak_rss_mib", "targets_pass")}))
    return 0 if result["all_trials_succeeded"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
