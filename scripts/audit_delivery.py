"""Read-only local delivery audit; records artifact identity and documentation drift."""
import argparse
import hashlib
import importlib.util
import json
from pathlib import Path, PurePosixPath
import re
import subprocess
import tomllib
from urllib.parse import unquote
import zipfile

SOURCE = Path(__file__).resolve().parent.parent


def sha256(path):
    digest = hashlib.sha256()
    with path.open("rb") as stream:
        for block in iter(lambda: stream.read(1024 * 1024), b""):
            digest.update(block)
    return digest.hexdigest()


def package_module():
    spec = importlib.util.spec_from_file_location("tessera_package", SOURCE / "scripts/package.py")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


def documentation(delivery):
    current = {SOURCE / "README.md", SOURCE / "docs/README.md", SOURCE / "docs/LICENSING.md",
               SOURCE / "docs/project-format.md", SOURCE / "docs/production-readiness.md"}
    documents = sorted((SOURCE / "docs").rglob("*.md"))
    documents += [SOURCE / "README.md", delivery / "START HERE.md"]
    documents += sorted((delivery / "Documentation").rglob("*.md"))
    records = []
    broken = []
    for path in documents:
        if not path.is_file():
            continue
        active = path in current or path == delivery / "START HERE.md" or (
            path.parent == delivery / "Documentation" and path.name in {"README.md", "PRODUCTION-READINESS.md", "USER-GUIDE.md"})
        text = path.read_text(encoding="utf-8")
        title = next((line.removeprefix("# ") for line in text.splitlines() if line.startswith("# ")), path.name)
        records.append({"path": str(path.relative_to(delivery)), "title": title,
                        "authority": "current" if active else "historical-evidence", "sha256": sha256(path)})
        for match in re.finditer(r"\[[^\]\n]*\]\(([^)\n]+)\)", text):
            link = match[1].strip().split(' "', 1)[0].strip("<>")
            if not link or link.startswith(("#", "https:", "http:", "mailto:")):
                continue
            destination = unquote(link.split("#", 1)[0])
            if destination and not (path.parent / destination).exists():
                broken.append({"document": str(path.relative_to(delivery)), "link": link,
                               "authority": "current" if active else "historical-evidence"})
    return {"documents": records, "broken_links": broken}


def audit(delivery):
    checks = []
    def check(name, passed, detail):
        checks.append({"check": name, "status": "pass" if passed else "fail", "detail": detail})

    package = delivery / "Tessera-macOS.zip"
    version = tomllib.loads((SOURCE / "Cargo.toml").read_text())["package"]["version"]
    identity = package_module().source_identity(SOURCE)
    artifact = {"path": str(package.relative_to(delivery)), "sha256": sha256(package), "bytes": package.stat().st_size}
    with zipfile.ZipFile(package) as archive:
        names = archive.namelist()
        check("zip-integrity", archive.testzip() is None, "CRC checked for every entry")
        safe = len(names) == len(set(names)) and all(
            name.startswith("Tessera/") and ".." not in PurePosixPath(name).parts and "\\" not in name for name in names)
        check("package-paths", safe, "Unique, relative product entries")
        hashes = json.loads(archive.read("Tessera/SHA256SUMS.json"))
        expected = {name.removeprefix("Tessera/") for name in names if not name.endswith("/SHA256SUMS.json")}
        check("checksum-inventory", set(hashes) == expected, "Every payload is enumerated")
        mismatches = [name for name, digest in hashes.items()
                      if hashlib.sha256(archive.read("Tessera/" + name)).hexdigest() != digest]
        check("package-checksums", not mismatches, mismatches)
        missing = [name for name in ["LICENSE", "docs/LICENSING.md", "BUILD-INFO.json"] if "Tessera/" + name not in names]
        check("release-notices-and-identity", not missing, missing)
        if "Tessera/BUILD-INFO.json" in names:
            info = json.loads(archive.read("Tessera/BUILD-INFO.json"))
            check("source-identity", info.get("source") == identity, "Packaged build-input manifest equals the active source")
            check("version", info.get("version") == version, version)
        drift = []
        for name, digest in hashes.items():
            if name.startswith("Tessera.app/"):
                path = delivery / name
            elif name.startswith("examples/"):
                path = delivery / "Examples" / Path(name).name
            else:
                continue
            if not path.is_file() or sha256(path) != digest:
                drift.append(name)
        check("installed-app-and-examples", not drift, drift)
        documents = [relative for relative in package_module().DOCUMENTS
                     if "Tessera/" + relative not in names or archive.read("Tessera/" + relative) != (SOURCE / relative).read_bytes()]
        check("packaged-current-documents", not documents, documents)
    docs = documentation(delivery)
    broken = [entry for entry in docs["broken_links"] if entry["authority"] == "current"]
    check("current-document-links", not broken, broken)
    for relative in ("project-format.md", "project.schema.json"):
        copy = delivery / "Documentation" / relative
        check("current-copy:" + relative, copy.is_file() and sha256(copy) == sha256(SOURCE / "docs" / relative), relative)
    signature = subprocess.run(["codesign", "--verify", "--deep", "--strict", str(delivery / "Tessera.app")], capture_output=True, text=True)
    check("macos-signature-integrity", signature.returncode == 0, signature.stderr.strip() or "Verified; does not imply Developer ID/notarization")
    return {"schema": "tessera-delivery-audit-v1", "version": version,
            "source": identity, "artifact": artifact, "checks": checks, "documentation": docs,
            "status": "pass" if all(c["status"] == "pass" for c in checks) else "fail",
            "scope": "Artifact and documentation coherence, not production authorization or native workflow acceptance"}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--delivery", type=Path, default=SOURCE.parent)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    report = audit(args.delivery.resolve())
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(report, indent=2) + "\n", encoding="utf-8")
    print(json.dumps({"status": report["status"], "checks": report["checks"],
                      "markdown_documents": len(report["documentation"]["documents"]),
                      "historical_broken_links": sum(e["authority"] != "current" for e in report["documentation"]["broken_links"])}, indent=2))
    return 0 if report["status"] == "pass" else 1


if __name__ == "__main__":
    raise SystemExit(main())
