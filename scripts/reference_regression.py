#!/usr/bin/env python3
"""Independent frequency OLS references from exact binary64 inputs.

Run with the existing Python 3.11 / NumPy 2.0.2 / mpmath 1.3.0 environment.
The JSON contains the input cohorts, so regeneration needs no scratch evidence.
Exact rational weighted algebra detects algebraic zero; 180-digit mpmath evaluates
inference. NumPy independently checks ordinary scaled least-squares coefficients.
The mathematical references do not prescribe a kernel's computed zero residual.
"""
import argparse
import copy
from fractions import Fraction as Q
from functools import lru_cache
import json
import math
from pathlib import Path
import sys

import mpmath as mp
import numpy as np

DPS = 180
PROPAGATED_CASES = {'reference_near_A_3.55e-13', 'reference_near_C_2e-12'}
mp.mp.dps = DPS
DEFAULT = Path(__file__).resolve().parents[1] / 'tests/fixtures/regression-fit.json'
RANGE = 'The estimate is outside finite Float64 range.'
UNDERFLOW = 'A nonzero estimate is below Float64 range.'
NO_DF = 'There are no residual degrees of freedom.'
ZERO = 'Residual variance is exactly zero.'
CONSTANT = 'The outcome has no variation.'


def high(value):
    return mp.mpf(value.numerator) / value.denominator if isinstance(value, Q) else mp.mpf(value)


def estimate(value=None, reason=None):
    if reason is not None:
        return {'status': 'unavailable', 'reference': None, 'value': None, 'reason': reason}
    value = high(value)
    assert mp.isfinite(value)
    rounded = float(value)
    reason = RANGE if not math.isfinite(rounded) else UNDERFLOW if rounded == 0 and value else None
    return {'status': 'unavailable' if reason else 'available',
            'reference': mp.nstr(value, DPS), 'value': None if reason else rounded, 'reason': reason}


def inverse(gram):
    """Small exact weighted normal equations are a reference, never production code."""
    p = len(gram)
    a = [list(row) + [Q(i == j) for j in range(p)] for i, row in enumerate(gram)]
    pivot = 0
    for column in range(p):
        selected = next((i for i in range(pivot, p) if a[i][column]), None)
        if selected is None:
            continue
        a[pivot], a[selected] = a[selected], a[pivot]
        divisor = a[pivot][column]
        a[pivot] = [v / divisor for v in a[pivot]]
        for i in range(p):
            if i != pivot:
                multiple = a[i][column]
                a[i] = [v - multiple * t for v, t in zip(a[i], a[pivot])]
        pivot += 1
    return (None if pivot < p else [r[p:] for r in a]), pivot


def beta_tail(a, b, x):
    """I_x(b,a), including huge integer degrees of freedom without expansion."""
    if x == 1:
        return mp.mpf(1)
    if x == 0:
        return mp.mpf(0)
    if b < 10000:
        return mp.betainc(b,a,0,x,regularized=True)
    # With u=x*exp(-s/b), the integral has an exponentially decaying integrand.
    # This avoids hypergeometric convergence failures at enormous beta shapes.
    log_factor = b * mp.log(x) - mp.log(b) - mp.log(mp.beta(b, a))
    integral = mp.quad(lambda s: mp.exp(-s) *
                       (1 - x + x * (-mp.expm1(-s / b))) ** (a - 1),
                       [0, 1, 10, 100, mp.inf])
    return mp.exp(log_factor) * integral


def student_p(t, df):
    t = abs(t)
    if not t:
        return mp.mpf(1)
    return beta_tail(mp.mpf('.5'), mp.mpf(df) / 2, df / (df + t*t))


@lru_cache(maxsize=None)
def critical(df, confidence):
    target = 1 - mp.mpf(confidence) / 100
    # Central integration stays well conditioned for the three supported levels.
    d = mp.mpf(df)
    norm = mp.exp(mp.loggamma((d+1)/2) - mp.loggamma(d/2)) / mp.sqrt(d*mp.pi)
    cdf = lambda t: 1 - 2 * norm * mp.quad(
        lambda x: mp.exp(-(d+1)/2 * mp.log1p(x*x/d)), [0, t])
    guess = mp.sqrt(2) * mp.erfinv(1-target)
    root = mp.findroot(lambda t: cdf(t)-target, (guess, guess*mp.mpf('1.1')))
    assert abs(cdf(root)-target) < mp.mpf('1e-160')
    return root


def scaled_design(x, w):
    n, p = sum(w), len(x[0])
    centres = [sum(k*r[j] for r,k in zip(x,w))/n for j in range(p)]
    scales = [mp.mpf(1)] + [mp.sqrt(high(sum(k*(r[j]-centres[j])**2
                         for r,k in zip(x,w))/n)) for j in range(1,p)]
    a = mp.matrix([[mp.sqrt(mp.mpf(k)/n) * (1 if j == 0 else
                    high(r[j]-centres[j])/scales[j] if scales[j] else 0)
                    for j in range(p)] for r,k in zip(x,w)])
    return a, centres, scales


