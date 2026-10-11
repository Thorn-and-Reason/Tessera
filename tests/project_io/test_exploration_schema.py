"""Run with PYTHONPATH=.build/schema-tools python3 tests/project_io/test_exploration_schema.py -v."""
import copy
import hashlib
import json
import unittest
from pathlib import Path
from jsonschema import Draft202012Validator

ROOT = Path(__file__).resolve().parents[2]
CURATED_IDENTITIES = {
    'docs/project-v1.schema.json': '158ec6e130aabfe6706e3eba617477e09da82bbd1b56d0b192e98946ea2aa51b',
    'tests/fixtures/pre-tessera-format/sample.json': '5c2875344fae2dc1baa1bb6e4ce16404ae44cbc11daf6013dac495da85507337',
    'docs/project-v2.schema.json': '102d913ec5cd04456fb5168643fbb032112fbe0e2d1eb6666aeeba8592a067e3',
    'tests/fixtures/pre-tessera-format/mixed-analysis.json': 'd08f39e3db767ad6a311d9d70c25e21839c8c40d4bef7e85e883250f22a5cd42',
}

class ExplorationSchemaTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.schema = json.loads((ROOT / 'docs/project.schema.json').read_text())
        Draft202012Validator.check_schema(cls.schema)
        cls.validator = Draft202012Validator(cls.schema)
        cls.example = json.loads((ROOT / 'examples/exploration.tessera').read_text())

    def test_all_three_schemas_and_examples(self):
        for example in ['sample', 'mixed-analysis', 'exploration', 'regression-analysis']:
            self.validator.validate(json.loads((ROOT/f'examples/{example}.tessera').read_text()))

    def test_converted_examples_preserve_every_dataset_and_capture_value(self):
        for name in ['sample', 'mixed-analysis', 'exploration', 'regression-analysis']:
            old = json.loads((ROOT/f'tests/fixtures/pre-tessera-format/{name}.json').read_text())
            self.assertEqual(old['format'], 'foreign-project')
            self.assertFalse(self.validator.is_valid(old), name)
            old['format'] = 'tessera-project'
            old['version'] = 3
            old['next_column_id'] = str(max(int(c['id']) for c in old['dataset']['columns'])+1)
            old['preparation'] = []
            for run in old['runs']:
                if 'operation' in run:
                    run['analysis'] = {run.pop('operation'): run['analysis']}
            current = json.loads((ROOT/f'examples/{name}.tessera').read_text())
            self.assertEqual(old, current, name)

    def test_curated_reference_bytes_are_stable(self):
        for path, expected in CURATED_IDENTITIES.items():
            self.assertEqual(hashlib.sha256((ROOT/path).read_bytes()).hexdigest(),expected,path)

    def test_closed_required_new_shapes_and_old_version_refusal(self):
        for i, tag in enumerate(['frequencies','histogram','scatter','boxplot'],5):
            payload=self.example['runs'][i]['analysis'][tag]
            for field in payload:
                bad=copy.deepcopy(self.example)
                del bad['runs'][i]['analysis'][tag][field]
                self.assertFalse(self.validator.is_valid(bad),(tag,field))
            bad=copy.deepcopy(self.example)
            bad['runs'][i]['analysis'][tag]['unknown']=0
            self.assertFalse(self.validator.is_valid(bad))
            bad=copy.deepcopy(self.example)
            bad['runs'][i]['analysis'][tag]=list(payload.values())
            self.assertFalse(self.validator.is_valid(bad))
        for version in (1,2):
            old=json.loads((ROOT/f'docs/project-v{version}.schema.json').read_text())
            bad=copy.deepcopy(self.example);bad['version']=version
            self.assertFalse(Draft202012Validator(old).is_valid(bad))

    def test_limits_nullable_estimates_and_binning(self):
        def invalid(index, tag, field, value):
            bad=copy.deepcopy(self.example);bad['runs'][index]['analysis'][tag][field]=value
            self.assertFalse(self.validator.is_valid(bad),(tag,field))
        for tag,index in [('frequencies',5),('histogram',6),('scatter',7),('boxplot',8)]:
            invalid(index,tag,'source_rows',2_000_001)
        for bins in [{'count':0},{'count':129},{'count':1,'automatic':None},'invalid']:
            bad=copy.deepcopy(self.example);bad['runs'][6]['analysis']['histogram']['request']['bins']=bins
            self.assertFalse(self.validator.is_valid(bad))
        invalid(6,'histogram','n',2**53+1)
        invalid(7,'scatter','points',[self.example['runs'][7]['analysis']['scatter']['points'][0]]*2001)
        invalid(6,'histogram','range',[{'value':None,'reason':None}]*2)
        invalid(6,'histogram','range',[{'value':0,'reason':'unavailable'}]*2)
        invalid(6,'histogram','range',[{'value':None,'reason':''}]*2)

    def test_old_definitions_preserve_semantics(self):
        earlier=json.loads((ROOT/'docs/project-v2.schema.json').read_text())
        for name,value in earlier['$defs'].items():
            if name != 'completed':self.assertEqual(value,self.schema['$defs'][name],name)
        self.assertEqual(earlier['$defs']['completed']['oneOf'],self.schema['$defs']['completed']['oneOf'][:5])

if __name__ == '__main__':
    unittest.main()
