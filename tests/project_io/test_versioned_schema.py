"""Run with PYTHONPATH=.build/schema-tools python3 tests/project_io/test_versioned_schema.py -v."""
import copy
import hashlib
import json
import unittest
from pathlib import Path

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[2]
TAGS = ("describe", "pearson", "one_sample_t", "paired_t", "independent_t")
MAX_CELLS = 2_000_000
MAX_FREQUENCY = 2**53
# Curated structural reference identities; original development snapshots are private.
CURATED_V1_IDENTITIES = {
    "docs/project-v1.schema.json": "158ec6e130aabfe6706e3eba617477e09da82bbd1b56d0b192e98946ea2aa51b",
    "tests/fixtures/pre-tessera-format/sample.json": "5c2875344fae2dc1baa1bb6e4ce16404ae44cbc11daf6013dac495da85507337",
}


class VersionedSchemaTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        schema = json.loads((ROOT / "docs/project.schema.json").read_text())
        Draft202012Validator.check_schema(schema)
        cls.validator = Draft202012Validator(schema)
        cls.example = json.loads((ROOT / "examples/mixed-analysis.tessera").read_text())

    def payload(self, document, index):
        return document["runs"][index]["analysis"][TAGS[index]]

    def assert_valid(self, document):
        errors = list(self.validator.iter_errors(document))
        self.assertFalse(errors, "\n".join(error.message for error in errors))

    def test_no_frequency_role_requires_zero_weight_diagnostics_for_all_five_methods(self):
        for index, tag in enumerate(TAGS):
            for field in ("missing_weight_rows", "zero_weight_rows"):
                with self.subTest(method=tag, diagnostic=field):
                    document = copy.deepcopy(self.example)
                    self.payload(document, index)["diagnostics"][field] = 1
                    self.assertFalse(self.validator.is_valid(document))

    def test_each_new_unweighted_n_has_the_source_cell_ceiling(self):
        paths = (
            (1, "rows", 0),
            (2, "sample", None),
            (3, "first", None),
            (3, "second", None),
            (4, "groups", 0),
            (4, "groups", 1),
        )
        for index, field, position in paths:
            with self.subTest(method=TAGS[index], field=field, position=position):
                document = copy.deepcopy(self.example)
                result = self.payload(document, index)[field]
                if position is not None:
                    result = result[position]
                result["n"] = MAX_CELLS
                self.assert_valid(document)
                result["n"] += 1
                self.assertFalse(self.validator.is_valid(document))

    def test_weighted_large_n_controls_keep_frequency_semantics(self):
        document = copy.deepcopy(self.example)
        for index in range(5):
            payload = self.payload(document, index)
            # Column 1 is numeric and already captured in every method's role snapshots.
            payload["request"]["frequency_weight"] = "1"
            payload["diagnostics"]["missing_weight_rows"] = 1
            payload["diagnostics"]["zero_weight_rows"] = 1
        self.payload(document, 1)["rows"][0]["n"] = MAX_FREQUENCY
        self.payload(document, 2)["sample"]["n"] = MAX_FREQUENCY
        self.payload(document, 2)["statistics"]["df"]["value"] = MAX_FREQUENCY - 1
        for field in ("first", "second"):
            self.payload(document, 3)[field]["n"] = MAX_FREQUENCY
        self.payload(document, 3)["statistics"]["df"]["value"] = MAX_FREQUENCY - 1
        for group in self.payload(document, 4)["groups"]:
            group["n"] = MAX_FREQUENCY // 2
        self.payload(document, 4)["statistics"]["df"]["value"] = MAX_FREQUENCY - 2
        self.assert_valid(document)
        self.payload(document, 1)["rows"][0]["n"] = MAX_FREQUENCY + 1
        self.assertFalse(self.validator.is_valid(document))

    def test_combined_source_arithmetic_is_a_shared_semantic_check(self):
        document = copy.deepcopy(self.example)
        for group in self.payload(document, 4)["groups"]:
            group["n"] = MAX_CELLS // 2
        self.payload(document, 4)["diagnostics"]["filtered_rows"] = 1
        self.payload(document, 4)["statistics"]["df"]["value"] = MAX_CELLS // 2 - 1
        # Individual fields fit the schema. The shared model rejects their over-limit sum.
        self.assert_valid(document)

    def test_current_examples_and_curated_reference_bytes(self):
        for example in ("sample.tessera", "mixed-analysis.tessera"):
            document = json.loads((ROOT / "examples" / example).read_text())
            self.validator.validate(document)
        for path, digest in CURATED_V1_IDENTITIES.items():
            self.assertEqual(hashlib.sha256((ROOT / path).read_bytes()).hexdigest(), digest)


if __name__ == "__main__":
    unittest.main()