def numpy_check(a, y, w, centres, scales, beta):
    unit = max(abs(v) for v in y) or Q(1)
    ym = sum(k*v for k,v in zip(w,y))/sum(w)
    b = np.array([float(high(v-ym)/high(unit))*math.sqrt(k/sum(w)) for v,k in zip(y,w)])
    matrix = np.array(a.tolist(), dtype=float)
    gamma = np.linalg.lstsq(matrix, b, rcond=None)[0]
    slopes = [mp.mpf(float(gamma[j]))*high(unit)/scales[j] for j in range(1,len(beta))]
    original = [high(ym)+mp.mpf(float(gamma[0]))*high(unit)-
                mp.fsum(slopes[j-1]*high(centres[j]) for j in range(1,len(beta)))] + slopes
    error = max(abs(v-high(r))/abs(high(r)) if r else abs(v) for v,r in zip(original,beta))
    return {'method': 'numpy.linalg.lstsq on independently materialised scaled weighted design',
            'maximum_coefficient_relative_error': float(error)}


def reference(case):
    assert case['relative_tolerance'] == (3e-4 if case['name'].startswith('reference_near_') else 1e-10)
    data = case['input']
    coding = case['coding']
    if coding['kind'] == 'treatment':
        coded = [[row[j] for j in coding['numeric_columns']] +
                 [float(row[factor['column']] == level) for factor in coding['factors']
                  for level in factor['levels'] if level != factor['reference']]
                 for row in coding['source_rows']]
        assert data['x'] == coded, 'Treatment columns differ from the declared references.'
    x = [[Q(1)] + [Q(float(v)) for v in row] for row in data['x']]
    y, w = [Q(float(v)) for v in data['y']], data['w']
    m, n, p = len(x), sum(w), len(x[0])
    assert len(y) == len(w) == m and all(len(r) == p for r in x)
    assert all(type(k) is int and k > 0 for k in w) and n <= 2**53
    assert len(case['coding']['terms']) == p
    base = {'physical_rows': m, 'frequency': n, 'terms': p}
    if p > 64:
        return dict(base, mathematical_status='not_evaluated_operational_size', fit=None)
    gram = [[sum(k*r[i]*r[j] for r,k in zip(x,w)) for j in range(p)] for i in range(p)]
    inv, rank = inverse(gram)
    if inv is None:
        return dict(base, mathematical_status='unidentified', mathematical_rank=rank, fit=None)
    rhs = [sum(k*r[j]*v for r,v,k in zip(x,y,w)) for j in range(p)]
    beta = [sum(r*v for r,v in zip(row,rhs)) for row in inv]
    fitted = [sum(r*b for r,b in zip(row,beta)) for row in x]
    residual = [v-f for v,f in zip(y,fitted)]
    ym = sum(k*v for k,v in zip(w,y))/n
    sse = sum(k*v*v for k,v in zip(w,residual))
    sst = sum(k*(v-ym)**2 for k,v in zip(w,y))
    explained = sum(k*(v-ym)**2 for k,v in zip(w,fitted))
    assert sse + explained == sst
    df, exact_zero = n-p, sse == 0
    a, centres, scales = scaled_design(x,w)
    singular = mp.svd(a, compute_uv=False)
    condition = singular[0]/singular[len(singular)-1]
    declared_rank = sum(v > singular[0]*mp.mpf('1e-12') for v in singular)
    variance = sse/df if df else None
    unavailable = NO_DF if not df else ZERO if exact_zero else None
    covariance = [[estimate(variance*v) if df else estimate(reason=NO_DF) for v in row] for row in inv]
    coefficients = []
    for j,b in enumerate(beta):
        se = mp.sqrt(high(variance*inv[j][j])) if df else None
        t = high(b)/se if not unavailable else None
        coefficients.append({'estimate': estimate(b),
            'standard_error': estimate(se) if df else estimate(reason=NO_DF),
            't': estimate(t) if not unavailable else estimate(reason=unavailable),
            'p': estimate(student_p(t,df)) if not unavailable else estimate(reason=unavailable),
            'intervals': {str(c): [estimate(high(b)-critical(df,c)*se),
                                    estimate(high(b)+critical(df,c)*se)] if df else
                                   [estimate(reason=NO_DF),estimate(reason=NO_DF)] for c in [90,95,99]}})
    f = high(explained)/high(sse)*df/(p-1) if not unavailable else None
    statistics = {'sse': estimate(sse), 'sst': estimate(sst), 'model_ss': estimate(explained),
        'residual_standard_error': estimate(mp.sqrt(high(variance))) if df else estimate(reason=NO_DF),
        'r_squared': estimate(explained/sst) if sst else estimate(reason=CONSTANT),
        'adjusted_r_squared': estimate(1-variance/(sst/(n-1))) if df and sst else
                             estimate(reason=NO_DF if not df else CONSTANT),
        'f': estimate(f) if not unavailable else estimate(reason=unavailable),
        'f_p': estimate(beta_tail(mp.mpf(p-1)/2,mp.mpf(df)/2,df/(df+(p-1)*f)))
               if not unavailable else estimate(reason=unavailable)}
    truth = dict(base, mathematical_status='full_rank', mathematical_rank=rank,
        policy_rank_on_high_precision_scaled_design=declared_rank,
        mathematical_exact_zero_residual=exact_zero,
        computed_zero_requirement='not_applicable' if case['policy']['status'] != 'fit' else
                                  'exact_zero' if len(set(y)) == 1 else
                                  'route_dependent' if exact_zero else 'positive',
        independent_scaled_condition=estimate(condition),
        fit={'rank': p, 'model_df': p-1, 'residual_df': df, 'total_df': n-1,
             'coefficients': coefficients, 'statistics': statistics,
             'covariance': covariance, 'fitted': list(map(estimate,fitted)),
             'residual': list(map(estimate,residual)), 'variance': estimate(variance) if df else estimate(reason=NO_DF)})
    if exact_zero and case['policy']['status']=='fit':
        truth['route_reference']={
            'inverse_gram':[[estimate(v) for v in row] for row in inv],
            'critical_values':{str(c):estimate(critical(df,c)) for c in [90,95,99]} if df else {},
            'student_tail_limit':estimate(2*mp.gamma(mp.mpf(df+1)/2)*mp.power(df,mp.mpf(df-2)/2)/(mp.sqrt(mp.pi)*mp.gamma(mp.mpf(df)/2))) if df else None,
            'interpretation':'Exact original-design inverse and Student constants; conditional inference uses independently checked retained computed residuals. No mathematical zero is imposed on a positive route.'}
    if case['name'] in PROPAGATED_CASES:
        band = mp.mpf('0.0003')
        truth['probability_input_bands'] = {
            'relative_input_band': 3e-4,
            'computed_input_tail_relative_tolerance': 1e-10,
            'coefficients': [[estimate(student_p(abs(mp.mpf(c['t']['reference']))*(1+band),df)),
                              estimate(student_p(abs(mp.mpf(c['t']['reference']))*(1-band),df))]
                             for c in coefficients],
            'f_p': [estimate(beta_tail(mp.mpf(p-1)/2,mp.mpf(df)/2,
                                       df/(df+(p-1)*f*(1+band)))),
                    estimate(beta_tail(mp.mpf(p-1)/2,mp.mpf(df)/2,
                                       df/(df+(p-1)*f*(1-band))))],
            'interpretation': 'Only named near-reference inputs have the accepted 3e-4 statistic band. Probability accuracy is conditional on that input; each tail still requires 1e-10 at its actual log input.'}
    if case['policy']['status'] == 'fit' and condition < mp.mpf('1e8') and not exact_zero:
        truth['numpy_cross_check'] = numpy_check(a,y,w,centres,scales,beta)
        assert truth['numpy_cross_check']['maximum_coefficient_relative_error'] < 1e-10
    return truth


