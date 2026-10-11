#!/usr/bin/env python3
"""Independent exact Float64 reliability controls; no production tolerance policy."""
import argparse
from fractions import Fraction as F
import hashlib
import json
import math
from pathlib import Path
import platform
import sys


def rational(x):
    return F.from_float(float(x))


def encode(q):
    return None if q is None else f'{q.numerator}/{q.denominator}'


def sqrt_exact(q):
    a, b = math.isqrt(q.numerator), math.isqrt(q.denominator)
    return F(a, b) if a*a == q.numerator and b*b == q.denominator else None


def cohort(case):
    """Validate filtered positive-frequency base before original completeness."""
    result = []
    k = len(case['rows'][0]['values']) if case['rows'] else case['k']
    assert k >= 2
    base_n = 0
    reversals = case.get('reversals', {})
    for key,bounds in reversals.items():
        if not str(key).isdigit() or not 0 <= int(key) < k or len(bounds)!=2:
            raise ValueError('invalid reverse record')
        lo,hi=map(float,bounds)
        if not (math.isfinite(lo) and math.isfinite(hi) and lo<hi):
            raise ValueError('invalid reverse bounds')
    for row in case['rows']:
        if not row.get('include', True):
            continue
        w = row.get('weight', 1)
        if w is None or w == 0:
            continue
        if not isinstance(w, int) or isinstance(w, bool) or w < 0:
            raise ValueError('invalid frequency')
        base_n += w
        if base_n > 2**53:
            raise ValueError('base frequency ceiling')
        values = list(row['values'])
        for key, bounds in reversals.items():
            i = int(key)
            lo, hi = map(float, bounds)
            if not (math.isfinite(lo) and math.isfinite(hi) and lo < hi):
                raise ValueError('invalid reverse bounds')
            if values[i] is not None:
                x = float(values[i])
                if not lo <= x <= hi:
                    raise ValueError('reverse bounds')
                values[i] = float(rational(lo) + rational(hi) - rational(x))
        if all(x is not None for x in values):
            if not all(math.isfinite(float(x)) for x in values):
                raise ValueError('nonfinite item')
            result.append((tuple(map(rational, values)), w))
    return result, k


def covariance(rows, k):
    n = sum(w for _, w in rows)
    means = [sum((w*x[i] for x,w in rows), F())/n for i in range(k)] if n else [None]*k
    g = [[sum((w*(x[i]-means[i])*(x[j]-means[j]) for x,w in rows), F())
          for j in range(k)] for i in range(k)] if n else [[F()]*k for _ in range(k)]
    return n, means, g


def direct_energy(rows, indices):
    n = sum(w for _,w in rows)
    if not n:
        return F()
    totals = [(sum((x[i] for i in indices), F()),w) for x,w in rows]
    mean = sum((w*t for t,w in totals),F())/n
    return sum((w*(t-mean)**2 for t,w in totals),F())


def alpha(diagonal, energy, k, n):
    return F(k,k-1)*(1-diagonal/energy) if n >= 2 and k >= 2 and energy > 0 else None


def exact_controls(rows, k):
    n, means, g = covariance(rows,k)
    indices = list(range(k))
    total = direct_energy(rows,indices)
    assert total == sum(map(sum,g),F())
    raw = alpha(sum((g[i][i] for i in indices),F()),total,k,n)
    deletions, corrected = [], []
    for i in indices:
        retained = [j for j in indices if j != i]
        energy = direct_energy(rows,retained)
        assert energy == sum((g[j][l] for j in retained for l in retained),F())
        deletions.append(alpha(sum((g[j][j] for j in retained),F()),energy,k-1,n))
        cross = sum((g[i][j] for j in retained),F())
        # Independent centered item versus directly formed remaining totals.
        if n:
            mt = sum((w*sum((x[j] for j in retained),F()) for x,w in rows),F())/n
            direct_cross = sum((w*(x[i]-means[i])*(sum((x[j] for j in retained),F())-mt)
                                for x,w in rows),F())
            assert direct_cross == cross
        corrected.append({'sign': (cross>0)-(cross<0),
                          'squared': encode(cross*cross/(g[i][i]*energy))}
                         if n>=2 and g[i][i]>0 and energy>0 else None)
    return {'n':n,'physical_rows':len(rows),'means':list(map(encode,means)),
            'sample_variances':[encode(g[i][i]/(n-1)) if n>=2 else None for i in indices],
            'scatter':[[encode(q) for q in row] for row in g],
            'total_energy':encode(total),'raw_alpha':encode(raw),
            'raw_deletion':list(map(encode,deletions)),'corrected':corrected}, g, means


