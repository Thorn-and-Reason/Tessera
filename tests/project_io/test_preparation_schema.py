"""Current closed preparation records, identity strings and bounded recipe shape."""
import copy
import json
import unittest
from pathlib import Path
from jsonschema import Draft202012Validator
ROOT = Path(__file__).resolve().parents[2]
class PreparationSchema(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        schema = json.loads((ROOT / 'docs/project.schema.json').read_text())
        Draft202012Validator.check_schema(schema)
        cls.validator = Draft202012Validator(schema)
        cls.base = json.loads((ROOT / 'examples/sample.tessera').read_text())
    def record(self, step):
        value = copy.deepcopy(self.base)
        value['dataset']['revision'] = '2'
        value['preparation'] = [{'source_revision': '1', 'result_revision': '2', 'step': step}]
        return value
    def test_closed_steps_and_bounds(self):
        steps = [{'compute': {'name':'y','left':{'column':'1'},'operator':'add','right':{'constant':2}}},
                 {'recode': {'source':'1','name':'label','kind':'text','rules':[{'from':2,'to':'two'}],'unmatched':'missing'}},
                 {'sort': {'source':'1','descending':False}},
                 {'filter': {'source':'1','predicate':{'compare':{'comparison':'greater','value':2}}}},
                 {'filter': {'source':'1','predicate':'missing'}}, 'dataset_edit']
        for step in steps:
            value = self.record(step)
            self.validator.validate(value)
            for bad_record in [[], {'source_revision':'1','result_revision':'2','step':step,'extra':True}]:
                bad = copy.deepcopy(value); bad['preparation'][0] = bad_record
                self.assertFalse(self.validator.is_valid(bad))
        for version in [0,1,2,4]:
            value = copy.deepcopy(self.base); value['version'] = version
            self.assertFalse(self.validator.is_valid(value))
        value=self.record(steps[0]);value['preparation'] *= 257
        self.assertFalse(self.validator.is_valid(value))
    def test_new_fields_are_required_and_ids_canonical(self):
        for field in ['next_column_id','preparation']:
            bad = copy.deepcopy(self.base); del bad[field]
            self.assertFalse(self.validator.is_valid(bad))
        for identity in ['0','01',1,'18446744073709551616']:
            bad=copy.deepcopy(self.base);bad['next_column_id']=identity
            self.assertFalse(self.validator.is_valid(bad))