def compare(actual, expected, rtol, path=''):
    """Strict output checker: required fields, statuses, reasons and scalar ranges."""
    if isinstance(expected,dict) and set(expected) == {'status','reference','value','reason'}:
        assert isinstance(actual,dict) and set(actual) == {'status','value','reason'}, path
        assert actual['status'] == expected['status'] and actual['reason'] == expected['reason'], path
        if expected['status'] == 'unavailable':
            assert actual['value'] is None, path
            return
        got = actual['value']
        assert type(got) in (float,int) and math.isfinite(got), path
        want = mp.mpf(expected['reference'])
        if want:
            assert got != 0, path
        allowance = mp.mpf(math.ulp(0.0)) if 0 < abs(want) < sys.float_info.min else abs(want)*rtol
        assert abs(mp.mpf(got)-want) <= allowance, (path,got,str(want))
    elif isinstance(expected,dict):
        assert isinstance(actual,dict) and set(actual) == set(expected), path
        for key,value in expected.items():
            compare(actual[key],value,rtol,path+'/'+key)
    elif isinstance(expected,list):
        assert isinstance(actual,list) and len(actual) == len(expected), path
        for i,(got,want) in enumerate(zip(actual,expected)):
            compare(got,want,rtol,path+f'/{i}')
    else:
        assert type(actual) is type(expected) and actual == expected, path


def rounded(reference):
    if isinstance(reference,dict):
        return {k:rounded(v) for k,v in reference.items() if k != 'reference'}
    return list(map(rounded,reference)) if isinstance(reference,list) else reference


