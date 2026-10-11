"""Measure distinct Rust production source lines with matching installed LLVM tools."""
import argparse
import json
import os
import re
import subprocess
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def test_lines(lines):
    excluded = set()
    for index, line in enumerate(lines):
        if "#[cfg(test)]" not in line and not re.search(r"#\[cfg\(all\(\s*test\s*,", line):
            continue
        excluded.add(index + 1)
        cursor = index + 1
        while cursor < len(lines) and lines[cursor].strip().startswith("#["):
            excluded.add(cursor + 1)
            cursor += 1
        if cursor == len(lines):
            continue
        depth = 0
        started = False
        while cursor < len(lines):
            text = lines[cursor]
            excluded.add(cursor + 1)
            depth += text.count("{") - text.count("}")
            started |= "{" in text
            if (started and depth == 0) or (not started and text.rstrip().endswith(";")):
                break
            cursor += 1
    return excluded


def module_edges(path):
    """Resolve rustfmt-style external module declarations, including inline owners."""
    lines = path.read_text().splitlines()
    excluded = test_lines(lines)
    directory = path.parent if path.name in ("lib.rs", "main.rs", "mod.rs") else path.with_suffix("")
    inline = []
    attributes = []
    edges = []
    # shortcut: line-oriented rustfmt declarations; extend for macro-generated or multiline modules.
    for number, line in enumerate(lines, 1):
        stripped = line.strip()
        indent = len(line) - len(line.lstrip())
        while inline and stripped.startswith("}") and indent <= inline[-1][0]:
            inline.pop()
        if stripped.startswith("#["):
            attributes.append(stripped)
            continue
        if not stripped or stripped.startswith("//"):
            continue
        declaration = re.fullmatch(r"(?:pub(?:\([^)]*\))?\s+)?mod\s+(\w+)\s*([;{])", stripped)
        if declaration:
            name, ending = declaration.groups()
            owner = inline[-1][1] if inline else directory
            override = re.search(r'#\[path\s*=\s*"([^"\n]+)"\]', " ".join(attributes))
            if ending == "{":
                inline.append((indent, owner / name))
            else:
                if override:
                    candidates = [(owner if inline else path.parent) / override[1]]
                else:
                    candidates = [owner / (name + ".rs"), owner / name / "mod.rs"]
                matches = [candidate.resolve() for candidate in candidates if candidate.is_file()]
                if len(matches) != 1:
                    raise ValueError(f"Cannot resolve Rust module {name} in {path}:{number}")
                edges.append((matches[0], number in excluded))
        attributes = []
    return edges


def source_roots(root):
    return [path for path in (root / "src", root / "crates/tessera-workbook/src") if path.is_dir()]


def external_test_files(root):
    graph = {path.resolve(): module_edges(path) for source in source_roots(root)
             for path in source.rglob("*.rs")}

    def reachable(seeds, production):
        found = set()
        pending = list(seeds)
        while pending:
            path = pending.pop()
            if path in found:
                continue
            found.add(path)
            pending.extend(child for child, test_only in graph.get(path, [])
                           if not production or not test_only)
        return found

    test_roots = {child for edges in graph.values() for child, test_only in edges if test_only}
    production_roots = {(source / name).resolve() for source in source_roots(root)
                        for name in ("lib.rs", "main.rs")}
    return reachable(test_roots, False) - reachable(production_roots, True)


def summarize(lcov, root):
    root = root.resolve()
    sources = {}
    excluded_files = external_test_files(root)
    for path in sorted(path for source in source_roots(root) for path in source.rglob("*.rs")):
        if path.resolve() in excluded_files:
            continue
        sources[str(path.resolve())] = ({}, test_lines(path.read_text().splitlines()))
    current = None
    for line in lcov.splitlines():
        if line.startswith("SF:"):
            current = str(Path(line[3:]).resolve())
        elif line.startswith("DA:") and current in sources:
            number, count = map(int, line[3:].split(",")[:2])
            counts, excluded = sources[current]
            if number not in excluded:
                counts[number] = max(counts.get(number, 0), count)
    rows = []
    missing = []
    for path, (counts, excluded) in sources.items():
        relative = Path(path).relative_to(root).as_posix()
        if not counts:
            missing.append(relative)
            continue
        rows.append({"file": relative, "covered": sum(count > 0 for count in counts.values()),
                     "total": len(counts), "excluded_test_lines": sorted(excluded),
                     "uncovered_lines": sorted(number for number, count in counts.items() if count == 0)})
    covered = sum(row["covered"] for row in rows)
    total = sum(row["total"] for row in rows)
    return {"covered": covered, "total": total, "percent": 100 * covered / total if total else 0,
            "files": rows, "not_instrumented_files": missing,
            "excluded_test_files": sorted(path.relative_to(root).as_posix() for path in excluded_files),
            "policy": "Distinct source DA lines, max hit count across objects; application and workbook worker included. External dependencies, tests/, examples/, cfg(test) items and exclusively test-owned external module trees excluded. Desktop included. Uncompiled platform files listed explicitly."}