def standardized(rows, indices, g, means, mp):
    n = sum(w for _,w in rows)
    m = len(indices)
    if n<2 or m<2 or any(g[i][i]==0 for i in indices):
        return None
    def convert(q):
        return mp.mpf(q.numerator)/q.denominator
    # Group square classes: distinct rational square roots are linearly independent.
    groups = []
    for i in indices:
        for base, members in groups:
            ratio = sqrt_exact(g[i][i]/g[base][base])
            if ratio is not None:
                members.append((i,ratio))
                break
        else:
            groups.append((i,[(i,F(1))]))
    if all(sum(((x[i]-means[i])/ratio for i,ratio in members),F()) == 0
           for x,w in rows for base,members in groups):
        return None
    roots = [sqrt_exact(g[i][i]) for i in indices]
    if all(q is not None for q in roots):
        energy = sum((w*sum(((x[i]-means[i])/s for i,s in zip(indices,roots)),F())**2
                      for x,w in rows),F())
        if energy == 0:
            return None
    direct = mp.fsum(w*mp.fsum(convert(x[i]-means[i])/mp.sqrt(convert(g[i][i]))
                             for i in indices)**2 for x,w in rows)
    matrix = mp.fsum(convert(g[i][j])/mp.sqrt(convert(g[i][i])*convert(g[j][j]))
                    for i in indices for j in indices)
    allowance = mp.eps * 1000 * max(1,abs(direct),abs(matrix))
    assert abs(direct-matrix) <= allowance
    assert direct > 0
    if len(groups)==1:
        base,members=groups[0]
        energy=sum((w*sum(((x[i]-means[i])/ratio for i,ratio in members),F())**2 for x,w in rows),F())/g[base][base]
        return convert(F(m,m-1)*(1-F(m)/energy))
    return mp.mpf(m)/(m-1)*(1-m/direct)


def self_checks(mp):
    def rows(values):
        return [(tuple(map(rational,x)),1) for x in values]
    result,g,means = exact_controls(rows([[0,0],[1,2]]),2)
    assert result['raw_alpha']=='8/9'
    assert result['corrected']==[{'sign':1,'squared':'1/1'}]*2
    assert result['raw_deletion']==[None,None]
    with mp.workdps(160):
        for values, expected in (([[0,0],[1,2]],1), ([[0,3],[1,3]],None),
                                 ([[0,0],[1,-1]],None), ([[0,0],[1,-2]],None),
                                 ([[0,0],[1,1],[2,2],[3,3]],1)):
            selected=rows(values)
            _,mu,gram=covariance(selected,2)
            assert standardized(selected,[0,1],gram,mu,mp)==expected
        selected=rows([[0,0,7],[1,2,7]])
        _,mu,gram=covariance(selected,3)
        assert standardized(selected,[0,1],gram,mu,mp)==1
    # Four observations: sample-SD normalized scatter needs division by N-1.
    r = rows([[0,0],[1,1],[2,2],[3,3]])
    n,mu,g = covariance(r,2)
    wrong_gz = F(n-1)*4
    assert F(2)*(1-F(2,wrong_gz)) != 1
    # Orthogonal items: corrected correlation zero; self-containing covariance positive.
    r = rows([[-1,-1],[-1,1],[1,-1],[1,1]])
    result,g,_ = exact_controls(r,2)
    assert result['corrected'][0]['squared']=='0/1'
    assert g[0][0]+g[0][1] != g[0][1]
    c={'rows':[{'values':[0,0,0]},{'values':[1,1,1]},{'values':[None,9,9]}]}
    selected,k = cohort(c)
    assert len(selected)==2
    wrongly_recovered = [r for r in c['rows'] if all(v is not None for v in r['values'][1:])]
    assert len(wrongly_recovered)!=len(selected)
    for reversals in ({'0':[1,1]},{'2':[0,1]}):
        try:
            cohort({'k':2,'rows':[],'reversals':reversals})
        except ValueError:
            pass
        else:
            raise AssertionError('empty cohort bypassed reverse request validation')