def self_check(cases):
    by_name = {c['name']:c for c in cases}
    for case in cases:
        if case['reference'].get('fit'):
            compare(rounded(case['reference']['fit']),case['reference']['fit'],case['relative_tolerance'])
    simple = by_name['simple']['reference']['fit']
    t = mp.mpf(simple['coefficients'][1]['t']['reference'])
    assert abs(mp.mpf(simple['statistics']['f']['reference'])-t*t) < mp.mpf('1e-170')
    weighted = by_name['weighted']['reference']['fit']
    expanded = by_name['expanded']['reference']['fit']
    for key in weighted:
        literal = [v for v,k in zip(weighted[key],by_name['weighted']['input']['w'])
                   for _ in range(k)] if key in ['fitted','residual'] else weighted[key]
        assert literal == expanded[key], key
    factor_fits = [by_name['two_factors_reference_'+r]['reference']['fit'] for r in ['A','B','C']]
    assert all(f['fitted'] == factor_fits[0]['fitted'] and f['statistics'] == factor_fits[0]['statistics'] for f in factor_fits)
    # A to B: intercept gains B, A contrast is -B, C contrast is C-B.
    transform = mp.eye(6)
    transform[0,3],transform[3,3],transform[4,3] = 1,-1,-1
    covariance = mp.matrix([[mp.mpf(v['reference']) for v in row] for row in factor_fits[0]['covariance']])
    recoded = transform * covariance * transform.T
    for i in range(6):
        for j in range(6):
            assert abs(recoded[i,j]-mp.mpf(factor_fits[1]['covariance'][i][j]['reference'])) < mp.mpf('1e-170')
    tiny = by_name['scale_1e-162_frequency_1125899906842624']['reference']['fit']
    assert tiny['variance']['reason'] == UNDERFLOW
    assert tiny['coefficients'][1]['standard_error']['value'] > 0
    mutations = []
    for case_name,field in [('simple','coefficients'),('simple','statistics'),('simple','covariance')]:
        case = by_name[case_name]; wanted = case['reference']['fit']; bad = rounded(wanted)
        del bad[field]; mutations.append((bad,wanted,case['relative_tolerance'],'missing-'+field))
    wanted = by_name['scale_1e-161_frequency_1']['reference']['fit']
    for value in [None,float('nan'),float('inf'),0.0]:
        bad = rounded(wanted); bad['coefficients'][0]['standard_error']['value'] = value
        mutations.append((bad,wanted,1e-10,'tiny-SE-'+str(value)))
    bad = rounded(tiny); bad['variance']['status'] = 'available'
    mutations.append((bad,tiny,1e-10,'underflow-status'))
    bad = rounded(tiny); bad['variance']['reason'] = ZERO
    mutations.append((bad,tiny,1e-10,'false-exact-zero'))
    for bad,wanted,tol,label in mutations:
        try:
            compare(bad,wanted,tol)
        except AssertionError:
            continue
        raise AssertionError('Accepted mutation '+label)
    return len(mutations)


def check_computed_tails(fixture, outputs):
    cases = {c['name']: c for c in fixture['cases']
             if 'probability_input_bands' in c['reference']}
    expected = {(name, j) for name, c in cases.items()
                for j in range(c['reference']['terms'])}
    student, omnibus = outputs
    assert len(student) == len(expected)
    assert {(r['name'],r['coefficient']) for r in student} == expected
    assert len(omnibus) == len(cases) and {r['name'] for r in omnibus} == set(cases)
    errors = []
    for row in student + omnibus:
        case = cases[row['name']]; reference = case['reference']; fit = reference['fit']
        bounds = reference['probability_input_bands']
        if 'coefficient' in row:
            assert set(row) == {'name','coefficient','df','t','log_hi','log_lo','p','reason'}
            assert row['df'] == fit['residual_df']
            assert math.isfinite(row['log_hi']) and math.isfinite(row['log_lo'])
            log = mp.mpf(row['log_hi']) + mp.mpf(row['log_lo'])
            statistic = mp.exp(log)
            true = abs(mp.mpf(fit['coefficients'][row['coefficient']]['t']['reference']))
            assert math.isfinite(row['t']) and abs(abs(mp.mpf(row['t']))/statistic-1) <= mp.mpf('1e-10')
            probability = student_p(statistic,row['df'])
            interval = bounds['coefficients'][row['coefficient']]
        else:
            assert set(row) == {'name','df1','df2','f','log_f','p','reason'}
            assert row['df1'] == fit['model_df'] and row['df2'] == fit['residual_df']
            assert math.isfinite(row['log_f']) and math.isfinite(row['f'])
            statistic = mp.exp(mp.mpf(row['log_f']))
            true = mp.mpf(fit['statistics']['f']['reference'])
            assert abs(mp.mpf(row['f'])/statistic-1) <= mp.mpf('1e-10')
            a,b = mp.mpf(row['df1'])/2,mp.mpf(row['df2'])/2
            probability = beta_tail(a,b,b/(b+a*statistic))
            interval = bounds['f_p']
        assert abs(statistic/true-1) <= mp.mpf('0.0003')
        compare({'status':'available','value':row['p'],'reason':row['reason']},estimate(probability),1e-10,row['name'])
        assert mp.mpf(interval[0]['reference']) <= row['p'] <= mp.mpf(interval[1]['reference'])
        errors.append(float(abs(mp.mpf(row['p'])/probability-1)))
    return max(errors)


