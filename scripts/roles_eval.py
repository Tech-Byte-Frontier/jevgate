#!/usr/bin/env python3
"""Frozen role evaluation through the real CLI; source workspaces never contain labels."""
import argparse
import collections
import hashlib
import json
import math
import pathlib
import statistics
import subprocess
import time

ROLES = ('test_scenario', 'test_support', 'framework_tool', 'application_library')


def read(path):
    return json.loads(path.read_text())


def save(path, value):
    path.write_text(json.dumps(value, indent=2) + '\n')


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def preview(binary, workspace, case):
    command = [str(binary), 'check', case['path'], '--roles-only', '--dry-run', '--show-requests', '--format', 'json']
    result = subprocess.run(command, cwd=workspace, capture_output=True, text=True, check=True)
    report = json.loads(result.stdout)
    assert len(report['initial_requests']) == 1 and not report['errors']
    request = report['initial_requests'][0]
    assert request['state']['file']['source_hash'] == case['source_hash']
    return report, hashlib.sha256(json.dumps(request, sort_keys=True).encode()).hexdigest()


def freeze(root, binary):
    manifest = read(root/'manifest.json')
    assert not (root/'freeze.json').exists(), 'Already frozen; make a new dataset version instead of overwriting first evidence'
    cases_by_id = {case['id']: case for case in manifest['cases']}
    assert len(cases_by_id) == len(manifest['cases']), 'Duplicate case IDs'
    for case in manifest['cases']:
        if case.get('path_variant_of'):
            original = cases_by_id[case['path_variant_of']]
            assert case['source_hash'] == original['source_hash'], 'Path controls must have identical source bytes'
            assert case['repository'] == original['repository'] and case['split'] == original['split'], 'Path variants must stay with their source repository and split'
    repos = collections.defaultdict(set)
    labels = []
    frozen = []
    for case in manifest['cases']:
        workspace = root/case['id']/'workspace'
        assert digest(workspace/case['path']) == case['source_hash']
        assert not (workspace/'labels.json').exists()
        repos[case['repository']].add(case['split'])
        report, fingerprint = preview(binary, workspace, case)
        save(root/case['id']/'preview.json', report)
        regions = report['initial_requests'][0]['state']['regions']
        frozen.append({'id': case['id'], 'request_hash': fingerprint, 'source_hash': case['source_hash']})
        labels.append({'id': case['id'], 'regions': [{'index': i, 'evidence': r, 'roles': dict.fromkeys(ROLES), 'evidence_sufficient': None, 'rationale': ''} for i, r in enumerate(regions)]})
    assert all(len(splits) == 1 for splits in repos.values()), 'Repository leakage between development and holdout'
    save(root/'labels.template.json', {'review_status': 'pending-human-review', 'reviewer': None, 'cases': labels})
    save(root/'freeze.json', {'manifest_sha256': digest(root/'manifest.json'), 'binary_sha256': digest(binary), 'cases': frozen})


def run(root, binary, split, env_file, output):
    frozen = read(root/'freeze.json')
    assert frozen['manifest_sha256'] == digest(root/'manifest.json'), 'Manifest changed after freeze'
    assert frozen['binary_sha256'] == digest(binary), 'Binary changed after freeze; create a new experiment'
    cases = [c for c in read(root/'manifest.json')['cases'] if c['split'] == split]
    assert cases
    output.mkdir(exist_ok=False)
    save(output/'run.json', {'split': split, 'binary_sha256': digest(binary), 'freeze_sha256': digest(root/'freeze.json'), 'labels_sha256': digest(root/'labels.json')})
    batches = []
    for case in cases:
        workspace = root/case['id']/'workspace'
        assert digest(workspace/case['path']) == case['source_hash']
        report, fingerprint = preview(binary, workspace, case)
        assert fingerprint == next(c for c in frozen['cases'] if c['id'] == case['id'])['request_hash']
        target = output/case['id']; target.mkdir()
        save(target/'preview.json', report)
        command = [str(binary), 'check', case['path'], '--roles-only', '--format', 'json', '--refresh', '--max-requests', '1', '--concurrency', '1', '--env-file', str(env_file)]
        save(target/'command.json', {'argv': command, 'cwd': str(workspace)})
        start = time.monotonic()
        with (target/'result.json').open('w') as stdout, (target/'stderr').open('w') as stderr:
            result = subprocess.run(command, cwd=workspace, stdout=stdout, stderr=stderr)
        elapsed = time.monotonic()-start
        save(target/'execution.json', {'exit_code': result.returncode, 'seconds': elapsed})
        report = read(target/'result.json')
        batch = {k: report[k] for k in ('api_requests', 'paid_input_tokens', 'paid_output_tokens', 'complete', 'errors')}
        batch.update(case=case['id'], seconds=elapsed, estimated_usd=report['paid_input_tokens']*.042/1e6)
        batches.append(batch); save(output/'usage.json', batches)
        print(json.dumps(batch), flush=True)
        if result.returncode or not report['complete']:
            raise SystemExit('Stopped on first failure; preserve results and inspect before any retry')