def command(args, **kwargs):
    return subprocess.run(args, cwd=ROOT, check=True, **kwargs)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--raw", type=Path)
    args = parser.parse_args()
    destination = args.output.resolve()
    destination.mkdir(parents=True, exist_ok=True)
    raw = args.raw.resolve() if args.raw else destination / "raw"
    raw.mkdir(parents=True, exist_ok=True)
    target = ROOT / ".build/coverage-target"
    flags = os.environ.get("RUSTFLAGS", "")
    if "instrument-coverage" not in flags:
        flags += " -C instrument-coverage"
    environment = dict(os.environ, CARGO_BUILD_JOBS="2", CARGO_TARGET_DIR=str(target),
                       RUSTFLAGS=flags.strip(), LLVM_PROFILE_FILE=str(raw / "%m-%p.profraw"))
    objects = set()
    worker = ROOT / "crates/tessera-workbook/Cargo.toml"
    try:
        if worker.is_file():
            proof = command(["cargo", "test", "--offline", "--locked", "--manifest-path", str(worker),
                             "--message-format=json"], env=environment, capture_output=True, text=True)
            (destination / "worker-test-run.log").write_text(proof.stderr + proof.stdout)
            for line in proof.stdout.splitlines():
                if line.startswith("{"):
                    item = json.loads(line)
                    if item.get("reason") == "compiler-artifact" and item.get("executable"):
                        objects.add(item["executable"])
        result = command(["cargo", "test", "--offline", "--all-features", "--message-format=json"],
                         env=environment, capture_output=True, text=True)
    except subprocess.CalledProcessError as error:
        (destination / "test-run.log").write_text((error.stderr or "") + (error.stdout or ""))
        raise
    (destination / "test-run.log").write_text(result.stderr + result.stdout)
    for line in result.stdout.splitlines():
        if not line.startswith("{"):
            continue
        item = json.loads(line)
        if item.get("reason") == "compiler-artifact" and item.get("executable"):
            objects.add(item["executable"])
    if not objects:
        raise RuntimeError("No current instrumented test objects were reported.")
    sysroot = Path(command(["rustc", "--print", "sysroot"], capture_output=True, text=True).stdout.strip())
    host = command(["rustc", "-vV"], capture_output=True, text=True).stdout
    host = re.search(r"^host: (.+)$", host, re.MULTILINE).group(1)
    llvm = sysroot / "lib/rustlib" / host / "bin"
    profiles = sorted(raw.glob("*.profraw"))
    if not profiles:
        raise RuntimeError("No execution profiles were written.")
    merged = destination / "merged.profdata"
    command([str(llvm / "llvm-profdata"), "merge", "-sparse", *(str(path) for path in profiles), "-o", str(merged)])
    objects = sorted(objects)
    export = [str(llvm / "llvm-cov"), "export", objects[0], "-format=lcov", "-instr-profile=" + str(merged)]
    for obj in objects[1:]:
        export.extend(["-object", obj])
    lcov = command(export, capture_output=True, text=True).stdout
    (destination / "production.lcov").write_text(lcov)
    report = summarize(lcov, ROOT)
    report.update({"configuration": "all-features " + host, "objects": objects,
                   "profiles": len(profiles), "minimum_percent": 80.0})
    (destination / "summary.json").write_text(json.dumps(report, indent=2) + "\n")
    print(json.dumps({key: report[key] for key in ("configuration", "covered", "total", "percent", "not_instrumented_files")}))
    return 0 if report["percent"] >= 80.0 else 1


if __name__ == "__main__":
    raise SystemExit(main())
