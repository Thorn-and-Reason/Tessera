"""Build macOS universal, Windows x64 or native Linux GNU archives from a product allowlist."""
import argparse
import hashlib
import json
import os
import plistlib
import shutil
import subprocess
import tempfile
import tomllib
import zipfile
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
VERSION = tomllib.loads((ROOT / "Cargo.toml").read_text())["package"]["version"]
BUILD_NUMBER = VERSION
DOCUMENTS = (
    "README.md", "LICENSE", "docs/LICENSING.md", "docs/INSTALLING.md",
    "docs/using-tessera.md", "docs/project-format.md", "docs/project.schema.json",
    "docs/build-and-distribution.md", "docs/production-readiness.md",
    "docs/design/instant-survey-report.md", "docs/design/survey-improvements.md",
    "examples/sample.tessera", "examples/mixed-analysis.tessera",
    "examples/exploration.tessera", "examples/regression-analysis.tessera",
    "assets/fonts/LICENSE", "assets/fonts/Libron-LICENSE.txt",
    "assets/fonts/provenance.txt", "assets/fonts/glyph-check.txt",
)


def source_identity(root):
    """Commit the actual build inputs, including uncommitted source and worker."""
    files = set()
    for relative in ("src", "assets", "crates/tessera-workbook/src"):
        files.update(path for path in (root / relative).rglob("*") if path.is_file())
    files.update((root / "scripts").glob("*.py"))
    for relative in (*DOCUMENTS, "Cargo.toml", "Cargo.lock", "build.rs",
                     "crates/tessera-workbook/Cargo.toml", "crates/tessera-workbook/Cargo.lock",
                     "scripts/package.py"):
        path = root / relative
        if path.is_file():
            files.add(path)
    identities = {path.relative_to(root).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
                  for path in sorted(files)}
    canonical = json.dumps(identities, sort_keys=True, separators=(",", ":")).encode()
    return {"sha256": hashlib.sha256(canonical).hexdigest(), "files": identities}


def command(args, **kwargs):
    return subprocess.run(args, cwd=ROOT, check=True, **kwargs)


def dependency_notices(target):
    packages = {}
    for manifest in (ROOT / "Cargo.toml", ROOT / "crates/tessera-workbook/Cargo.toml"):
        result = command(["cargo", "metadata", "--manifest-path", str(manifest),
                          "--offline", "--locked", "--format-version", "1",
                          "--filter-platform", target], capture_output=True, text=True)
        metadata = json.loads(result.stdout)
        active = {node["id"] for node in metadata["resolve"]["nodes"]}
        for package in metadata["packages"]:
            if package["id"] in active and package["source"] is not None:
                packages[(package["name"], package["version"], package["source"])] = package
    notices = ["Tessera — third-party dependency notices\n",
               "Font notices accompany this file in assets/fonts/.\n"]
    for package in sorted(packages.values(), key=lambda value: (value["name"], value["version"])):
        notices.extend([f"\n{'=' * 72}\n{package['name']} {package['version']}\n",
                        f"Licence: {package.get('license') or 'See accompanying licence text'}\n",
                        f"Source: {package['source']}\n",
                        f"Authors: {', '.join(package['authors'])}\n"])
        folder = Path(package["manifest_path"]).parent
        licenses = sorted(path for path in folder.iterdir()
                          if path.is_file() and path.name.lower().startswith(("license", "licence", "copying", "copyright")))
        additional = sorted(path for path in folder.iterdir()
                            if path.is_file() and path.name.lower().startswith(("notice", "authors")))
        for path in additional:
            if not path.resolve().is_relative_to(folder.resolve()):
                raise ValueError(f"Dependency licence escapes its package: {package['name']}")
        # r-efi carries its licence grant and copyright notices in AUTHORS.
        has_grant = licenses or any("Permission is hereby granted" in path.read_text(encoding="utf-8")
                                    for path in additional)
        licenses.extend(additional)
        # Embedded components can carry independent terms (e.g. tracing's spin).
        licenses.extend(sorted(path for path in folder.rglob("*")
                               if path.is_file() and path.parent != folder
                               and path.name.lower().split(".")[0] in
                               ("license", "licence", "copying", "copyright", "notice", "notices")
                               and path.suffix.lower() != ".rs"))
        # egui's embedded fallback fonts carry separate OFL/UFL/Hack/emoji notices.
        if (folder / "fonts").is_dir():
            licenses.extend(path for path in sorted((folder / "fonts").rglob("*.txt")) if path not in licenses)
        if package.get("license_file"):
            custom = folder / package["license_file"]
            if custom not in licenses:
                licenses.append(custom)
            has_grant = True
        for path in licenses:
            if not path.resolve().is_relative_to(folder.resolve()):
                raise ValueError(f"Dependency licence escapes its package: {package['name']}")
            notices.append(f"\n--- {path.relative_to(folder).as_posix()} ---\n")
            notices.append(path.read_text(encoding="utf-8", errors="replace"))
        if not has_grant:
            notices.extend(supplemental_licenses(package, folder))
    return "".join(notices)