def metrics(rows):
    scored = [r for r in rows if isinstance(r['expected'], bool)]
    decided = [r for r in scored if r['status'] in ('present', 'absent')]
    positives = sum(r['expected'] for r in scored)
    negatives = len(scored)-positives
    fp = sum(r['status']=='present' and not r['expected'] for r in scored)
    fn = sum(r['status']=='absent' and r['expected'] for r in scored)
    bins = []
    for index in range(5):
        low = index/5
        values = [r for r in scored if min(4, int(r['probability']*5)) == index]
        if values:
            bins.append({'lower': low, 'count': len(values), 'mean_probability': statistics.mean(r['probability'] for r in values), 'positive_fraction': statistics.mean(r['expected'] for r in values)})
    return {'count': len(rows), 'labeled': len(scored), 'positive_labels': positives, 'negative_labels': negatives,
            'false_positives': fp, 'false_negatives': fn,
            'false_positive_rate': fp/negatives if negatives else None, 'false_negative_rate': fn/positives if positives else None,
            'positive_not_recovered': sum(r['expected'] and r['status']!='present' for r in scored),
            'uncertain': sum(r['status']=='uncertain' for r in rows), 'needs_context': sum(r['status']=='needs-context' for r in rows),
            'decision_coverage': len(decided)/len(scored) if scored else None,
            'brier_score': statistics.mean((r['probability']-r['expected'])**2 for r in scored) if scored else None,
            'calibration_bins': bins,
            'expected_calibration_error': sum(b['count']*abs(b['mean_probability']-b['positive_fraction']) for b in bins)/len(scored) if scored else None}


