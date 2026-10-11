"""Create a cache-free source archive with a supplied Cargo-vendored dependency union."""
import argparse
import hashlib
import io
import json
from pathlib import Path
import subprocess
import tarfile

from package import ROOT, source_identity


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--vendor", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("Archive already exists; use a new destination.")
    vendor = args.vendor.resolve()
    if not vendor.is_dir() or not any(vendor.glob("*/.cargo-checksum.json")):
        parser.error("Supply a complete cargo vendor directory.")
    names = subprocess.run(["git", "ls-files", "--cached", "--others", "--exclude-standard", "-z"],
                           cwd=ROOT, check=True, capture_output=True).stdout.decode().split("\0")
    files = {}
    for name in sorted(set(names) - {""}):
        path = ROOT / name
        if not path.is_file() or path.is_symlink():
            continue
        if any(part in {".git", ".build", "target", ".superpowers", "__pycache__"} for part in path.relative_to(ROOT).parts):
            continue
        files[name] = path
    for path in sorted(vendor.rglob("*")):
        if path.is_symlink():
            parser.error(f"Unexpected vendor symlink: {path}")
        if path.is_file():
            files["vendor/" + path.relative_to(vendor).as_posix()] = path
    extras = {
        ".cargo/config.toml": b'[source.crates-io]\nreplace-with = "vendored-sources"\n\n[source.vendored-sources]\ndirectory = "vendor"\n',
        "SOURCE-ARCHIVE.md": b'''# Tessera source snapshot

This archive contains the application, independent workbook worker, assets,
tests, documentation, locked Cargo graphs and their vendored registry source.
It is a development candidate, not a production release approval.

Install Rust 1.95+ (verified with 1.97.1), its native platform compiler/linker
and platform development libraries. On macOS, install Xcode Command Line Tools.
From this extracted directory:

    cargo test --locked --offline --all-features
    cargo test --locked --offline --manifest-path crates/tessera-workbook/Cargo.toml
    cargo build --locked --offline --release --manifest-path crates/tessera-workbook/Cargo.toml --target-dir target --bin tessera-workbook-worker
    cargo run --locked --offline --release

Excel requires the sibling workbook-worker binary. See README.md for packaging
and Python checks (the Python dependencies and Rust toolchain are not vendored).
The vendor configuration also applies to the independent worker workspace.
Source-archive identities are recorded in SOURCE-MANIFEST.json; the base build
input identity is comparable to the installer BUILD-INFO.json.
''',
    }
    identities = {name: hashlib.sha256(path.read_bytes()).hexdigest() for name, path in files.items()}
    identities.update({name: hashlib.sha256(data).hexdigest() for name, data in extras.items()})
    extras["SOURCE-MANIFEST.json"] = (json.dumps({"build_input_identity": source_identity(ROOT),
                                                 "files": identities}, indent=2) + "\n").encode()
    args.output.parent.mkdir(parents=True, exist_ok=True)
    with args.output.open("xb") as destination, tarfile.open(fileobj=destination, mode="w:gz") as archive:
        for name, path in sorted(files.items()):
            info = archive.gettarinfo(str(path), "Tessera-source/" + name)
            info.uid = info.gid = info.mtime = 0
            info.uname = info.gname = ""
            with path.open("rb") as stream:
                archive.addfile(info, stream)
        for name, data in sorted(extras.items()):
            info = tarfile.TarInfo("Tessera-source/" + name)
            info.size = len(data)
            info.mode = 0o644
            archive.addfile(info, io.BytesIO(data))
    print(json.dumps({"archive": str(args.output.resolve()), "files": len(identities),
                      "vendor_packages": len(list(vendor.glob("*/.cargo-checksum.json"))),
                      "build_input_sha256": source_identity(ROOT)["sha256"]}))


if __name__ == "__main__":
    main()