def check_intervals(fixture,rows):
    cases={c['name']:c for c in fixture['cases'] if c['name'] in PROPAGATED_CASES}
    expected={(name,j,c) for name,case in cases.items()
              for j in range(case['reference']['terms']) for c in ['90','95','99']}
    assert len(rows)==len(expected) and {(r['name'],r['coefficient'],r['confidence']) for r in rows}==expected
    errors=[]
    for row in rows:
        assert set(row)=={'name','coefficient','confidence','interval'}
        actual=row['interval'];assert isinstance(actual,list) and len(actual)==2
        expected=cases[row['name']]['reference']['fit']['coefficients'][row['coefficient']]['intervals'][row['confidence']]
        values=[];references=[]
        for got,want in zip(actual,expected):
            assert set(got)=={'status','value','reason'}
            assert got['status']=='available' and got['reason'] is None and type(got['value']) in (float,int)
            assert math.isfinite(got['value'])
            reference=mp.mpf(want['reference'])
            assert not reference or got['value']!=0
            values.append(mp.mpf(got['value']));references.append(reference)
        error=mp.sqrt(mp.fsum((a-b)**2 for a,b in zip(values,references)))
        norm=mp.sqrt(mp.fsum(v*v for v in references))
        assert error<=mp.mpf('0.0003')*norm
        errors.append(float(error/norm))
    return max(errors)

def norm_error(actual, expected):
    assert isinstance(actual,list) and len(actual)==len(expected)
    values=[];references=[]
    for got,want in zip(actual,expected):
        assert set(got)=={'status','value','reason'}
        assert got['status']==want['status']=='available' and got['reason'] is None
        assert type(got['value']) in (int,float) and math.isfinite(got['value'])
        reference=mp.mpf(want['reference'])
        assert not reference or got['value']!=0
        values.append(mp.mpf(got['value']));references.append(reference)
    norm=mp.sqrt(mp.fsum(v*v for v in references))
    error=mp.sqrt(mp.fsum((a-b)**2 for a,b in zip(values,references)))
    assert norm
    return float(error/norm)


def check_fields(fixture,rows):
    cases={c['name']:c for c in fixture['cases'] if c['name'] in PROPAGATED_CASES}
    wanted={}
    for name,c in cases.items():
        fit=c['reference']['fit'];p=c['reference']['terms']
        wanted.update({(name,f,j):[fit['coefficients'][j][f]]
                       for j in range(p) for f in ['estimate','standard_error','t','p']})
        wanted.update({(name,f,None):[v] for f,v in fit['statistics'].items()})
        wanted.update({(name,'covariance',j):fit['covariance'][j] for j in range(p)})
        wanted.update({(name,f,None):fit[f] for f in ['fitted','residual']})
    assert len(rows)==len(wanted)
    assert {(r['name'],r['field'],r['index']) for r in rows}==set(wanted)
    observations={};covariances={name:[] for name in cases}
    for row in rows:
        assert set(row)=={'name','field','index','values'}
        key=(row['name'],row['field'],row['index']);name,field,index=key
        expected=wanted[key];actual=row['values']
        error=norm_error(actual,expected)
        observations['/'.join(str(v) for v in key)]=error
        if field=='covariance':
            covariances[name].append((index,actual,expected))
            observations[f'{name}/covariance_diagonal/{index}']=norm_error([actual[index]],[expected[index]])
        elif field in ['p','f_p']:
            bands=cases[name]['reference']['probability_input_bands']
            bounds=bands['coefficients'][index] if field=='p' else bands['f_p']
            assert mp.mpf(bounds[0]['reference'])<=actual[0]['value']<=mp.mpf(bounds[1]['reference'])
        else:
            assert error<=3e-4,(key,error)
    for name,rows in covariances.items():
        rows.sort(key=lambda r:r[0])
        error=norm_error([v for _,a,_ in rows for v in a],[v for _,_,b in rows for v in b])
        assert error<=3e-4,(name,'whole covariance',error)
        observations[f'{name}/covariance_whole_matrix']=error
    return observations


def reject_field_mutations(fixture,rows):
    count=0
    # Exercise each distinct field, including every covariance row and diagonal.
    for position,row in enumerate(rows):
        for mutation in ['missing','null','nonfinite','zero','status','reason','perturbed']:
            bad=copy.deepcopy(rows);values=bad[position]['values']
            if mutation=='missing':values.pop()
            elif mutation=='null':values[0]['value']=None
            elif mutation=='nonfinite':values[0]['value']=float('nan')
            elif mutation=='zero':values[0]['value']=0.
            elif mutation=='status':values[0]['status']='unavailable'
            elif mutation=='reason':values[0]['reason']='corrupted'
            else:
                if row['field']=='covariance':
                    group=[v['value'] for r in rows if r['name']==row['name'] and r['field']=='covariance' for v in r['values']]
                else:group=[v['value'] for v in values]
                values[0]['value']+=0.01*math.hypot(*group)
            try:check_fields(fixture,bad)
            except (AssertionError,TypeError):count+=1;continue
            raise AssertionError(f'Accepted field mutation {row["name"]}/{row["field"]}/{row["index"]}/{mutation}')
    return count


