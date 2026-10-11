import importlib.util
import tempfile
import subprocess
import sys
import unittest
from unittest.mock import patch
from pathlib import Path


class MeasurementTests(unittest.TestCase):
    def setUp(self):
        directory = Path(self.enterContext(tempfile.TemporaryDirectory()))
        self.success_binary = directory / "success.py"
        self.failure_binary = directory / "failure.py"
        self.success_binary.write_text("raise SystemExit(0)\n")
        self.failure_binary.write_text("raise SystemExit(1)\n")
        actual = subprocess.Popen
        probes = {str(self.success_binary.resolve()), str(self.failure_binary.resolve())}

        def launch(args, **kwargs):
            if str(Path(args[0]).resolve()) in probes:
                args = [sys.executable, *args]
            return actual(args, **kwargs)

        self.enterContext(patch.object(subprocess, "Popen", side_effect=launch))

    def test_failed_trials_cannot_pass_targets(self):
        spec = importlib.util.spec_from_file_location("measure", Path(__file__).with_name("measure.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory:
            fixture = Path(directory) / "data.csv"
            fixture.write_text("A\n1\n")
            module.HISTORICAL_INSPECT_SHA256 = module.sha256(fixture)
            result = module.measure(self.failure_binary, fixture, 5, "inspect")
        self.assertEqual(result["successful_trials"], 0)
        self.assertIsNone(result["median_wall_ms"])
        self.assertFalse(result["targets_pass"])
        self.assertTrue(all(trial["exit"] != 0 for trial in result["trials"]))

    def test_pearson_trials_and_any_failure_reject_success(self):
        spec = importlib.util.spec_from_file_location("measure", Path(__file__).with_name("measure.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory:
            fixture = Path(directory) / "data.csv"
            fixture.write_text("A,B\n1,2\n")
            result = module.measure(self.success_binary, fixture, 5, "pearson")
            self.assertEqual(result["successful_trials"], 5)
            self.assertTrue(result["all_trials_succeeded"])
            failed = module.measure(self.failure_binary, fixture, 5, "pearson")
            self.assertFalse(failed["all_trials_succeeded"])

    def test_one_failed_trial_rejects_other_successful_trials(self):
        spec = importlib.util.spec_from_file_location("measure", Path(__file__).with_name("measure.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory:
            fixture = Path(directory) / "data.csv"
            fixture.write_text("A\n1\n")
            module.HISTORICAL_INSPECT_SHA256 = module.sha256(fixture)
            actual = module.subprocess.Popen
            count = 0
            def launch(args, **kwargs):
                nonlocal count
                count += 1
                return actual([str(self.failure_binary if count == 6 else self.success_binary)], **kwargs)
            with patch.object(module.subprocess, "Popen", side_effect=launch):
                result = module.measure(self.success_binary, fixture, 6, "inspect")
            self.assertEqual(count, 6)
            self.assertEqual([trial["exit"] for trial in result["trials"]], [0, 0, 0, 0, 0, 1])
            self.assertEqual(result["successful_trials"], 5)
            self.assertFalse(result["all_trials_succeeded"])
            self.assertFalse(result["targets_pass"])

    def test_chart_operations_retain_failed_trials_without_import_targets(self):
        spec = importlib.util.spec_from_file_location("measure", Path(__file__).with_name("measure.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory:
            fixture = Path(directory) / "data.csv"
            fixture.write_text("X,Y,Group\n1,2,A\n")
            for operation in ("frequencies", "histogram", "scatter", "boxplot"):
                with self.subTest(operation=operation):
                    result = module.measure(self.failure_binary, fixture, 5, operation)
                    self.assertEqual(len(result["trials"]), 5)
                    self.assertFalse(result["all_trials_succeeded"])
                    self.assertIsNone(result["targets_pass"])
                    self.assertIsNone(result["targets"])

    def test_extended_operations_and_stage_metrics(self):
        spec = importlib.util.spec_from_file_location("measure", Path(__file__).with_name("measure.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        expected = {"one-sample-t", "paired-t", "independent-t", "regression", "reliability",
                    "anova-one-way", "anova-factorial", "anova-repeated", "load", "save", "export", "edit", "groups"}
        self.assertTrue(expected.issubset(module.OPERATIONS))
        self.assertEqual(module.parse_metrics("stage_read_ms=1.25\nretained_project_bytes=42\n"),
                         {"stage_ms": {"read": 1.25}, "retained_bytes": {"project": 42}})
        self.assertEqual(module.parse_metrics("stage_read_ms=nan\nretained_project_bytes=-1\n"),
                         {"stage_ms": {}, "retained_bytes": {}})

    def test_unrelated_fixture_has_no_historical_target(self):
        spec = importlib.util.spec_from_file_location("measure", Path(__file__).with_name("measure.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory:
            fixture = Path(directory) / "other.csv"
            fixture.write_text("A\n1\n")
            result = module.measure(self.success_binary, fixture, 5, "inspect")
            self.assertFalse(result["targets_applicable"])
            self.assertIsNone(result["targets"])
            self.assertIsNone(result["targets_pass"])

    def test_lifecycle_operations_use_independent_trials(self):
        spec = importlib.util.spec_from_file_location("measure", Path(__file__).with_name("measure.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with tempfile.TemporaryDirectory() as directory:
            fixture = Path(directory) / "data.csv"
            fixture.write_text("X,Y\n1,2\n")
            for operation in ("cancel", "shutdown", "save-load"):
                with self.subTest(operation=operation):
                    result = module.measure(self.success_binary, fixture, 5, operation)
                    self.assertEqual(result["successful_trials"], 5)
                    self.assertFalse(result["targets_applicable"])
                    self.assertIsNone(result["targets_pass"])

    def test_trial_minimum_is_enforced(self):
        spec = importlib.util.spec_from_file_location("measure", Path(__file__).with_name("measure.py"))
        module = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(module)
        with self.assertRaises(ValueError):
            module.measure(self.failure_binary, Path("missing"), 4, "inspect")


if __name__ == "__main__":
    unittest.main()
