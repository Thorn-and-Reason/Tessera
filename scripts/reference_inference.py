#!/usr/bin/env python3
"""Generate independent Student-t references; never invoked by Cargo tests.
Run: python3 scripts/reference_inference.py
Requires mpmath 1.3.0. Input numbers are converted from their exact Float64 value.
"""
import json
from pathlib import Path
import mpmath as mp
mp.mp.dps = 75

def tail(t, df):
    x, v = mp.mpf(float(t)), mp.mpf(float(df))
    return tail_exact(x, v)

def tail_exact(x, v):
    if v == 1:
        return 2 * mp.atan(1 / x) / mp.pi
    if v == 2:
        # Rationalized analytical form avoids cancellation at large x.
        return 2 / (mp.sqrt(x*x+2) * (mp.sqrt(x*x+2)+x))
    return mp.betainc(v/2, mp.mpf('0.5'), 0, v/(v+x*x), regularized=True)

cases=[]
for df in [1,1.25,1.5,2,3,30,127.99999999999999,128,128.00000000000003,100000,2**53]:
    for t in [0.01,0.5,1.9999999999999998,2,2.0000000000000004,5,20,38.4,38.5,38.6,1e200]:
        value=tail(t,df)
        cases.append(dict(t=t,df=df,exact=mp.nstr(value,70),rounded=float(value)))
for t,df in [(3e307,1),(1e154,2),(37.6,2**53)]:
    value=tail(t,df)
    cases.append(dict(t=t,df=df,exact=mp.nstr(value,70),rounded=float(value)))
critical=[]
for df in [1,2,3,30,127.99999999999999,128,128.00000000000003,100000,2**53]:
    for confidence,alpha in [('P90','0.10'),('P95','0.05'),('P99','0.01')]:
        a=mp.mpf(alpha)
        # Independent high-precision root finding, with exact mp inputs.
        v=mp.mpf(df)
        def objective(x):
            return mp.betainc(v/2,mp.mpf('0.5'),0,v/(v+x*x),regularized=True)-a
        lower,upper=mp.mpf(0),mp.mpf(128)
        for _ in range(240):
            middle=(lower+upper)/2
            if objective(middle)>0: lower=middle
            else: upper=middle
        value=(lower+upper)/2
        critical.append(dict(df=df,confidence=confidence,exact=mp.nstr(value,70),rounded=float(value)))
log_tails=[]
for log_t in [707.5,707.9,708,708.5,709,709.5,710,710.5,711,712,714,716,720,740,744,745,746,1000]:
    value=2*mp.atan(mp.exp(-log_t))/mp.pi
    log_tails.append(dict(log_abs_t=log_t,df=1,exact=mp.nstr(value,70),rounded=float(value)))
for log_t,df in [(354.5,2),(566.5,1.25),(472.5,1.5)]:
    value=tail_exact(mp.exp(mp.mpf(log_t)),mp.mpf(df))
    log_tails.append(dict(log_abs_t=log_t,df=df,exact=mp.nstr(value,70),rounded=float(value)))
# Exercise upper subnormal precision across df and both exact-input APIs.
for df in [1,1.25,1.5,2,3,10,30,127.99999999999999,128,128.00000000000003,200,100000,2**53]:
    for target in [-708.5,-709,-710,-712,-720,-744]:
        v=mp.mpf(df)
        lower,upper=(mp.log(37),mp.log(40)) if df>=100000 else (mp.mpf(0),mp.mpf(750))
        for _ in range(64):
            middle=(lower+upper)/2
            if mp.log(tail_exact(mp.exp(middle),v))>target: lower=middle
            else: upper=middle
        log_t=float((lower+upper)/2)
        value=tail_exact(mp.exp(mp.mpf(log_t)),v)
        log_tails.append(dict(log_abs_t=log_t,df=df,exact=mp.nstr(value,70),rounded=float(value)))
        x=float(mp.exp((lower+upper)/2))
        if x<float('inf'):
            value=tail(x,df)
            cases.append(dict(t=x,df=df,exact=mp.nstr(value,70),rounded=float(value)))
output=dict(provenance=dict(library='mpmath',version=mp.__version__,decimal_digits=75,command='python3 scripts/reference_inference.py',method='Analytical df=1/2 tails; regularized incomplete beta for other tails; 240-step 75-digit root bisection for critical values. Exact binary64 inputs.'),tails=cases,critical=critical,log_tails=log_tails)
Path(__file__).resolve().parents[1].joinpath('tests/fixtures/inference-reference.json').write_text(json.dumps(output,indent=2)+'\n')