def cases():
    def make(name, values, **extra):
        return {'id':name,'k':len(values[0]) if values else extra.pop('k',2),
                'rows':[{'values':x} for x in values],**extra}
    tiny = float.fromhex('0x0.0000000000001p-1022')
    huge = float.fromhex('0x1.fffffffffffffp+1023')
    out = [make('identical_n4',[[0,0],[1,1],[2,2],[3,3]]),make('identical',[[0,0],[1,1]]),make('scaled',[[0,0],[1,2]]),
           make('constant',[[0,3],[1,3]]),make('opposites',[[0,0],[1,-1]]),
           make('negative_eight',[[0,0],[1,-2]]),make('empty',[],k=2),
           make('one',[[3,4]]),make('constant_removed',[[0,0,7],[1,2,7]]),
           make('zero_remaining',[[0,0,0],[1,1,-1]]),
           make('mixed_units',[[0,0,0],[1,2**500,2**-500],[2,-2**500,2**-499]]),
           make('large_offsets',[[2**52,2**51],[2**52+1,2**51+2],[2**52+2,2**51+4]]),
           make('subnormal',[[0,0],[tiny,2*tiny]]),
           make('huge_sd',[[huge,huge],[-huge,-huge]]),
           make('signed_cancellation',[[0,0,0],[2**500,-2**500,2**-500],[2**501,-2**501,2**-499]]),
           make('near_zero_alpha',[[0,0],[1,1],[1,-1],[0,2**-40]]),
           make('missing_deleted',[[0,0,0],[1,2,3],[None,9,9]]),
           make('weighted',[[0,0],[1,2],[3,-1]]),make('weighted_single',[[4,9]]),
           make('reverse_endpoints',[[0,0],[1,2]],reversals={'0':[0,1]}),
           make('reverse_overflow',[[huge,1],[math.nextafter(huge,0),2]],reversals={'0':[huge/2,huge]})]
    out[-4]['rows'][0]['weight']=2
    out[-4]['rows'][1]['weight']=3
    out[-4]['rows'][2]['weight']=1
    out[-3]['rows'][0]['weight']=2
    out += [make('bounds_incomplete',[[2,None],[0,1]],reversals={'0':[0,1]},error='reverse bounds'),
            make('invalid_weight_incomplete',[[None,1]],error='invalid frequency'),
            make('base_ceiling_incomplete',[[None,1]],error='base frequency ceiling')]
    out[-2]['rows'][0]['weight']=-1
    out[-1]['rows'][0]['weight']=2**53+1
    excluded = make('excluded_bounds',[[2,None],[2,None],[2,None],[0,1],[1,2]],reversals={'0':[0,1]})
    excluded['rows'][0]['include']=False
    excluded['rows'][1]['weight']=0
    excluded['rows'][2]['weight']=None
    out.append(excluded)
    ignored = make('filtered_invalid_weight',[[None,1],[0,0],[1,2]])
    ignored['rows'][0].update(include=False,weight=-1)
    out.append(ignored)
    base=[[0,1,3],[1,3,-2],[2,-1,4],[4,2,1]]
    out += [make('invariance_base',base),
            make('invariance_permuted',[[r[2],r[0],r[1]] for r in base]),
            make('invariance_negative_common',[[-8*v for v in r] for r in base]),
            make('invariance_positive_common',[[8*v for v in r] for r in base]),
            make('invariance_offsets',[[v+16*(i+1) for i,v in enumerate(r)] for r in base]),
            make('invariance_positive_itemwise',[[v*2**i for i,v in enumerate(r)] for r in base]),
            make('explicit_reverse_control',[[1,0],[0,2]])]
    return out


