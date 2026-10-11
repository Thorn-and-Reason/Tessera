#!/usr/bin/env python3
"""Independent mpmath oracle for the exact Float64 log-F API; no Rust imports."""
import argparse
import copy
import json
import math
from pathlib import Path
import sys

import mpmath as mp

VERSION = 1
DPS = 160
mp.mp.dps = DPS
DEFAULT = Path(__file__).resolve().parents[1] / 'tests/fixtures/regression-tails.json'


def reference(case):
    # Promote the actual binary64 API input exactly, not its displayed decimal.
    f = mp.exp(mp.mpf(case['log_f']))
    a, b = mp.mpf(case['df1']) / 2, mp.mpf(case['df2']) / 2
    return mp.betainc(b, a, 0, b / (b + a * f), regularized=True)


def generate(historical):
    rows = []
    for old in json.loads(historical.read_text())['cases']:
        rows.append(dict(name=old['name'], df1=old['d1'], df2=old['d2'],
                         log_f=math.log(old['f']), original_f=old['f'],
                         historical_reference=old['reference'],
                         historical_status=old['expected_status'],
                         cancel_at=old.get('cancel_at')))
    for df1 in range(1, 64):
        rows.append(dict(name=f'grid_{df1}', df1=df1, df2=37,
                         log_f=math.log(2.25), original_f=2.25,
                         historical_reference=None, historical_status=None, cancel_at=None))
    for df1 in [1, 2, 3, 31, 62, 63]:
        rows.append(dict(name=f'underflow_{df1}', df1=df1, df2=10,
                         log_f=1000.0, original_f=None, historical_reference=None,
                         historical_status=None, cancel_at=None))
    for offset in [-1e-7, 1e-7, -1e-12, 1e-12]:
        rows.append(dict(name=f'near_one_{offset}', df1=2, df2=2,
                         log_f=-54 * math.log(2) + offset, original_f=None,
                         historical_reference=None, historical_status=None, cancel_at=None))
    for row in rows:
        ref = reference(row)
        status = ('cancelled' if row['cancel_at'] is not None else
                  'underflow' if float(ref) == 0 else 'available')
        row.update(expected_status=status, reference=mp.nstr(ref, DPS),
                   rounded=float(ref), relative_tolerance=1e-10, subnormal_ulps=1)
    return dict(generator_version=VERSION, mpmath_version=mp.__version__, dps=DPS,
                input_semantics='mp.mpf(actual Float64 log_f), then high-precision exp', cases=rows)


def check_fixture(fixture):
    assert fixture['generator_version'] == VERSION
    assert fixture['dps'] == DPS
    assert fixture['mpmath_version'] == '1.3.0'
    assert fixture['input_semantics']
    rows = fixture['cases']
    assert len(rows) == 317 and len({r['name'] for r in rows}) == len(rows)
    fields = {'name', 'df1', 'df2', 'log_f', 'original_f', 'historical_reference',
              'historical_status', 'cancel_at', 'expected_status', 'reference',
              'rounded', 'relative_tolerance', 'subnormal_ulps'}
    for row in rows:
        assert set(row) == fields, row['name']
        ref = reference(row)
        assert mp.isfinite(ref) and 0 < ref <= 1
        assert abs(mp.mpf(row['reference']) / ref - 1) < mp.mpf('1e-150'), row['name']
        assert row['rounded'] == float(ref), row['name']
        assert row['relative_tolerance'] == 1e-10 and row['subnormal_ulps'] == 1
        status = ('cancelled' if row['cancel_at'] is not None else
                  'underflow' if float(ref) == 0 else 'available')
        assert row['expected_status'] == status, row['name']