def summarize(root, outputs, destination):
    assert not destination.exists(), 'Preserve earlier summaries; choose a new output path'
    manifest = read(root/'manifest.json'); labels = read(root/'labels.json'); rows=[]; evidence=[]; evaluated={}; usage=[]; coverage=[]
    assert labels['review_status'] in ('pending-human-review', 'human-reviewed')
    if labels['review_status']=='human-reviewed':
        assert labels.get('reviewer') and labels.get('reviewed_at'), 'Human review needs provenance'
    for output in outputs:
        run_info=read(output/'run.json')
        assert run_info['freeze_sha256']==digest(root/'freeze.json')
        assert run_info['labels_sha256']==digest(root/'labels.json'), 'Labels changed after inference'
        usage.extend(read(output/'usage.json'))
        for case in manifest['cases']:
            result=output/case['id']/'result.json'
            if not result.exists(): continue
            assert case['id'] not in evaluated, 'Do not pool repeated runs as independent examples'
            report=read(result)
            if not report['complete']: continue
            file=report['files'][0]
            assert file['source_hash']==case['source_hash']
            assessment=file['role_assessment'];evaluated[case['id']]=assessment
            coverage.append({'case':case['id'],'regions_classified':len(assessment['regions']),
                             'regions_omitted':assessment['limitations']['regions_omitted'],
                             'unsupported_parser_paths':assessment['limitations']['unsupported_parser_paths'],
                             'file_status':file['status'],'model':assessment['model'],'role_version':assessment['version']})
            expected=next(c for c in labels['cases'] if c['id']==case['id'])
            assert len(expected['regions'])==len(assessment['regions'])
            for region, target in zip(assessment['regions'],expected['regions']):
                assert region['index']==target['index'] and region['evidence']==target['evidence']
                evidence.append({'case':case['id'],'expected':target['evidence_sufficient'],'probability':region['evidence_sufficiency']['noul']})
                for role in ROLES:
                    signal=region['roles'][role];p=signal['answer']['noul'];assert math.isfinite(p) and 0<=p<=1
                    rows.append({'case':case['id'],'region':region['index'],'role':role,'repository':case['repository'],'language':case['language'],'split':case['split'],'expected':target['roles'][role],'probability':p,'status':signal['status']})
    grouped={}
    variants={c['id'] for c in manifest['cases'] if c.get('path_variant_of')}
    canonical_rows=[row for row in rows if row['case'] not in variants]
    for axis in ('overall','language','repository','split'):
        groups=collections.defaultdict(list)
        for row in canonical_rows: groups[(row['role'],row.get(axis,'all'))].append(row)
        grouped[axis]={f'{role}/{value}':metrics(group) for (role,value),group in sorted(groups.items())}
    path_checks=[]
    for case in manifest['cases']:
        original=case.get('path_variant_of')
        if original and case['id'] in evaluated and original in evaluated:
            left=evaluated[original]['regions'];right=evaluated[case['id']]['regions']
            assert len(left)==len(right)
            differences=[]
            for a,b in zip(left,right):
                for role in ROLES:
                    x=a['roles'][role];y=b['roles'][role]
                    differences.append({'region':a['index'],'role':role,'absolute_probability_change':abs(x['answer']['noul']-y['answer']['noul']),'status_changed':x['status']!=y['status']})
            path_checks.append({'original':original,'variant':case['id'],'max_probability_change':max(d['absolute_probability_change'] for d in differences),'status_changes':sum(d['status_changed'] for d in differences),'differences':differences})
    summary={'label_review_status':labels['review_status'],'reviewer':labels.get('reviewer'),
             'interpretation':'Provisional metrics against agent-proposed labels; NOT human-validated accuracy' if labels['review_status']!='human-reviewed' else 'Frozen labeled sample; not a guarantee across codebases',
             'cases_evaluated':list(evaluated),'cases_missing':[c['id'] for c in manifest['cases'] if c['id'] not in evaluated],
             'metrics':grouped,'path_checks':path_checks,'coverage':coverage,'evidence_sufficiency':evidence,'rows':rows,
             'metric_scope':'Canonical source cases only; byte-identical path variants are measured separately and do not inflate accuracy or calibration denominators. Regions within one source are correlated.',
             'batch':{'attempts':sum(b['api_requests'] for b in usage),'input_tokens':sum(b['paid_input_tokens'] for b in usage),'output_tokens':sum(b['paid_output_tokens'] for b in usage),'estimated_usd':sum(b['estimated_usd'] for b in usage)},
             'adoption':'Specialists remain disconnected from these roles pending human-reviewed evaluation and downstream comparison.'}
    save(destination,summary)
    print(json.dumps({k:summary[k] for k in ('label_review_status','cases_evaluated','cases_missing','batch')},indent=2))


def main():
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('action',choices=['freeze','run','summarize'])
    parser.add_argument('root',type=pathlib.Path)
    parser.add_argument('--binary',type=pathlib.Path,default=pathlib.Path('target/release/jevgate'))
    parser.add_argument('--split',choices=['development','holdout'],default='development')
    parser.add_argument('--env-file',type=pathlib.Path,default=pathlib.Path('.env'))
    parser.add_argument('--output',type=pathlib.Path)
    parser.add_argument('--runs',type=pathlib.Path,nargs='+')
    args=parser.parse_args();root=args.root.resolve();binary=args.binary.resolve()
    if args.action=='freeze': freeze(root,binary)
    elif args.action=='run':
        assert args.output, '--output required';run(root,binary,args.split,args.env_file.resolve(),args.output.resolve())
    else:
        assert args.runs and args.output, '--runs and --output required';summarize(root,[p.resolve() for p in args.runs],args.output.resolve())


if __name__=='__main__':main()
