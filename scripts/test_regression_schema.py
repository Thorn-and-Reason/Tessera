"""Current regression schema checks with historical payload evidence.

Use the existing jsonschema library; no package installation is part of this script.
The fixtures below exercise shapes, not solver correctness or real encoded output.
"""
import copy
import hashlib
import json
import unittest
from pathlib import Path

from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[1]
CURATED_IDENTITIES = {
    "docs/project-v1.schema.json": "158ec6e130aabfe6706e3eba617477e09da82bbd1b56d0b192e98946ea2aa51b",
    "docs/project-v2.schema.json": "102d913ec5cd04456fb5168643fbb032112fbe0e2d1eb6666aeeba8592a067e3",
    "docs/project-v3.schema.json": "ae286a20ac3d80cc79815fad708be0801f406e6695a0a7439b151a10a4f5f6aa",
    "tests/fixtures/pre-tessera-format/exploration.json": "407ed15e538bc7539b150482b53ef0a21ccea205775b5be389f261898f8f1845",
    "tests/fixtures/pre-tessera-format/mixed-analysis.json": "d08f39e3db767ad6a311d9d70c25e21839c8c40d4bef7e85e883250f22a5cd42",
    "tests/fixtures/pre-tessera-format/sample.json": "5c2875344fae2dc1baa1bb6e4ce16404ae44cbc11daf6013dac495da85507337"
}


def available(value=1.0):
    return {"value": value, "reason": None}


def unavailable(reason="Structural fixture: scalar unavailable."):
    return {"value": None, "reason": reason}


def structural_payload():
    """A typed-shape fixture only; it is not a Task 5 result or a numerical oracle."""
    coefficient = {"estimate": available(), "standard_error": available(),
                   "t": available(), "p": available(0.5),
                   "interval": [available(0), available(2)]}
    extent = {"range": [available(0), available(1)],
              "undefined_rows": 0, "undefined_frequency": 0}
    residual_point = {"cohort_ordinal": 0, "row": 0, "frequency": 1,
                      "fitted": available(), "residual": available(),
                      "leverage": available(0.5), "studentised": available(),
                      "cooks_distance": available(0.1)}
    qq_point = {"rank": 0, "row": 0, "frequency": 1,
                "theoretical": available(-1), "residual": available()}
    diagnostics = {
        "residual_mean": available(0), "residual_sample_sd": available(),
        "residual_range": [available(-1), available(1)],
        "fitted_range": [available(0), available(2)],
        "leverage": extent, "studentised": copy.deepcopy(extent),
        "cooks_distance": copy.deepcopy(extent),
        "residual_points": [residual_point], "residual_reduced": False,
        "qq_points": [qq_point], "qq_reduced": False,
        "theoretical_range": [available(-1), available(1)],
    }
    statistics = {name: available() for name in
                  ("sse", "sst", "model_ss", "residual_standard_error",
                   "r_squared", "adjusted_r_squared", "f")}
    statistics["f_p"] = available(0.5)
    fit = {"rank": 3, "model_df": 2, "residual_df": 1, "total_df": 3,
           "condition": available(2), "condition_warned": False,
           "outcome_scale": available(), "term_scales": [available()] * 3,
           "coefficients": [copy.deepcopy(coefficient) for _ in range(3)],
           "statistics": statistics, "residual_diagnostics": diagnostics}
    snapshots = [{"id": str(i), "name": name, "kind": kind,
                  "description": "", "missing_rules": "none", "value_labels": []}
                 for i, name, kind in [(1, "Outcome", "number"),
                                       (2, "Numeric", "number"),
                                       (3, "Category", "text")]]
    return {
        "revision": "1", "request": {
            "outcome": "1", "predictors": [
                {"column": "2", "role": "numeric"},
                {"column": "3", "role": {"categorical": {"reference": "A"}}}],
            "confidence": "95%", "filter": {"predicates": []},
            "frequency_weight": None,
        },
        "columns": snapshots, "source_rows": 4,
        "diagnostics": {"filtered_rows": 0, "missing_weight_rows": 0,
                        "zero_weight_rows": 0, "complete_case_rows": 0},
        "fitted_rows": 4, "frequency": 4, "incomplete_case_frequency": 0,
        "levels": [{"column": "3", "values": ["A", "B"]}],
        "terms": ["intercept", {"numeric": {"column": "2"}},
                  {"contrast": {"column": "3", "level": "B", "reference": "A"}}],
        "method": {"numerical_version": 1, "rank_threshold": 1e-12,
                   "condition_warning": 1e8}, "outcome": {"fitted": fit},
    }


class RegressionSchemaTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.schema = json.loads((ROOT / "docs/project.schema.json").read_text())
        Draft202012Validator.check_schema(cls.schema)
        cls.payload = structural_payload()

    def validator(self, definition):
        return Draft202012Validator({"$schema": self.schema["$schema"],
                                     "$defs": self.schema["$defs"],
                                     "$ref": "#/$defs/" + definition})

    def test_curated_reference_files_keep_their_bytes(self):
        self.assertTrue(CURATED_IDENTITIES)
        for path, expected in CURATED_IDENTITIES.items():
            self.assertEqual(hashlib.sha256((ROOT / path).read_bytes()).hexdigest(),
                             expected, path)

    def test_earlier_definitions_keep_their_contract(self):
        previous = json.loads((ROOT / "docs/project-v3.schema.json").read_text())
        for name, definition in previous["$defs"].items():
            if name != "completed":
                self.assertEqual(definition, self.schema["$defs"][name], name)
        self.assertEqual(previous["$defs"]["completed"]["oneOf"],
                         self.schema["$defs"]["completed"]["oneOf"][:9])

    def test_schema_only_project_wrapper_accepts_ten_families(self):
        document = json.loads((ROOT / "examples/exploration.tessera").read_text())
        document["version"] = 3
        document["runs"].append({"id": document["next_run_id"],
                                 "analysis": {"regression": self.payload}})
        document["next_run_id"] = str(int(document["next_run_id"]) + 1)
        Draft202012Validator(self.schema).validate(document)
        for version in (1, 2, 3):
            older = json.loads((ROOT / f"docs/project-v{version}.schema.json").read_text())
            document["version"] = version
            self.assertFalse(Draft202012Validator(older).is_valid(document))

    def test_every_new_record_is_closed_map_and_every_field_is_required(self):
        p = self.payload
        fit = p["outcome"]["fitted"]
        diagnostic = fit["residual_diagnostics"]
        failure = {"kind": "numerical", "reason": "Numerical fixture failure.",
                   "expected_terms": 3, "rank": None, "implicated_terms": []}
        records = {
            "regression_analysis": p, "regression_request": p["request"],
            "regression_predictor": p["request"]["predictors"][1],
            "regression_levels": p["levels"][0], "regression_method": p["method"],
            "regression_failure": failure, "regression_fit": fit,
            "regression_coefficient": fit["coefficients"][0],
            "regression_statistics": fit["statistics"],
            "regression_diagnostic_extent": diagnostic["leverage"],
            "regression_diagnostics": diagnostic,
            "regression_residual_point": diagnostic["residual_points"][0],
            "regression_qq_point": diagnostic["qq_points"][0],
        }
        for name, record in records.items():
            validator = self.validator(name)
            validator.validate(record)
            self.assertFalse(validator.is_valid(list(record.values())), name)
            bad = copy.deepcopy(record)
            bad["unexpected"] = 0
            self.assertFalse(validator.is_valid(bad), name)
            for field in record:
                with self.subTest(record=name, omitted=field):
                    bad = copy.deepcopy(record)
                    del bad[field]
                    self.assertFalse(validator.is_valid(bad))

    def test_all_discriminated_variants_are_single_tag_objects_or_unit_strings(self):
        variants = {
            "regression_predictor_role": ["numeric", {"categorical": {"reference": ""}},
                                           {"categorical": {"reference": 0}}],
            "regression_term": self.payload["terms"],
            "regression_failure_kind": ["empty_cohort", "unidentified", "nonconvergence",
                                         "numerical", {"absent_reference": {"column": "3"}},
                                         {"no_contrast": {"column": "3"}}],
            "regression_outcome": [self.payload["outcome"], {"unavailable": {
                "kind": "empty_cohort", "reason": "Empty fixture cohort.",
                "expected_terms": None, "rank": None, "implicated_terms": []}}],
        }
        for name, values in variants.items():
            validator = self.validator(name)
            for value in values:
                validator.validate(value)
                if isinstance(value, dict):
                    tag, record = next(iter(value.items()))
                    for field in record:
                        bad = copy.deepcopy(value)
                        del bad[tag][field]
                        self.assertFalse(validator.is_valid(bad), (name, tag, field))
                    self.assertFalse(validator.is_valid({tag: list(record.values())}))
                    self.assertFalse(validator.is_valid({tag: {**record, "unknown": 0}}))
                    self.assertFalse(validator.is_valid({**value, "unknown": None}))
                else:
                    self.assertFalse(validator.is_valid({value: None}), name)
            for bad in ({}, {"unknown": {}}, ["numeric", None], "unknown"):
                self.assertFalse(validator.is_valid(bad), name)

    def test_actual_guarded_task5_captures_match_the_schema(self):
        fixture = ROOT / "tests/fixtures/regression-schema-captures.json"
        self.assertEqual(hashlib.sha256(fixture.read_bytes()).hexdigest(),
                         "11abd6fa97abd8044174c69e075685361c5772ba8b2a6afc461f017fdae1dfc4")
        records = json.loads(fixture.read_text())
        self.assertEqual(len(records), 84)
        self.assertEqual({r["confidence"] for r in records}, {"P90", "P95", "P99"})
        self.assertEqual(len({r["name"] for r in records}), 28)
        for record in records:
            with self.subTest(case=record["name"], confidence=record["confidence"]):
                self.validator("regression_analysis").validate(record["payload"])

    def test_early_failure_does_not_infer_a_complete_model_expansion(self):
        early = copy.deepcopy(self.payload)
        early["request"]["predictors"] = [
            {"column": str(i + 2), "role": {"categorical": {"reference": "A"}}}
            for i in range(70)]
        early.update({"fitted_rows": 0, "frequency": 0, "levels": [],
                      "terms": ["intercept"], "outcome": {"unavailable": {
            "kind": "empty_cohort", "reason": "No complete fixture cohort.",
            "expected_terms": None, "rank": None, "implicated_terms": []}}})
        self.validator("regression_analysis").validate(early)
        # Captured metadata completeness is a Rust semantic responsibility.
        early["levels"] = [{"column": str(i + 2), "values": ["A"]} for i in range(64)]
        self.validator("regression_analysis").validate(early)

    def test_typed_references_ids_and_method_constants(self):
        validator = self.validator("regression_request")
        for reference in (None, True, [], {}, float("inf")):
            bad = copy.deepcopy(self.payload["request"])
            bad["predictors"][1]["role"]["categorical"]["reference"] = reference
            self.assertFalse(validator.is_valid(bad), repr(reference))
        for identity in ("0", "01", "-1", "18446744073709551616", 1):
            bad = copy.deepcopy(self.payload["request"])
            bad["outcome"] = identity
            self.assertFalse(validator.is_valid(bad), identity)
        for field, value in (("numerical_version", 2), ("rank_threshold", 1e-10),
                             ("condition_warning", 1e12)):
            bad = {**self.payload["method"], field: value}
            self.assertFalse(self.validator("regression_method").is_valid(bad), field)

    def test_nullable_fields_and_independent_scalar_availability(self):
        fit = copy.deepcopy(self.payload["outcome"]["fitted"])
        for field in ("sse", "sst", "model_ss"):
            fit["statistics"][field] = unavailable("A nonzero estimate is below Float64 range.")
        self.validator("regression_fit").validate(fit)
        for field in ("sse", "sst", "model_ss"):
            for estimate in ({"value": None}, {"reason": None},
                             {"value": None, "reason": None},
                             {"value": 0, "reason": "Contradictory fixture."},
                             {"value": None, "reason": " "}):
                bad = copy.deepcopy(fit)
                bad["statistics"][field] = estimate
                self.assertFalse(self.validator("regression_fit").is_valid(bad))

    def test_cohort_point_and_term_local_bounds(self):
        validator = self.validator("regression_analysis")
        for field, value in (("source_rows", 2_000_001), ("frequency", 2**53 + 1),
                             ("incomplete_case_frequency", -1),
                             ("terms", ["intercept"] * 65)):
            bad = {**self.payload, field: value}
            self.assertFalse(validator.is_valid(bad), field)
        diagnostics = self.payload["outcome"]["fitted"]["residual_diagnostics"]
        for field in ("residual_points", "qq_points"):
            bad = copy.deepcopy(diagnostics)
            bad[field] *= 2001
            self.assertFalse(self.validator("regression_diagnostics").is_valid(bad), field)
        for definition, point in (("regression_residual_point", diagnostics["residual_points"][0]),
                                  ("regression_qq_point", diagnostics["qq_points"][0])):
            for field, value in (("row", 2_000_000), ("frequency", 0), ("frequency", 2**53 + 1)):
                self.assertFalse(self.validator(definition).is_valid({**point, field: value}))

    def test_schema_acceptance_does_not_establish_cross_field_energy_consistency(self):
        bad = copy.deepcopy(self.payload)
        statistics = bad["outcome"]["fitted"]["statistics"]
        statistics.update({"sse": available(0), "sst": available(20),
                           "model_ss": available(200), "r_squared": available(1)})
        # The full-release Rust semantic mutation must reject this shape-valid corruption.
        self.validator("regression_analysis").validate(bad)

    def test_schema_acceptance_does_not_establish_order_or_cohort_consistency(self):
        for mutate in (
            lambda p: p["terms"].reverse(),
            lambda p: p["levels"][0]["values"].reverse(),
            lambda p: p.update({"frequency": 3}),
        ):
            bad = copy.deepcopy(self.payload)
            mutate(bad)
            self.validator("regression_analysis").validate(bad)


if __name__ == "__main__":
    unittest.main()
