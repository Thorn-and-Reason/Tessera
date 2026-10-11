import importlib.util
import tempfile
import subprocess
from unittest.mock import patch
import unittest
from pathlib import Path


class CoverageTests(unittest.TestCase):
    def test_platform_gated_test_blocks_are_excluded(self):
        spec = importlib.util.spec_from_file_location("coverage", Path(__file__).with_name("coverage.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        lines = ["fn runtime() {}", "#[cfg(all(test, unix))]", "mod tests {", "fn check() {}", "}"]
        self.assertEqual(module.test_lines(lines), {2, 3, 4, 5})
        self.assertEqual(module.test_lines(["#[cfg(any(test, feature = \"desktop\"))]", "fn runtime() {}"]), set())

    def test_workbook_worker_is_production_and_its_checks_are_excluded(self):
        spec = importlib.util.spec_from_file_location("coverage", Path(__file__).with_name("coverage.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            files = {
                "src/lib.rs": "fn app() {}\n",
                "crates/tessera-workbook/src/lib.rs": "mod worker;\n#[cfg(test)]\nmod checks;\n",
                "crates/tessera-workbook/src/main.rs": "fn main() {}\n",
                "crates/tessera-workbook/src/worker.rs": "pub fn parse() {}\n",
                "crates/tessera-workbook/src/checks.rs": "fn test_only() {}\n",
            }
            for name, content in files.items():
                path = root / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(content)
            lcov = "".join(f"SF:{root / name}\nDA:1,1\nend_of_record\n" for name in files
                           if name.endswith(("worker.rs", "checks.rs")) or name == "src/lib.rs")
            result = module.summarize(lcov, root)
        self.assertEqual((result["covered"], result["total"]), (2, 2))
        self.assertEqual({row["file"] for row in result["files"]}, {
            "src/lib.rs", "crates/tessera-workbook/src/worker.rs",
        })
        self.assertEqual(result["excluded_test_files"], ["crates/tessera-workbook/src/checks.rs"])

    def test_distinct_lines_and_test_only_blocks(self):
        spec = importlib.util.spec_from_file_location("coverage", Path(__file__).with_name("coverage.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            source = root / "src/main.rs"
            source.parent.mkdir()
            source.write_text("fn main() {}\n#[cfg(test)]\nmod tests {\nfn check() {}\n}\nfn other() {}\n")
            (source.parent / "windows.rs").write_text("fn platform_only() {}\n")
            lcov = f"SF:{source}\nDA:1,0\nDA:4,5\nDA:6,0\nend_of_record\nSF:{source}\nDA:1,2\nDA:4,0\nend_of_record\n"
            result = module.summarize(lcov, root)
        self.assertEqual(result["covered"], 1)
        self.assertEqual(result["total"], 2)
        self.assertEqual(result["percent"], 50.0)
        self.assertEqual(result["not_instrumented_files"], ["src/windows.rs"])


    def test_external_test_modules_follow_rust_module_ownership(self):
        spec = importlib.util.spec_from_file_location("coverage", Path(__file__).with_name("coverage.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            files = {
                "lib.rs": 'mod runtime;\n#[path = "shared.rs"]\nmod live;\n#[cfg(test)]\nmod checks;\n#[cfg(test)]\n#[path = "fixtures/custom.rs"]\npub(crate) mod controls;\n#[cfg(test)]\nmod inline {\n    mod child;\n}\n',
                "runtime.rs": 'pub fn runtime() {}\n',
                "shared.rs": 'pub fn shared() {}\n',
                "checks.rs": 'mod nested;\n#[path = "shared.rs"]\nmod shared;\nfn check() {}\n',
                "checks/nested.rs": 'fn nested() {}\n',
                "fixtures/custom.rs": '#[path = "nested_fixture.rs"]\nmod fixture;\nfn custom() {}\n',
                "fixtures/nested_fixture.rs": 'fn fixture() {}\n',
                "inline/child.rs": 'fn child() {}\n',
                "windows.rs": 'fn platform() {}\n',
            }
            for name, content in files.items():
                path = root / "src" / name
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(content)
            lcov = "".join(f"SF:{root / 'src' / name}\nDA:{len(content.splitlines())},1\nend_of_record\n"
                           for name, content in files.items() if name not in ("lib.rs", "windows.rs"))
            result = module.summarize(lcov, root)
        self.assertEqual((result["covered"], result["total"]), (2, 2))
        self.assertEqual({row["file"] for row in result["files"]}, {"src/runtime.rs", "src/shared.rs"})
        self.assertEqual(set(result["excluded_test_files"]), {
            "src/checks.rs", "src/checks/nested.rs", "src/fixtures/custom.rs",
            "src/fixtures/nested_fixture.rs", "src/inline/child.rs",
        })
        self.assertEqual(result["not_instrumented_files"], ["src/lib.rs", "src/windows.rs"])

    def test_failed_test_run_keeps_diagnostics(self):
        spec = importlib.util.spec_from_file_location("coverage", Path(__file__).with_name("coverage.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory:
            output = Path(directory)
            error = subprocess.CalledProcessError(101, ["cargo", "test"], output="failed test output", stderr="failure details\n")
            with patch("sys.argv", ["coverage.py", "--output", str(output)]), patch.object(module, "command", side_effect=error):
                with self.assertRaises(subprocess.CalledProcessError):
                    module.main()
            self.assertEqual((output / "test-run.log").read_text(), "failure details\nfailed test output")


if __name__ == "__main__":
    unittest.main()
