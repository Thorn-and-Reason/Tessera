import hashlib
import json
import importlib.util
import plistlib
import subprocess
import sys
import tempfile
import unittest
import zipfile
from pathlib import Path
from unittest.mock import patch


class PackageTests(unittest.TestCase):
    def test_supplemental_licence_is_revision_and_content_bound(self):
        spec = importlib.util.spec_from_file_location("package", Path(__file__).with_name("package.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            records = root / "assets/dependency-licenses"
            records.mkdir(parents=True)
            (root / ".cargo_vcs_info.json").write_text(json.dumps({"git": {"sha1": "revision"}}))
            data = b"upstream licence permission"
            (records / "license.txt").write_bytes(data)
            record = {"license": "MIT", "revision": "revision", "files": [{"file": "license.txt", "sha256": hashlib.sha256(data).hexdigest(), "upstream_name": "LICENSE", "url": "https://example.invalid/revision/LICENSE"}]}
            (records / "manifest.json").write_text(json.dumps({"fixture-1": record}))
            package = {"name": "fixture", "version": "1", "license": "MIT"}
            with patch.object(module, "ROOT", root):
                self.assertIn(data.decode(), "".join(module.supplemental_licenses(package, root)))
                (records / "license.txt").write_bytes(b"changed")
                with self.assertRaisesRegex(ValueError, "checksum"):
                    module.supplemental_licenses(package, root)
                (root / ".cargo_vcs_info.json").write_text(json.dumps({"git": {"sha1": "other revision"}}))
                with self.assertRaisesRegex(ValueError, "revision"):
                    module.supplemental_licenses(package, root)

    def test_dependency_notices_include_upstream_notice_and_fail_without_licence(self):
        spec = importlib.util.spec_from_file_location("package", Path(__file__).with_name("package.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory:
            folder = Path(directory)
            (folder / "LICENSE").write_text("fixture permission")
            (folder / "NOTICES.md").write_text("required upstream attribution")
            (folder / "src/spin").mkdir(parents=True)
            (folder / "src/spin/LICENSE").write_text("embedded component permission")
            package = {"id": "fixture", "name": "fixture", "version": "1", "source": "registry+fixture", "license": "MIT", "authors": [], "manifest_path": str(folder / "Cargo.toml")}
            result = subprocess.CompletedProcess([], 0, stdout=json.dumps({"packages": [package], "resolve": {"nodes": [{"id": "fixture"}]}}))
            with patch.object(module, "command", return_value=result):
                self.assertIn("required upstream attribution", module.dependency_notices("aarch64-apple-darwin"))
                self.assertIn("embedded component permission", module.dependency_notices("aarch64-apple-darwin"))
                (folder / "LICENSE").unlink()
                with self.assertRaisesRegex(ValueError, "licence text"):
                    module.dependency_notices("aarch64-apple-darwin")
                (folder / "AUTHORS").write_text("Permission is hereby granted; copyright fixture authors")
                self.assertIn("copyright fixture authors", module.dependency_notices("aarch64-apple-darwin"))

    def test_linux_cli_accepts_choice_and_reports_unsupported_host(self):
        spec = importlib.util.spec_from_file_location("package", Path(__file__).with_name("package.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory) / "linux.zip"
            with patch.object(sys, "argv", ["package.py", "--platform", "linux", "--output", str(output)]), patch.object(module, "build", side_effect=ValueError("requires native Linux GNU")) as build, patch("sys.stderr") as stderr:
                with self.assertRaises(SystemExit) as failure:
                    module.main()
                self.assertEqual(failure.exception.code, 1)
                build.assert_called_once_with("linux")
                self.assertIn("Packaging failed: requires native Linux GNU", "".join(call.args[0] for call in stderr.write.call_args_list))
            self.assertFalse(output.exists())

    def test_linux_archive_permissions_hashes_and_preservation(self):
        spec = importlib.util.spec_from_file_location("package", Path(__file__).with_name("package.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "app"
            binary.write_bytes(b"linux fixture")
            binary.chmod(0o600)
            worker = root / "worker"
            worker.write_bytes(b"worker fixture")
            worker.chmod(0o600)
            output = root / "linux.zip"
            with patch.object(module, "DOCUMENTS", ()):
                module.archive("linux", output, binary, worker, root, "notices")
            with zipfile.ZipFile(output) as package:
                self.assertEqual(set(package.namelist()), {"Tessera/tessera", "Tessera/tessera-workbook-worker", "Tessera/THIRD-PARTY-NOTICES.txt", "Tessera/BUILD-INFO.json", "Tessera/SHA256SUMS.json"})
                self.assertEqual(package.getinfo("Tessera/tessera").external_attr >> 16 & 0o777, 0o755)
                self.assertEqual(package.getinfo("Tessera/tessera-workbook-worker").external_attr >> 16 & 0o777, 0o755)
                self.assertEqual(package.read("Tessera/tessera-workbook-worker"), worker.read_bytes())
                identities = json.loads(package.read("Tessera/SHA256SUMS.json"))
                self.assertEqual(set(identities), {"tessera", "tessera-workbook-worker", "THIRD-PARTY-NOTICES.txt", "BUILD-INFO.json"})
                for name, digest in identities.items():
                    self.assertEqual(hashlib.sha256(package.read("Tessera/" + name)).hexdigest(), digest)
            original = output.read_bytes()
            with self.assertRaises(FileExistsError):
                module.archive("linux", output, binary, worker, root, "replacement")
            self.assertEqual(output.read_bytes(), original)

    def test_linux_archive_modes_do_not_depend_on_packaging_host(self):
        spec = importlib.util.spec_from_file_location("package", Path(__file__).with_name("package.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        from_file = zipfile.ZipInfo.from_file

        def windows_metadata(*args, **kwargs):
            info = from_file(*args, **kwargs)
            info.create_system = 0
            info.external_attr = 0o100666 << 16
            return info

        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "app"
            binary.write_bytes(b"executable")
            output = root / "linux.zip"
            with patch.object(module, "DOCUMENTS", ()), patch.object(zipfile.ZipInfo, "from_file", windows_metadata):
                module.archive("linux", output, binary, binary, root, "notices")
            with zipfile.ZipFile(output) as package:
                for name in ("tessera", "tessera-workbook-worker"):
                    info = package.getinfo("Tessera/" + name)
                    self.assertEqual(info.create_system, 3)
                    self.assertEqual(info.external_attr >> 16, 0o100755)

    def test_linux_build_uses_supported_native_host_only(self):
        spec = importlib.util.spec_from_file_location("package", Path(__file__).with_name("package.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        for host in ("x86_64-unknown-linux-gnu", "aarch64-unknown-linux-gnu", "aarch64-apple-darwin", "x86_64-unknown-linux-musl"):
            calls = []
            def fake_command(args, **kwargs):
                calls.append(args)
                if args[0] == "cargo":
                    self.assertEqual(kwargs["env"]["CARGO_TARGET_DIR"], str(module.ROOT / "target"))
                return subprocess.CompletedProcess(args, 0, stdout=f"rustc 1.97\nhost: {host}\n")
            with self.subTest(host=host), patch.object(module, "command", fake_command), patch.object(module, "dependency_notices", return_value="notices") as notices:
                if host.endswith("linux-gnu"):
                    binary, worker, text = module.build("linux")
                    self.assertEqual(binary, module.ROOT / "target" / host / "release/tessera")
                    self.assertEqual(worker, module.ROOT / "target" / host / "release/tessera-workbook-worker")
                    self.assertEqual(text, "notices")
                    self.assertEqual(calls, [["rustc", "-vV"], ["cargo", "build", "--offline", "--locked", "--release", "--bin", "tessera", "--target", host], ["cargo", "build", "--manifest-path", str(module.ROOT / "crates/tessera-workbook/Cargo.toml"), "--offline", "--locked", "--release", "--bin", "tessera-workbook-worker", "--target", host]])
                    notices.assert_called_once_with(host)
                else:
                    with self.assertRaisesRegex(ValueError, "native.*Linux GNU"):
                        module.build("linux")
                    self.assertEqual(calls, [["rustc", "-vV"]])
                    notices.assert_not_called()

    def test_allowlisted_package_and_existing_destination(self):
        spec = importlib.util.spec_from_file_location("package", Path(__file__).with_name("package.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            for relative in module.DOCUMENTS:
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text("fixture")
            binary = root / "tessera.exe"
            binary.write_bytes(b"fixture executable")
            worker = root / "worker"
            worker.write_bytes(b"worker fixture")
            excluded = root / "unrelated-private.txt"
            excluded.write_text("do not package")
            (root / "examples/perf_probe.rs").write_text("development only")
            (root / "scripts").mkdir()
            (root / "scripts/reference_inference.py").write_text("reference only")
            (root / "crates/tessera-workbook/src").mkdir(parents=True)
            (root / "crates/tessera-workbook/src/main.rs").write_text("worker source")
            (root / "workbook-proof").write_bytes(b"proof binary")
            (root / "tests").mkdir()
            (root / "tests/workbook.rs").write_text("tests")
            output = root / "release.zip"
            module.archive("windows", output, binary, worker, root, "dependency notices")
            with zipfile.ZipFile(output) as package:
                names = package.namelist()
                self.assertIn("Tessera/tessera.exe", names)
                self.assertEqual(package.read("Tessera/tessera-workbook-worker.exe"), worker.read_bytes())
                self.assertIn("Tessera/docs/project.schema.json", names)
                self.assertIn("Tessera/docs/project-format.md", names)
                self.assertIn("Tessera/LICENSE", names)
                self.assertIn("Tessera/docs/LICENSING.md", names)
                info = json.loads(package.read("Tessera/BUILD-INFO.json"))
                self.assertEqual(info["source"]["files"]["LICENSE"], hashlib.sha256(b"fixture").hexdigest())
                self.assertEqual(info["distribution_status"], "local-development-not-production-approved")
                self.assertIn("Tessera/assets/fonts/LICENSE", names)
                for document in ("examples/sample.tessera", "examples/mixed-analysis.tessera", "examples/exploration.tessera", "examples/regression-analysis.tessera"):
                    self.assertIn("Tessera/" + document, names)
                self.assertFalse(any("project-format-v" in name or "tests/fixtures/" in name or "verification/" in name for name in names))
                self.assertEqual(module.VERSION, "0.3.0")
                self.assertEqual(module.BUILD_NUMBER, module.VERSION)
                self.assertFalse(any("perf_probe" in name or "reference" in name or "workbook-proof" in name or "crates/" in name or "tests/" in name for name in names))
                expected = {"Tessera/" + name for name in module.DOCUMENTS}
                expected.update({"Tessera/tessera.exe", "Tessera/tessera-workbook-worker.exe", "Tessera/THIRD-PARTY-NOTICES.txt", "Tessera/BUILD-INFO.json", "Tessera/SHA256SUMS.json"})
                self.assertEqual(set(names), expected)
                self.assertEqual(len(names), len(expected))
                identities = json.loads(package.read("Tessera/SHA256SUMS.json"))
                self.assertEqual(set(identities), {name.removeprefix("Tessera/") for name in names if not name.endswith("SHA256SUMS.json")})
                for name, digest in identities.items():
                    self.assertEqual(hashlib.sha256(package.read("Tessera/" + name)).hexdigest(), digest)
                self.assertFalse(any("unrelated-private" in name for name in names))
                self.assertEqual(package.read("Tessera/THIRD-PARTY-NOTICES.txt"), b"dependency notices")
            original = output.read_bytes()
            with self.assertRaises(FileExistsError):
                module.archive("windows", output, binary, worker, root, "replacement")
            self.assertEqual(output.read_bytes(), original)

    def test_macos_brand_identity_and_icon(self):
        spec = importlib.util.spec_from_file_location("package", Path(__file__).with_name("package.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            icon = root / "assets/tessera-icon.png"
            icon.parent.mkdir()
            icon.write_bytes(b"brand icon")
            (root / "LICENSE").write_text("application licence")
            (root / "docs").mkdir()
            (root / "docs/LICENSING.md").write_text("distribution notice")
            binary = root / "tessera"
            binary.write_bytes(b"fixture executable")
            worker = root / "worker"
            worker.write_bytes(b"worker fixture")

            signs = []
            def native_command(args, **kwargs):
                if args[0] == "sips":
                    self.assertEqual(Path(args[4]), icon)
                    Path(args[6]).write_bytes(b"scaled icon")
                elif args[0] == "iconutil":
                    Path(args[5]).write_bytes(b"icns fixture")
                elif args[0] == "codesign":
                    signs.append(Path(args[-1]))
                else:
                    self.fail(f"Unexpected command: {args}")

            output = root / "macos.zip"
            with patch.object(module, "DOCUMENTS", ()), patch.object(module, "command", native_command):
                module.archive("macos", output, binary, worker, root, "notices")
            self.assertEqual([path.name for path in signs], ["tessera-workbook-worker", "Tessera.app"])
            with zipfile.ZipFile(output) as package:
                prefix = "Tessera/Tessera.app/Contents/"
                metadata = plistlib.loads(package.read(prefix + "Info.plist"))
                self.assertEqual(metadata["CFBundleName"], "Tessera")
                self.assertEqual(metadata["CFBundleDisplayName"], "Tessera")
                self.assertEqual(metadata["CFBundleIdentifier"], "com.thornandreason.tessera")
                self.assertEqual(metadata["CFBundleExecutable"], "Tessera")
                self.assertEqual(metadata["CFBundleIconFile"], "Tessera.icns")
                self.assertEqual(package.read(prefix + "MacOS/Tessera"), binary.read_bytes())
                self.assertEqual(package.read(prefix + "MacOS/tessera-workbook-worker"), worker.read_bytes())
                for name in ("Tessera", "tessera-workbook-worker"):
                    self.assertEqual(package.getinfo(prefix + "MacOS/" + name).external_attr >> 16 & 0o777, 0o755)
                identities = json.loads(package.read("Tessera/SHA256SUMS.json"))
                for name, digest in identities.items():
                    self.assertEqual(hashlib.sha256(package.read("Tessera/" + name)).hexdigest(), digest)
                self.assertEqual(package.read(prefix + "Resources/tessera-icon.png"), icon.read_bytes())
                self.assertEqual(package.read(prefix + "Resources/Tessera.icns"), b"icns fixture")
                self.assertEqual(package.read(prefix + "Resources/LICENSE"), b"application licence")
                self.assertEqual(package.read(prefix + "Resources/LICENSING.md"), b"distribution notice")
                self.assertEqual(package.read(prefix + "Resources/THIRD-PARTY-NOTICES.txt"), b"notices")

    def test_windows_and_macos_build_both_executables(self):
        spec = importlib.util.spec_from_file_location("package", Path(__file__).with_name("package.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory, patch.object(module, "ROOT", Path(directory)):
            for platform, targets in (("windows", ("x86_64-pc-windows-gnu",)),
                                      ("macos", ("aarch64-apple-darwin", "x86_64-apple-darwin"))):
                calls = []
                def native_command(args, **kwargs):
                    calls.append(args)
                    if args[0] == "cargo":
                        self.assertEqual(kwargs["env"]["CARGO_TARGET_DIR"], str(module.ROOT / "target"))
                    return subprocess.CompletedProcess(args, 0)
                with self.subTest(platform=platform), patch.object(module, "command", native_command), patch.object(module, "dependency_notices", return_value="notices") as dependency_notices:
                    binary, worker, notices = module.build(platform)
                    self.assertEqual(notices, "notices")
                    dependency_notices.assert_called_once_with(*targets)
                    builds = [args for args in calls if args[0] == "cargo"]
                    self.assertEqual(len(builds), 2 * len(targets))
                    for target in targets:
                        helper = next(args for args in builds if "tessera-workbook-worker" in args and args[-1] == target)
                        self.assertEqual(helper, ["cargo", "build", "--manifest-path", str(module.ROOT / "crates/tessera-workbook/Cargo.toml"), "--offline", "--locked", "--release", "--bin", "tessera-workbook-worker", "--target", target])
                    if platform == "windows":
                        self.assertEqual(worker.name, "tessera-workbook-worker.exe")
                    else:
                        lipos = [args for args in calls if args[0] == "lipo"]
                        self.assertEqual(len(lipos), 2)
                        for name, destination in (("tessera", binary), ("tessera-workbook-worker", worker)):
                            self.assertIn(["lipo", "-create", *(str(module.ROOT / "target" / target / "release" / name) for target in targets), "-output", str(destination)], lipos)
                        self.assertIn(["codesign", "--force", "--sign", "-", str(worker)], calls)

    def test_notices_union_independent_worker_resolution_without_duplicates(self):
        spec = importlib.util.spec_from_file_location("package", Path(__file__).with_name("package.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            folder = root / "dependency"
            folder.mkdir()
            (folder / "LICENSE").write_text("fixture license")
            def dependency(name, version):
                return {"id": name + version, "name": name, "version": version,
                        "source": "registry+fixture", "license": "MIT", "authors": [],
                        "manifest_path": str(folder / "Cargo.toml")}
            shared = dependency("shared", "1.0")
            worker_only = dependency("worker-only", "2.0")
            old_syn = dependency("syn", "2.0")
            new_syn = dependency("syn", "3.0")
            calls = []
            def metadata(args, **kwargs):
                calls.append(args)
                packages = [shared, worker_only, new_syn] if "tessera-workbook" in args[3] else [shared, old_syn]
                content = {"packages": packages, "resolve": {"nodes": [{"id": package["id"]} for package in packages]}}
                return subprocess.CompletedProcess(args, 0, stdout=json.dumps(content))
            with patch.object(module, "ROOT", root), patch.object(module, "command", metadata):
                notices = module.dependency_notices("x86_64-pc-windows-gnu")
            self.assertEqual(len(calls), 2)
            self.assertIn(str(root / "crates/tessera-workbook/Cargo.toml"), calls[1])
            self.assertTrue(all("--locked" in args and "--offline" in args for args in calls))
            for name in ("shared 1.0", "worker-only 2.0", "syn 2.0", "syn 3.0"):
                self.assertEqual(notices.count("\n" + name + "\n"), 1)

    def test_universal_mac_notices_include_intel_only_dependencies(self):
        spec = importlib.util.spec_from_file_location("package", Path(__file__).with_name("package.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            folder = root / "dependency"
            folder.mkdir()
            (folder / "LICENSE").write_text("fixture license")
            def dependency(name, version):
                return {"id": name + version, "name": name, "version": version,
                        "source": "registry+fixture", "license": "MIT", "authors": [],
                        "manifest_path": str(folder / "Cargo.toml")}
            shared = dependency("shared", "1.0")
            intel_only = dependency("core_detect", "1.0.0")
            worker_only = dependency("worker-only", "2.0")
            old_syn = dependency("syn", "2.0")
            new_syn = dependency("syn", "3.0")
            calls = []
            def metadata(args, **kwargs):
                calls.append((args[3], args[-1]))
                packages = [shared, worker_only, new_syn] if "tessera-workbook" in args[3] else [shared, old_syn]
                if args[-1] == "x86_64-apple-darwin":
                    packages = [*packages, intel_only]
                content = {"packages": packages, "resolve": {"nodes": [{"id": package["id"]} for package in packages]}}
                return subprocess.CompletedProcess(args, 0, stdout=json.dumps(content))
            targets = ("aarch64-apple-darwin", "x86_64-apple-darwin")
            with patch.object(module, "ROOT", root), patch.object(module, "command", metadata):
                notices = module.dependency_notices(*targets)
            self.assertEqual(set(calls), {(str(root / manifest), target)
                                        for manifest in ("Cargo.toml", "crates/tessera-workbook/Cargo.toml")
                                        for target in targets})
            self.assertEqual(len(calls), 4)
            for name in ("shared 1.0", "core_detect 1.0.0", "worker-only 2.0", "syn 2.0", "syn 3.0"):
                self.assertEqual(notices.count("\n" + name + "\n"), 1)

    def test_missing_worker_fails_before_creation_and_preserves_existing_output(self):
        spec = importlib.util.spec_from_file_location("package", Path(__file__).with_name("package.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "app"
            binary.write_bytes(b"binary")
            missing = root / "missing-worker"
            for platform in ("macos", "windows", "linux"):
                output = root / platform / "release.zip"
                with self.subTest(platform=platform), self.assertRaises(FileNotFoundError):
                    module.archive(platform, output, binary, missing, root, "notices")
                self.assertFalse(output.parent.exists())
            output = root / "existing.zip"
            output.write_bytes(b"preserve")
            with self.assertRaises(FileExistsError):
                module.archive("linux", output, binary, missing, root, "notices")
            self.assertEqual(output.read_bytes(), b"preserve")

    def test_unsafe_allowlist_is_refused_before_archive_creation(self):
        spec = importlib.util.spec_from_file_location("package", Path(__file__).with_name("package.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            binary = root / "app.exe"
            binary.write_bytes(b"binary")
            worker = root / "worker"
            worker.write_bytes(b"worker fixture")
            for documents in [("../private.txt",), ("/private.txt",), ("README.md", "README.md"), ("C:\\private.txt",), ("docs//README.md",), ("",)]:
                module.DOCUMENTS = documents
                with self.subTest(documents=documents), self.assertRaises(ValueError):
                    module.archive("windows", root / "unsafe.zip", binary, worker, root, "notices")
                self.assertFalse((root / "unsafe.zip").exists())
            outside = root / "outside.txt"
            outside.write_text("private")
            product = root / "product"
            product.mkdir()
            (product / "README.md").symlink_to(outside)
            module.DOCUMENTS = ("README.md",)
            with self.assertRaises(ValueError):
                module.archive("windows", root / "symlink.zip", binary, worker, product, "notices")
            self.assertFalse((root / "symlink.zip").exists())


if __name__ == "__main__":
    unittest.main()