def check_results(fixture, path):
    outputs = [json.loads(line.split('TAIL_RESULT ', 1)[1]) for line in path.read_text().splitlines()
               if 'TAIL_RESULT ' in line]
    refs = {row['name']: row for row in fixture['cases']}
    assert len(outputs) == len(refs) and {r['name'] for r in outputs} == set(refs)
    audit = []
    for row in outputs:
        assert set(row) == {'name', 'status', 'value', 'reason'}
        case = refs[row['name']]
        assert row['status'] == case['expected_status'], row
        if row['status'] == 'available':
            value, ref = row['value'], mp.mpf(case['reference'])
            assert type(value) in (float, int) and math.isfinite(value) and 0 < value <= 1
            assert row['reason'] is None
            error = abs(mp.mpf(value) - ref)
            assert (error / ref <= case['relative_tolerance'] if ref >= sys.float_info.min else
                    error <= mp.mpf(math.ulp(value)) * case['subnormal_ulps']), row
            old = case['historical_reference']
            if old is not None:
                historical = mp.mpf(old)
                assert (abs(mp.mpf(value) / historical - 1) <= case['relative_tolerance']
                        if historical >= sys.float_info.min else
                        abs(mp.mpf(value) - historical) <= mp.mpf(math.ulp(value))), row
        else:
            assert row['value'] is None and isinstance(row['reason'], str) and row['reason'], row
        audit.append(dict(row, expected_status=case['expected_status'],
                          historical_status=case['historical_status'], reference=case['reference'],
                          relative_error=None if row['value'] is None else
                          float(abs(mp.mpf(row['value']) / mp.mpf(case['reference']) - 1)),
                          ulp_error=None if row['value'] is None else
                          float(abs(mp.mpf(row['value']) - mp.mpf(case['reference'])) /
                                mp.mpf(math.ulp(row['value'])))))
    print(f'PASS: {len(outputs)} individual statuses, required fields and tolerances')
    bounds = [json.loads(line.split('BOUND_RESULT ', 1)[1]) for line in path.read_text().splitlines()
              if 'BOUND_RESULT ' in line]
    assert len(bounds) == len(refs) and {r['name'] for r in bounds} == set(refs)
    worst = (mp.mpf(0), None, None, None)
    certified_count = 0
    for row in bounds:
        assert set(row) == {'name', 'log_hi', 'log_lo', 'margin', 'certified'}
        case = refs[row['name']]
        a, b = mp.mpf(case['df1']) / 2, mp.mpf(case['df2']) / 2
        ratio = mp.mpf(case['log_f']) + mp.log(case['df1']) - mp.log(case['df2'])
        log_q, log_z = -mp.log1p(mp.exp(-ratio)), -mp.log1p(mp.exp(ratio))
        if case['df1'] % 2:
            m = (case['df1'] - 1) // 2
            coefficient = 2 * mp.sqrt(b) * mp.rf(b + mp.mpf('.5'), m) / mp.rf(mp.mpf('1.5'), m)
        else:
            m = case['df1'] // 2
            coefficient = mp.rf(b, m) / mp.factorial(m)
        expected_bound = a * log_q + mp.log(coefficient) + min(0, b - 1) * log_z
        actual = mp.mpf(row['log_hi']) + mp.mpf(row['log_lo'])
        margin = mp.mpf(row['margin'])
        assert mp.isfinite(actual) and mp.isfinite(margin) and margin > 0
        error = abs(actual - expected_bound)
        assert error <= margin, row
        fraction = error / margin
        if fraction > worst[0]:
            worst = (fraction, row['name'], error, margin)
        expected_certified = actual + margin < -54 * mp.log(2)
        assert row['certified'] == expected_certified, row
        if row['certified']:
            assert expected_bound < -54 * mp.log(2), row
            assert float(reference(case)) == 1.0, row
            certified_count += 1
    print('PASS: independent lower-tail bounds; certified', certified_count)
    print('Maximum log-bound error/margin:', *[mp.nstr(x, 18) if isinstance(x, mp.mpf) else x for x in worst])
    return dict(cases=audit, lower_tail_bounds=bounds, certified_count=certified_count,
                worst_log_bound_error_margin=[str(x) for x in worst])



def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--historical', type=Path)
    parser.add_argument('--fixture', type=Path, default=DEFAULT)
    parser.add_argument('--results', type=Path)
    parser.add_argument('--audit-output', type=Path)
    parser.add_argument('--mutation-check', action='store_true')
    args = parser.parse_args()
    if args.historical:
        args.fixture.write_text(json.dumps(generate(args.historical), indent=2) + '\n')
    fixture = json.loads(args.fixture.read_text())
    check_fixture(fixture)
    print(f'PASS: {len(fixture["cases"])} independent {DPS}-digit references')
    if args.results:
        audit = check_results(fixture, args.results)
        if args.audit_output:
            args.audit_output.write_text(json.dumps(audit, indent=2) + "\n")
    if args.mutation_check:
        for mutation in ['wrong_expected', 'missing_field']:
            changed = copy.deepcopy(fixture)
            if mutation == 'wrong_expected':
                changed['cases'][0]['rounded'] *= 0.5
            else:
                del changed['cases'][0]['expected_status']
            try:
                check_fixture(changed)
            except (AssertionError, KeyError) as error:
                print(f'EXPECTED REJECTION {mutation}: {type(error).__name__}: {error}')
            else:
                raise AssertionError(f'checker accepted {mutation}')


if __name__ == '__main__':
    main()