def supplemental_licenses(package, folder):
    """Use reviewed, revision-bound upstream texts when a crate omits them."""
    root = ROOT / "assets/dependency-licenses"
    records = json.loads((root / "manifest.json").read_text())
    key = f"{package['name']}-{package['version']}"
    record = records.get(key)
    if not record or not record["files"] or record["license"] != package.get("license"):
        raise ValueError(f"Missing reviewed dependency licence text: {key}")
    vcs = json.loads((folder / ".cargo_vcs_info.json").read_text())
    if vcs["git"]["sha1"] != record["revision"]:
        raise ValueError(f"Dependency licence revision differs: {key}")
    notices = []
    for item in record["files"]:
        path = root / item["file"]
        if not path.resolve().is_relative_to(root.resolve()):
            raise ValueError(f"Supplemental licence escapes its directory: {key}")
        data = path.read_bytes()
        if hashlib.sha256(data).hexdigest() != item["sha256"]:
            raise ValueError(f"Supplemental licence checksum differs: {key}")
        notices.extend([f"\n--- {item['upstream_name']} ---\nSource: {item['url']}\n",
                        item.get("note", "") + "\n", data.decode("utf-8")])
    return notices


def archive(platform, output, binary, worker, root, notices):
    if output.exists():
        raise FileExistsError(f"Archive already exists: {output}")
    for executable in (binary, worker):
        if not executable.is_file():
            raise FileNotFoundError(f"Required executable missing: {executable}")
    if len(set(DOCUMENTS)) != len(DOCUMENTS):
        raise ValueError("Duplicate product document in allowlist.")
    for relative in DOCUMENTS:
        path = Path(relative)
        if (not relative or "\\" in relative or ":" in relative or path.as_posix() != relative
                or path.is_absolute() or ".." in path.parts
                or not (root / path).resolve().is_relative_to(root.resolve())):
            raise ValueError(f"Unsafe product document: {relative}")
    output.parent.mkdir(parents=True, exist_ok=True)
    with tempfile.TemporaryDirectory(prefix="thorn-reason-package-") as temporary:
        base = Path(temporary) / "Tessera"
        base.mkdir()
        if platform == "macos":
            app = base / "Tessera.app" / "Contents"
            (app / "MacOS").mkdir(parents=True)
            (app / "Resources").mkdir()
            executable = app / "MacOS" / "Tessera"
            shutil.copy2(binary, executable)
            executable.chmod(0o755)
            helper = app / "MacOS/tessera-workbook-worker"
            shutil.copy2(worker, helper)
            helper.chmod(0o755)
            shutil.copy2(root / "assets/tessera-icon.png", app / "Resources/tessera-icon.png")
            icons = Path(temporary) / "Tessera.iconset"
            icons.mkdir()
            for size in (16, 32, 128, 256, 512):
                for scale, suffix in ((1, ""), (2, "@2x")):
                    dimension = str(size * scale)
                    command(["sips", "--resampleHeightWidth", dimension, dimension,
                             str(root / "assets/tessera-icon.png"), "--out",
                             str(icons / f"icon_{size}x{size}{suffix}.png")], stdout=subprocess.DEVNULL)
            command(["iconutil", "--convert", "icns", str(icons), "--output",
                     str(app / "Resources/Tessera.icns")])
            with (app / "Info.plist").open("wb") as destination:
                plistlib.dump({"CFBundleName": "Tessera", "CFBundleDisplayName": "Tessera",
                               "CFBundleIdentifier": "com.thornandreason.tessera", "CFBundleExecutable": "Tessera",
                               "CFBundleIconFile": "Tessera.icns",
                               "CFBundlePackageType": "APPL", "CFBundleShortVersionString": VERSION,
                               "CFBundleVersion": BUILD_NUMBER, "NSHighResolutionCapable": True,
                                "LSMinimumSystemVersion": "11.0"}, destination)
            # Keep required notices when the app is dragged out of its ZIP folder.
            shutil.copy2(root / "LICENSE", app / "Resources/LICENSE")
            shutil.copy2(root / "docs/LICENSING.md", app / "Resources/LICENSING.md")
            (app / "Resources/THIRD-PARTY-NOTICES.txt").write_text(notices, encoding="utf-8")
            command(["codesign", "--force", "--sign", "-", str(helper)])
            command(["codesign", "--force", "--sign", "-", str(app.parent)])
        elif platform == "windows":
            shutil.copy2(binary, base / "tessera.exe")
            shutil.copy2(worker, base / "tessera-workbook-worker.exe")
        elif platform == "linux":
            executable = base / "tessera"
            shutil.copy2(binary, executable)
            executable.chmod(0o755)
            helper = base / "tessera-workbook-worker"
            shutil.copy2(worker, helper)
            helper.chmod(0o755)
        else:
            raise ValueError("Choose macos, windows or linux.")
        for relative in DOCUMENTS:
            destination = base / relative
            destination.parent.mkdir(parents=True, exist_ok=True)
            shutil.copy2(root / relative, destination)
        (base / "THIRD-PARTY-NOTICES.txt").write_text(notices, encoding="utf-8")
        (base / "BUILD-INFO.json").write_text(json.dumps({
            "product": "Tessera", "version": VERSION, "platform": platform,
            "distribution_status": "local-development-not-production-approved",
            "source": source_identity(root),
        }, indent=2) + "\n", encoding="utf-8")
        identities = {path.relative_to(base).as_posix(): hashlib.sha256(path.read_bytes()).hexdigest()
                      for path in sorted(base.rglob("*")) if path.is_file()}
        (base / "SHA256SUMS.json").write_text(json.dumps(identities, indent=2) + "\n", encoding="utf-8")
        # Exclusive archive creation preserves an existing destination even after a race.
        with output.open("xb") as destination:
            with zipfile.ZipFile(destination, "w", zipfile.ZIP_DEFLATED) as package:
                for path in sorted(base.rglob("*")):
                    if path.is_file():
                        info = zipfile.ZipInfo.from_file(path, path.relative_to(Path(temporary)).as_posix())
                        if platform in ("macos", "linux") and path in (executable, helper):
                            info.create_system = 3
                            info.external_attr = 0o100755 << 16
                        info.compress_type = zipfile.ZIP_DEFLATED
                        with path.open("rb") as source, package.open(info, "w") as entry:
                            shutil.copyfileobj(source, entry)