def route_reference(case, primary):
    assert set(primary)=={'scales','design','outcome','gamma','residual'}
    data=case['input'];weights=data['w'];n=sum(weights);m=len(weights);p=len(data['x'][0])+1;df=n-p
    eps=mp.mpf(2)**-52
    g=lambda k:k*eps/(1-k*eps)
    columns=list(map(list,zip(*data['x'])))+[data['y']]
    assert len(primary['scales'])==p and len(primary['design'])==m
    assert len(primary['outcome'])==len(primary['residual'])==m and len(primary['gamma'])==p
    normalized=[]
    for raw,scale in zip(columns,primary['scales']):
        assert len(scale)==4 and all(type(v) in (int,float) and math.isfinite(v) for v in scale)
        anchor,delta,mean,sd=map(mp.mpf,scale);raw=list(map(mp.mpf,raw))
        assert anchor==raw[0] and delta>0 and sd>0
        spread=max(abs(v-anchor) for v in raw)
        assert delta==(spread if spread else 1)
        exact_mean=mp.fsum(w*(v-anchor)/delta for w,v in zip(weights,raw))/n
        mean_bound=(g(m)+g(2))*mp.fsum(w*abs((v-anchor)/delta) for w,v in zip(weights,raw))/n
        assert abs(mean-exact_mean)<=mean_bound
        exact_sd=mp.sqrt(mp.fsum(w*((v-anchor)/delta-mean)**2 for w,v in zip(weights,raw))/n)
        if spread:assert abs(sd/exact_sd-1)<=g(4*m+4)
        else:assert mean==0 and sd==1
        normalized.append([((v-anchor)/delta-mean)/sd for v in raw])
    z=primary['design'];ys=primary['outcome'];residual=primary['residual'];gamma=primary['gamma']
    for i in range(m):
        assert len(z[i])==p and z[i][0]==1
        for j in range(p):
            actual=ys[i] if j==p-1 else z[i][j+1]
            assert type(actual) in (int,float) and math.isfinite(actual)
            anchor,delta,mean,sd=map(mp.mpf,primary['scales'][j])
            raw=mp.mpf(columns[j][i])
            # Four operations: subtraction, division, subtraction, division.
            bound=g(4)*((abs(raw)+abs(anchor))/delta+abs(mean))/sd
            assert abs(mp.mpf(actual)-normalized[j][i])<=bound
    assert all(type(v) in (int,float) and math.isfinite(v) for v in gamma+residual)
    a=mp.matrix([[mp.mpf(v)*mp.sqrt(mp.mpf(w)/n) for v in row] for row,w in zip(z,weights)])
    b=mp.matrix([mp.mpf(y)*mp.sqrt(mp.mpf(w)/n) for y,w in zip(ys,weights)])
    expected_gamma=mp.lu_solve(a.T*a,a.T*b)
    singular=mp.svd(a,compute_uv=False);condition=singular[0]/singular[len(singular)-1]
    # Existing Task 1 physical-row allowance H, condition-scaled as a
    # consistency check against the independent normalized least-squares solve.
    h=64*p*eps+8*g(m)
    bound=h*condition*(mp.norm(expected_gamma)+mp.norm(b)/singular[len(singular)-1])
    assert mp.norm(mp.matrix(gamma)-expected_gamma)<=bound
    predicted=[]
    for row,y,e in zip(z,ys,residual):
        products=[mp.mpf(v)*mp.mpf(c) for v,c in zip(row,gamma)]
        fitted=mp.fsum(products);predicted.append(fitted)
        # p rounded products, compensated accumulation bounded conservatively
        # by 2p operations, and the final residual subtraction: gamma_(2p+2).
        bound=g(2*p+2)*(abs(mp.mpf(y))+mp.fsum(abs(v) for v in products))
        assert abs(mp.mpf(e)-(mp.mpf(y)-fitted))<=bound
    if case['reference']['computed_zero_requirement']=='exact_zero':assert all(e==0 for e in residual)
    if case['name']=='perfect_fit_positive':assert any(e!=0 for e in residual)
    unit=mp.mpf(primary['scales'][-1][1])*mp.mpf(primary['scales'][-1][3])
    centre=mp.mpf(primary['scales'][-1][0])+mp.mpf(primary['scales'][-1][1])*mp.mpf(primary['scales'][-1][2])
    sse=mp.fsum(w*(mp.mpf(e)*unit)**2 for e,w in zip(residual,weights))
    sst=mp.fsum(w*(mp.mpf(y)*unit)**2 for y,w in zip(ys,weights))
    model=mp.fsum(w*(v*unit)**2 for v,w in zip(predicted,weights))
    original=[[Q(1)]+[Q(float(v)) for v in row] for row in data['x']]
    inv,_=inverse([[sum(w*r[j]*r[k] for r,w in zip(original,weights)) for k in range(p)] for j in range(p)])
    variance=sse/df if df else None;reason=NO_DF if not df else ZERO if not sse else None
    reference=case['reference']['fit'];outputs={}
    for confidence in [90,95,99]:
        coeff=[]
        for j,c in enumerate(reference['coefficients']):
            beta=mp.mpf(c['estimate']['reference']);se=mp.sqrt(variance*high(inv[j][j])) if df else None
            t=beta/se if not reason else None
            coeff.append({'estimate':estimate(beta),'standard_error':estimate(se) if df else estimate(reason=NO_DF),
                't':estimate(t) if not reason else estimate(reason=reason),
                'p':estimate(student_p(t,df)) if not reason else estimate(reason=reason),
                'interval':[estimate(beta-critical(df,confidence)*se),estimate(beta+critical(df,confidence)*se)] if df else [estimate(reason=NO_DF)]*2})
        f=model*df/(sse*(p-1)) if not reason else None
        outputs[str(confidence)]={'coefficients':coeff,'statistics':{
            'sse':estimate(sse),'sst':estimate(sst),'model_ss':estimate(model),
            'residual_standard_error':estimate(mp.sqrt(variance)) if df else estimate(reason=NO_DF),
            'r_squared':estimate(model/sst) if sst else estimate(reason=CONSTANT),
            'adjusted_r_squared':estimate(1-variance*(n-1)/sst) if df and sst else estimate(reason=NO_DF if not df else CONSTANT),
            'f':estimate(f) if not reason else estimate(reason=reason),
            'f_p':estimate(beta_tail(mp.mpf(p-1)/2,mp.mpf(df)/2,df/(df+(p-1)*f))) if not reason else estimate(reason=reason)},
            'covariance':[[estimate(variance*high(v)) if df else estimate(reason=NO_DF) for v in row] for row in inv],
            'fitted':[estimate(centre+unit*v) for v in predicted],
            'residual':[estimate(mp.mpf(e)*unit) for e in residual]}
    return outputs