def run(output, evidence):
    import mpmath as mp
    self_checks(mp)
    assert rational(0.1)==F(3602879701896397,36028797018963968)
    with mp.workdps(80):
        assert abs(mp.sqrt(2)**2-2)<mp.mpf('1e-75')
    inventory={'python':sys.version,'executable':sys.executable,'platform':platform.platform(),
               'mpmath':mp.__version__,'fraction_functional':True,'mpmath_functional':True}
    try:
        import numpy as np
        inventory['numpy']={'version':np.__version__,'functional':bool(np.array([1.,2.]).sum()==3.)}
    except Exception as exc:
        inventory['numpy']={'error':repr(exc)}
    records=[]
    for case in cases():
        record={'input':case,'input_float64_hex':[[None if x is None else float(x).hex()
                 for x in row['values']] for row in case['rows']]}
        try:
            rows,k=cohort(case)
        except ValueError as exc:
            assert str(exc)==case.get('error')
            record['expected_error']=str(exc)
            records.append(record)
            continue
        assert 'error' not in case
        exact,g,means=exact_controls(rows,k)
        literal=[(x,1) for x,w in rows for _ in range(w)]
        replicated,_,_=exact_controls(literal,k)
        assert {key:v for key,v in exact.items() if key!='physical_rows'} == {key:v for key,v in replicated.items() if key!='physical_rows'}
        record['scored_float64_hex']=[[float(x).hex() for x in values] for values,_ in rows]
        record['exact']=exact
        energy=F(exact['total_energy'])
        record['exact_total_energy_state']='ExactZero' if energy==0 else ('PositiveBelowFloat64' if energy < rational(float.fromhex('0x0.0000000000001p-1022')) else 'Positive')
        with mp.workdps(640):
            def publication(q, root=False):
                if q is None:
                    return None
                value=mp.mpf(F(q).numerator)/F(q).denominator
                if root:
                    value=mp.sqrt(value)
                rounded=float(value)
                return rounded.hex() if math.isfinite(rounded) else 'unrepresentable'
            record['float64_publication_controls']={'means':[publication(q) for q in exact['means']],
                'sample_sd':[publication(q,True) for q in exact['sample_variances']],
                'raw_alpha':publication(exact['raw_alpha']),
                'raw_deletion':[publication(q) for q in exact['raw_deletion']],
                'corrected':[None if q is None else (float(q['sign']*mp.sqrt(mp.mpf(F(q['squared']).numerator)/F(q['squared']).denominator))).hex() for q in exact['corrected']]}
        record['precision_controls']=[]
        previous=None
        for dps in (80,160,320,640):
            with mp.workdps(dps):
                all_indices=list(range(k))
                values=[standardized(rows,all_indices,g,means,mp)]+[
                    standardized(rows,[j for j in all_indices if j!=i],g,means,mp) for i in all_indices]
                literal_values=[standardized(literal,all_indices,g,means,mp)]+[
                    standardized(literal,[j for j in all_indices if j!=i],g,means,mp) for i in all_indices]
                assert [None if v is None else float(v).hex() for v in literal_values] == [None if v is None else float(v).hex() for v in values]
                rounded=[None if v is None else float(v).hex() for v in values]
                record['precision_controls'].append({'decimal_digits':dps,'standardized_alpha_and_deletions':[
                    None if v is None else mp.nstr(v, min(dps,90)) for v in values], 'rounded_float64':rounded})
                if previous is not None:
                    assert rounded==previous
                previous=rounded
        # Invariance controls use exact scored values; transformations are exactly representable here.
        if case['id']=='scaled':
            for transformed in ([tuple(reversed(x)) for x,w in rows],
                                [tuple(-8*v for v in x) for x,w in rows],
                                [tuple(v+F(i+1) for i,v in enumerate(x)) for x,w in rows]):
                check,_,_=exact_controls(list(zip(transformed,[w for x,w in rows])),k)
                assert check['raw_alpha']==exact['raw_alpha']
            with mp.workdps(80):
                changed=[(tuple(v*F(2**i) for i,v in enumerate(x)),w) for x,w in rows]
                _,cg,cm=exact_controls(changed,k)
                assert abs(standardized(changed,[0,1],cg,cm,mp)-1) <= 1000*mp.eps
        records.append(record)
    by_id={r['input']['id']:r for r in records}
    for name,expected in [('identical','1/1'),('scaled','8/9'),('constant','0/1'),('opposites',None),('negative_eight','-8/1')]:
        assert by_id[name]['exact']['raw_alpha']==expected
    for name,expected in [('identical_n4',1.0.hex()),('identical',1.0.hex()),('scaled',1.0.hex()),('constant',None),('opposites',None),('negative_eight',None)]:
        assert all(control['rounded_float64'][0]==expected for control in by_id[name]['precision_controls'])
    assert all(control['rounded_float64'][3]==1.0.hex() for control in by_id['constant_removed']['precision_controls'])
    assert by_id['negative_eight']['exact']['corrected']==[{'sign':-1,'squared':'1/1'}]*2
    for name in ('invariance_permuted','invariance_negative_common','invariance_positive_common','invariance_offsets'):
        assert by_id[name]['exact']['raw_alpha']==by_id['invariance_base']['exact']['raw_alpha']
    for name in ('invariance_permuted','invariance_negative_common','invariance_positive_common','invariance_offsets','invariance_positive_itemwise'):
        assert by_id[name]['precision_controls'][-1]['rounded_float64'][0]==by_id['invariance_base']['precision_controls'][-1]['rounded_float64'][0]
    for key in ('exact','precision_controls','scored_float64_hex'):
        assert by_id['reverse_endpoints'][key]==by_id['explicit_reverse_control'][key]
    if inventory['numpy'].get('functional'):
        assert np.array_equal(np.cov(np.array([[0.,0.],[1.,2.]]).T,ddof=1),np.array([[.5,1.],[1.,2.]]))
        inventory['numpy']['secondary_covariance_control']=True
    payload={'schema':'independent-reliability-oracle-v1','conventions':{
        'inputs':'Finite Float64 converted by Fraction.from_float; JSON hexadecimal companions are authoritative.',
        'reversal':'Inclusive positive-weight filtered-base bounds; exact L+U-x rounded once to Float64.',
        'cohort':'Original complete cohort retained for all deletions; integer frequency is literal replication.',
        'exact':'Rational means/scatter/raw alpha/correlation squared; standardized square roots use mpmath.',
        'precision':'80/160/320/640 decimal digits; independent direct standardized energy and correlation matrix identity.',
        'limits':'Small fixtures only; convergence is evidence, not a Rust error allowance or correct-rounding proof.'},
        'cases':records}
    output.parent.mkdir(parents=True,exist_ok=True)
    output.write_text(json.dumps(payload,indent=2,allow_nan=False)+'\n')
    evidence.mkdir(parents=True,exist_ok=True)
    inventory['fixture_sha256']=hashlib.sha256(output.read_bytes()).hexdigest()
    inventory['script_sha256']=hashlib.sha256(Path(__file__).read_bytes()).hexdigest()
    inventory['cases']=len(records)
    (evidence/'01-run.json').write_text(json.dumps(inventory,indent=2)+'\n')
    print(json.dumps(inventory,indent=2))


if __name__=='__main__':
    parser=argparse.ArgumentParser()
    parser.add_argument('--output',type=Path,default=Path('tests/fixtures/reliability.json'))
    parser.add_argument('--evidence',type=Path,default=Path('.build/reliability-policy/oracle'))
    args=parser.parse_args()
    run(args.output,args.evidence)
