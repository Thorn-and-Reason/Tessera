"""Shape/local-bound tests of private v5 captures; no runtime-v5 acceptance claim.

Tracked fixtures were exported by the private guarded Rust capture controls,
using accepted independent/repeated engines. Dynamic semantics stay in Rust.
"""
import copy
import hashlib
import json
import pathlib
import unittest
from jsonschema import Draft202012Validator

ROOT = pathlib.Path(__file__).resolve().parents[2]
CURATED_IDENTITIES = {
    1: '158ec6e130aabfe6706e3eba617477e09da82bbd1b56d0b192e98946ea2aa51b',
    2: '102d913ec5cd04456fb5168643fbb032112fbe0e2d1eb6666aeeba8592a067e3',
    3: 'ae286a20ac3d80cc79815fad708be0801f406e6695a0a7439b151a10a4f5f6aa',
    4: 'fe37cc80a40c84ed69979c8e49fed92f00136c22bd94725c5671a2950eeef600',
}

class AnovaSchema(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.schema = json.loads((ROOT / 'docs/project.schema.json').read_text())
        Draft202012Validator.check_schema(cls.schema)
        cls.validator = Draft202012Validator(cls.schema)
        cls.payloads = json.loads((ROOT / 'tests/fixtures/anova-v5-payloads.json').read_text())
        cls.base = json.loads((ROOT / 'examples/exploration.tessera').read_text())

    def document(self, payload):
        result = copy.deepcopy(self.base)
        result['version'] = 3
        result['runs'] = [{'id': '1', 'analysis': {'anova': copy.deepcopy(payload)}}]
        result['next_run_id'] = '2'
        return result

    def invalid(self, document):
        self.assertFalse(self.validator.is_valid(document))

    def test_all_designs_confidences_and_absent_family(self):
        for payload in self.payloads:
            for confidence in ['90%', '95%', '99%']:
                document = self.document(payload)
                document['runs'][0]['analysis']['anova']['request']['confidence'] = confidence
                self.validator.validate(document)
        self.assertEqual(len(self.payloads), 5)
        absent = self.payloads[-1]['families'][0]['members'][0]
        self.assertIn('absent_requested_level', absent['state'])
        self.assertIsNone(absent['raw_p']['value'])
        self.assertEqual(self.payloads[-1]['families'][0]['m'], 1)

    def test_old_bytes_definitions_examples_and_old_header_refusal(self):
        earlier = json.loads((ROOT / 'docs/project-v4.schema.json').read_text())
        for name, definition in earlier['$defs'].items():
            if name != 'completed':
                self.assertEqual(definition, self.schema['$defs'][name], name)
        self.assertEqual(earlier['$defs']['completed']['oneOf'],
                         self.schema['$defs']['completed']['oneOf'][:10])
        for version, expected in CURATED_IDENTITIES.items():
            path = ROOT / f'docs/project-v{version}.schema.json'
            self.assertEqual(hashlib.sha256(path.read_bytes()).hexdigest(), expected)
            old = json.loads(path.read_text())
            Draft202012Validator.check_schema(old)
            bad = self.document(self.payloads[0])
            bad['version'] = max(4, version + 1)
            self.invalid(bad)
            self.assertFalse(Draft202012Validator(old).is_valid(bad))
        for example in ['sample', 'mixed-analysis', 'exploration']:
            old = json.loads((ROOT / f'examples/{example}.tessera').read_text())
            self.validator.validate(old)
            old['version'] = 1
            self.invalid(old)

    def test_every_new_record_is_closed_and_required(self):
        # Walk records through their individual definitions; compare Rust map tests.
        payload = self.payloads[2]
        model = payload['outcome']['fitted']
        effect = model['effects'][0]
        family = payload['families'][0]
        records = {
            'anova': payload, 'anova_request': payload['request'],
            'anova_within_factor': payload['request']['design']['repeated']['factors'][0],
            'anova_pair_request': family['members'][0]['request'],
            'anova_family_request': payload['request']['all_pairs'][0],
            'anova_factor_levels': payload['grid']['factors'][0],
            'anova_grid_cell': payload['grid']['cells'][0], 'anova_grid': payload['grid'],
            'anova_failure': {'kind': 'numerical', 'reason': 'unavailable'},
            'anova_energy': model['error_ss'], 'anova_test': effect['test'],
            'anova_correction': effect['gg'], 'anova_effect': effect,
            'anova_model_result': model, 'anova_comparison': family['members'][0],
            'anova_contrast_family': family, 'anova_method': payload['method'],
        }
        for name, record in records.items():
            validator = Draft202012Validator({'$defs': self.schema['$defs'],
                                              '$ref': '#/$defs/' + name})
            validator.validate(record)
            self.assertFalse(validator.is_valid(list(record.values())), name)
            bad = copy.deepcopy(record)
            bad['unknown'] = 0
            self.assertFalse(validator.is_valid(bad), name)
            for field in record:
                bad = copy.deepcopy(record)
                del bad[field]
                self.assertFalse(validator.is_valid(bad), (name, field))

    def test_repeated_mapping_weights_and_local_duplicates(self):
        for measurement in [['2', '2', '4'], ['2'], ['0', '3', '4'], ['02', '3', '4']]:
            bad = self.document(self.payloads[2])
            bad['runs'][0]['analysis']['anova']['request']['design']['repeated']['measurements'] = measurement
            self.invalid(bad)
        bad = self.document(self.payloads[2])
        bad['runs'][0]['analysis']['anova']['request']['frequency_weight'] = '1'
        self.invalid(bad)
        for levels in [['a', 'a', 'c'], ['a', 'b', '\u2003'], ['a']]:
            bad = self.document(self.payloads[2])
            bad['runs'][0]['analysis']['anova']['request']['design']['repeated']['factors'][0]['levels'] = levels
            self.invalid(bad)
        for name in ['', '\u2003']:
            bad = self.document(self.payloads[2])
            bad['runs'][0]['analysis']['anova']['request']['design']['repeated']['factors'][0]['name'] = name
            self.invalid(bad)
        bad = self.document(self.payloads[0])
        requests = bad['runs'][0]['analysis']['anova']['request']['all_pairs']
        requests.append(copy.deepcopy(requests[0]))
        self.invalid(bad)

    def test_counts_corrections_probabilities_intervals_and_method_bounds(self):
        changes = [
            ('source_rows', 2_000_001), ('frequency', 2**53 + 1),
            ('observed_measurements', -1),
        ]
        for field, value in changes:
            bad = self.document(self.payloads[2])
            bad['runs'][0]['analysis']['anova'][field] = value
            self.invalid(bad)
        for value in [0, 1e-320, 1.1]:
            bad = self.document(self.payloads[2])
            bad['runs'][0]['analysis']['anova']['outcome']['fitted']['effects'][0]['gg']['test']['p']['value'] = value
            self.invalid(bad)
        for value in [0, 1.01]:
            bad = self.document(self.payloads[2])
            bad['runs'][0]['analysis']['anova']['outcome']['fitted']['effects'][0]['gg']['epsilon']['value'] = value
            self.invalid(bad)
        for value in [0, 64]:
            bad = self.document(self.payloads[2])
            bad['runs'][0]['analysis']['anova']['outcome']['fitted']['effects'][0]['dimension'] = value
            self.invalid(bad)
        bad = self.document(self.payloads[0])
        bad['runs'][0]['analysis']['anova']['method']['numerical_version'] = 2
        self.invalid(bad)
        bad = self.document(self.payloads[0])
        bad['runs'][0]['analysis']['anova']['families'][0]['m'] = 0
        self.invalid(bad)
        bad = self.document(self.payloads[0])
        bad['runs'][0]['analysis']['anova']['families'][0]['members'][0]['interval'] = []
        self.invalid(bad)

    def test_energy_state_local_dependencies(self):
        for state, value in [('exact_zero', 1), ('positive', 0), ({'unavailable': 'numerical'}, 1)]:
            bad = self.document(self.payloads[0])
            energy = bad['runs'][0]['analysis']['anova']['outcome']['fitted']['model_ss']
            energy['state'] = state
            energy['value'] = {'value': value, 'reason': None}
            self.invalid(bad)

    def test_canonical_unit_tags(self):
        for field, value in [('state', {'available': None}), ('variance', {'positive': None})]:
            bad = self.document(self.payloads[0])
            bad['runs'][0]['analysis']['anova']['families'][0]['members'][0][field] = value
            self.invalid(bad)
        bad = self.document(self.payloads[0])
        bad['runs'][0]['analysis']['anova']['families'][0]['kind'] = {'planned': None}
        self.invalid(bad)
        bad = self.document(self.payloads[0])
        bad['runs'][0]['analysis']['anova']['request']['planned_pairs'][0]['scope'] = {'marginal': None}
        self.invalid(bad)

    def test_dynamic_linked_degrees_remain_rust_obligation(self):
        bad = self.document(self.payloads[2])
        bad['runs'][0]['analysis']['anova']['outcome']['fitted']['effects'][0]['gg']['test']['df2']['value'] = 2
        self.validator.validate(bad)  # Structurally valid, rejected by controlled Rust validation.

if __name__ == '__main__':
    unittest.main()