def check_routes(fixture,lines):
    cases={c['name']:c for c in fixture['cases'] if 'route_reference' in c['reference']}
    rows=[json.loads(line.split('FIT_ROUTE_RESULT ',1)[1]) for line in lines if 'FIT_ROUTE_RESULT ' in line]
    assert len(rows)==len(cases) and {r['name'] for r in rows}==set(cases)
    count=0
    def leaves(value,path=()):
        if isinstance(value,dict) and 'status' in value:yield path;return
        for k,v in (value.items() if isinstance(value,dict) else enumerate(value)):
            yield from leaves(v,path+(k,))
    for row in rows:
        assert set(row)=={'name','primary','outputs'}
        expected=route_reference(cases[row['name']],row['primary'])
        compare(row['outputs'],expected,1e-10,row['name'])
        for path in leaves(row['outputs']):
            for mutation in ['missing','null','nonfinite','zero_or_one','status','reason','perturbed']:
                bad=copy.deepcopy(row['outputs']);value=bad
                for k in path:value=value[k]
                if mutation=='missing':del value['value']
                elif mutation=='null':
                    if value['value'] is None:value['value']=1.
                    else:value['value']=None
                elif mutation=='nonfinite':value['value']=float('nan')
                elif mutation=='zero_or_one':value['value']=0. if value['value'] else 1.
                elif mutation=='status':value['status']='corrupted'
                elif mutation=='reason':value['reason']='corrupted'
                else:value['value']=value['value']*1.01 if value['value'] else 1.
                try:compare(bad,expected,1e-10)
                except (AssertionError,TypeError):count+=1;continue
                raise AssertionError(f'Accepted route corruption {row["name"]}/{path}/{mutation}')
        # Independently reject meaningfully corrupted primary normalization,
        # coefficients and residuals before using them as conditional inputs.
        for field in ['design','outcome','gamma','residual','scales']:
            bad=copy.deepcopy(row['primary'])
            if field in ['design','scales']:bad[field][0][1]+=0.1
            else:bad[field][0]+=0.1
            try:route_reference(cases[row['name']],bad)
            except (AssertionError,ZeroDivisionError):count+=1;continue
            raise AssertionError(f'Accepted route input corruption {row["name"]}/{field}')
        energy=mp.fsum(mp.mpf(w)*mp.mpf(e)**2 for w,e in zip(cases[row['name']]['input']['w'],row['primary']['residual']))/sum(cases[row['name']]['input']['w'])
        print(f'ROUTE {row["name"]}: normalized residual norm {mp.nstr(mp.sqrt(energy),18)}; all three confidence levels checked')
    print(f'PASS {len(rows)} exact-algebraic routes; {count} rejected output/input corruptions')


