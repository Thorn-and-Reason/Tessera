"""Validate the current encoded reliability capture and retained payload definitions."""
import copy
import hashlib
import json
import pathlib
import unittest
from jsonschema import Draft202012Validator
ROOT = pathlib.Path(__file__).resolve().parents[2]
class ReliabilitySchema(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.schema = json.loads((ROOT / 'docs/project.schema.json').read_text())
        Draft202012Validator.check_schema(cls.schema)
        cls.validator = Draft202012Validator(cls.schema)
        fixture = ROOT / 'tests/fixtures/reliability-schema-capture.tessera'
        assert hashlib.sha256(fixture.read_bytes()).hexdigest() == '94a8105d352d225d40528010cb56da0b6ef1eeb753d8370ab8b0852f39870cef'
        cls.document = json.loads(fixture.read_text())
    def test_public_capture_and_earlier_definitions(self):
        self.validator.validate(self.document)
        old = json.loads((ROOT / 'docs/project-v5.schema.json').read_text())
        for key, value in old['$defs'].items():
            if key != 'completed':
                current = copy.deepcopy(self.schema['$defs'][key])
                additions = {'frequencies_request': {'group'}, 'frequencies_analysis': {'groups'}, 'boxplot_group': {'mean'}}.get(key, set())
                self.assertEqual(set(current.get('properties', {})) - set(value.get('properties', {})), additions)
                for field in additions:
                    self.assertNotIn(field, current.get('required', []))
                    del current['properties'][field]
                self.assertEqual(value, current, key)
        self.assertEqual(old['$defs']['completed']['oneOf'], self.schema['$defs']['completed']['oneOf'][:-1])
    def test_closed_records_and_exact_tags(self):
        for field, value in [('method', {'exact_centered_interval_v1': None}), ('method', 'future'), ('total_energy', {'positive': None}), ('total_energy', 'future')]:
            document = copy.deepcopy(self.document)
            document['runs'][0]['analysis']['reliability'][field] = value
            self.assertFalse(self.validator.is_valid(document))
        for version in [0, 1, 2, 4, 5, 6]:
            document = copy.deepcopy(self.document); document['version'] = version
            self.assertFalse(self.validator.is_valid(document))
if __name__ == '__main__': unittest.main()