def build(platform):
    environment = dict(os.environ, CARGO_BUILD_JOBS="2", CARGO_TARGET_DIR=str(ROOT / "target"))
    if platform == "linux":
        version = command(["rustc", "-vV"], capture_output=True, text=True).stdout
        host = next((line.removeprefix("host: ") for line in version.splitlines() if line.startswith("host: ")), "")
        if host not in ("x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu"):
            raise ValueError("Linux packaging requires a native x86_64 or aarch64 Linux GNU Rust host; build on Linux.")
        targets = (host,)
    elif platform == "macos":
        targets = ("aarch64-apple-darwin", "x86_64-apple-darwin")
    elif platform == "windows":
        targets = ("x86_64-pc-windows-gnu",)
    else:
        raise ValueError("Choose macos, windows or linux.")
    if platform == "windows" and shutil.which("x86_64-w64-mingw32-gcc"):
        environment.setdefault("CARGO_TARGET_X86_64_PC_WINDOWS_GNU_LINKER", shutil.which("x86_64-w64-mingw32-gcc"))
    for target in targets:
        command(["cargo", "build", "--offline", "--locked", "--release", "--bin", "tessera", "--target", target], env=environment)
        command(["cargo", "build", "--manifest-path", str(ROOT / "crates/tessera-workbook/Cargo.toml"),
                 "--offline", "--locked", "--release", "--bin", "tessera-workbook-worker", "--target", target], env=environment)
    if platform in ("windows", "linux"):
        release = ROOT / "target" / targets[0] / "release"
        suffix = ".exe" if platform == "windows" else ""
        return (release / ("tessera" + suffix),
                release / ("tessera-workbook-worker" + suffix), dependency_notices(targets[0]))
    universal = ROOT / ".build/package/tessera-universal"
    worker = ROOT / ".build/package/tessera-workbook-worker-universal"
    universal.parent.mkdir(parents=True, exist_ok=True)
    for name, destination in (("tessera", universal), ("tessera-workbook-worker", worker)):
        command(["lipo", "-create", *(str(ROOT / "target" / target / "release" / name) for target in targets),
                 "-output", str(destination)])
        command(["codesign", "--force", "--sign", "-", str(destination)])
    return universal, worker, dependency_notices(targets[0])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--platform", choices=("macos", "windows", "linux"), required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    if args.output.exists():
        parser.error("Archive already exists; choose a new output path.")
    try:
        binary, worker, notices = build(args.platform)
        archive(args.platform, args.output, binary, worker, ROOT, notices)
    except (OSError, ValueError, subprocess.CalledProcessError) as error:
        parser.exit(1, f"Packaging failed: {error}\n")
    print(args.output.resolve())


if __name__ == "__main__":
    main()