def check_result_file(fixture,path):
    lines = path.read_text().splitlines()
    check_routes(fixture,lines)
    outputs = tuple([json.loads(line.split(prefix,1)[1]) for line in lines if prefix in line]
                    for prefix in ['FIT_TAIL_RESULT ','FIT_F_RESULT '])
    error = check_computed_tails(fixture,outputs)
    intervals=[json.loads(line.split('FIT_INTERVAL_RESULT ',1)[1]) for line in lines if 'FIT_INTERVAL_RESULT ' in line]
    interval_error=check_intervals(fixture,intervals)
    fields=[json.loads(line.split('FIT_FIELD_RESULT ',1)[1]) for line in lines if 'FIT_FIELD_RESULT ' in line]
    observations=check_fields(fixture,fields)
    field_mutations=reject_field_mutations(fixture,fields)
    mutations = []
    missing = copy.deepcopy(outputs); missing[0].pop(); mutations.append(missing)
    for key,value in [('p',None),('p',0.),('p',float('nan')),('p',outputs[0][0]['p']*1.00001),
                      ('log_hi',outputs[0][0]['log_hi']+0.01),('reason','wrong status')]:
        bad = copy.deepcopy(outputs); bad[0][0][key] = value; mutations.append(bad)
    bad = copy.deepcopy(outputs); bad[1][0]['p'] *= 1.00001; mutations.append(bad)
    for bad in mutations:
        try:
            check_computed_tails(fixture,bad)
        except (AssertionError,TypeError):
            continue
        raise AssertionError('Accepted a computed-input probability corruption')
    interval_mutations=[]
    for mutation in ['missing','null','nonfinite','zero','reason','perturbed']:
        bad=copy.deepcopy(intervals)
        if mutation=='missing': bad[0]['interval'].pop()
        elif mutation=='null': bad[0]['interval'][0]['value']=None
        elif mutation=='nonfinite': bad[0]['interval'][0]['value']=float('nan')
        elif mutation=='zero': bad[0]['interval'][0]['value']=0.
        elif mutation=='reason': bad[0]['interval'][0]['reason']='corrupted'
        else:
            scale=math.hypot(*(v['value'] for v in bad[0]['interval']))
            bad[0]['interval'][0]['value']+=0.01*scale
        interval_mutations.append(bad)
    for bad in interval_mutations:
        try:
            check_intervals(fixture,bad)
        except (AssertionError,TypeError):
            continue
        raise AssertionError('Accepted a named interval corruption')
    print(json.dumps({'named_field_relative_errors':observations},indent=2,sort_keys=True,allow_nan=False))
    print(f'PASS {len(fields)} named field records; {field_mutations} rejected field corruptions')
    print(f'PASS {len(intervals)} named intervals; maximum normwise relative error {interval_error:.3e}; {len(interval_mutations)} rejected corruptions')
    print(f'PASS {len(outputs[0])} Student and {len(outputs[1])} F tails at actual log inputs; maximum relative error {error:.3e}; {len(mutations)} rejected corruptions')

def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--fixture',type=Path,default=DEFAULT)
    parser.add_argument('--write',action='store_true')
    parser.add_argument('--results',type=Path)
    args = parser.parse_args()
    assert np.__version__ == '2.0.2' and mp.__version__ == '1.3.0'
    saved = json.loads(args.fixture.read_text())
    if args.results:
        check_result_file(saved,args.results)
        return
    cases = []
    for old in saved['cases']:
        case = {k:v for k,v in old.items() if k != 'reference'}
        case['reference'] = reference(case)
        cases.append(case)
    result = {'generator_version':1,'numpy_version':np.__version__,'mpmath_version':mp.__version__,
        'decimal_precision':DPS,
        'input_semantics':'Promote actual binary64 x/y values exactly; integer weights and N remain exact.',
        'formulae':'beta=(X\u2032WX)^-1 X\u2032Wy; covariance=SSE/(N-p)*(X\u2032WX)^-1; df=N-p; two-sided Student t; F=(model_ss/(p-1))/(SSE/(N-p)).',
        'comparison_policy':'Ordinary scalar relative error 1e-10; representable subnormal 1 ULP; nonzero values cannot become zero. The 3e-4 band applies only to named near-reference recodings from Task 1, never a runtime gate. Named near-reference probabilities use monotone bounds propagated from that statistic band, with separate 1e-10 checks at computed log inputs. Each named near-reference confidence interval uses the accepted 3e-4 two-endpoint normwise check, stricter than the historical whole-field check; named covariance uses the historical whole-matrix 3e-4 normwise rule, fitted/residual vectors use their whole-vector norm, and every entry remains required and finite with exact status/reason and false-zero checks. Row and diagonal errors are recorded as observations, not new acceptance bands.',
        'zero_policy':'Mathematical exact-zero fields describe the algebraic limit; the kernel may retain positive computed residual energy. Test exact-zero inference using actual kernel energy and do not enforce this limit against a positive route.',
        'rank_rule_controls':[
            {'singular_values':[1.0,1e-12],'expected_rank':1},
            {'singular_values':[1.0,math.nextafter(1e-12,math.inf)],'expected_rank':2},
            {'singular_values':[1.0,math.nextafter(1e-12,0.0)],'expected_rank':1}],
        'warning_rule_controls':[
            {'condition':1e8,'warned':False},
            {'condition':math.nextafter(1e8,math.inf),'warned':True},
            {'condition':math.nextafter(1e8,0.0),'warned':False}],
        'cases':cases}
    mutations = self_check(cases)
    if args.write:
        args.fixture.write_text(json.dumps(result,indent=2,allow_nan=False)+'\n')
        assert json.loads(args.fixture.read_text()) == result
    else:
        assert result == saved, 'Fixture differs from independently regenerated references.'
    print(f'PASS {len(cases)} independent cases; {mutations} rejected corruptions; NumPy {np.__version__}; mpmath {mp.__version__}; {DPS} digits')


if __name__ == '__main__':
    main()
