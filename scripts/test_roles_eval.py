import unittest
from roles_eval import metrics, summarize, save, digest, ROLES
import pathlib
import tempfile
import contextlib
import io
import json

class MetricsTests(unittest.TestCase):
    def test_errors_and_abstentions_have_separate_denominators(self):
        rows=[dict(expected=False,probability=.9,status='present'),dict(expected=True,probability=.1,status='absent'),dict(expected=True,probability=.5,status='uncertain'),dict(expected=None,probability=.7,status='needs-context')]
        result=metrics(rows)
        self.assertEqual(result['labeled'],3)
        self.assertEqual(result['false_positives'],1)
        self.assertEqual(result['false_negatives'],1)
        self.assertEqual(result['positive_not_recovered'],2)
        self.assertEqual(result['decision_coverage'],2/3)
        self.assertEqual(result['needs_context'],1)
        self.assertAlmostEqual(result['brier_score'],(.81+.81+.25)/3)
    def test_calibration_bin_boundaries_count_each_answer_once(self):
        rows=[dict(expected=True,probability=p,status='uncertain') for p in [0,.2,.4,.6,.8,1]]
        result=metrics(rows)
        self.assertEqual(sum(b['count'] for b in result['calibration_bins']),len(rows))
        self.assertEqual([b['count'] for b in result['calibration_bins']],[1,1,1,1,2])
    def test_unlabeled_regions_are_not_negative_examples(self):
        result=metrics([dict(expected=None,probability=.99,status='needs-context')])
        self.assertIsNone(result['brier_score'])
        self.assertIsNone(result['false_positive_rate'])
        self.assertEqual(result['labeled'],0)

class SummaryTests(unittest.TestCase):
    def test_path_variant_changes_are_reported_without_inflating_accuracy(self):
        with tempfile.TemporaryDirectory() as temporary:
            root=pathlib.Path(temporary);output=root/'run';output.mkdir()
            cases=[];labels=[]
            for ident,path,probability in [('a','module.py',.95),('b','tests/module.py',.1)]:
                case=dict(id=ident,path=path,source_hash='frozen',repository='example',language='py',split='holdout')
                if ident=='b':case['path_variant_of']='a'
                cases.append(case)
                evidence=dict(path=path,name='operation',start_line=1,end_line=2)
                labels.append(dict(id=ident,regions=[dict(index=0,evidence=evidence,evidence_sufficient=True,roles={role:role=='test_scenario' for role in ROLES})]))
                roles={role:dict(answer=dict(type='noul',noul=probability if role=='test_scenario' else .01),status=('present' if probability>.8 else 'absent') if role=='test_scenario' else 'absent') for role in ROLES}
                assessment=dict(model='jev-1.13.0',version='fixture',regions=[dict(index=0,evidence=evidence,evidence_sufficiency=dict(noul=.99),roles=roles)],limitations=dict(regions_omitted=0,unsupported_parser_paths=[]))
                (output/ident).mkdir()
                save(output/ident/'result.json',dict(complete=True,files=[dict(source_hash='frozen',status='clear',role_assessment=assessment)]))
            save(root/'manifest.json',dict(cases=cases));save(root/'labels.json',dict(review_status='pending-human-review',cases=labels))
            save(root/'freeze.json',{})
            save(output/'run.json',dict(freeze_sha256=digest(root/'freeze.json'),labels_sha256=digest(root/'labels.json')))
            save(output/'usage.json',[])
            with contextlib.redirect_stdout(io.StringIO()):summarize(root,[output],root/'summary.json')
            summary=json.loads((root/'summary.json').read_text())
            self.assertEqual(summary['metrics']['overall']['test_scenario/all']['labeled'],1)
            self.assertEqual(summary['metrics']['overall']['test_scenario/all']['false_negatives'],0)
            self.assertEqual(summary['path_checks'][0]['status_changes'],1)
            self.assertAlmostEqual(summary['path_checks'][0]['max_probability_change'],.85)
            self.assertIn('NOT human-validated',summary['interpretation'])

if __name__=='__main__':unittest.main()
