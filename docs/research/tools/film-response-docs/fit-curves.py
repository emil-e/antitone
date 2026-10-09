"""Fit public PDF traces with a monotonic toe/straight-line family.

Optional rational shoulder has one extra nonnegative parameter. Curve points are
interpolated drawing traces, not independent measurements; no significance tests
or statistical film-population claims are made. Python standard library only.
"""
from pathlib import Path
import csv, json, math
ROOT=Path('.cache/probes/film-response-docs')

def softplus(x):
    return max(x,0)+math.log1p(math.exp(-abs(x)))

def nelder_mead(fn,start,steps,maxiter=1200):
    n=len(start)
    simplex=[list(start)]+[[v+(steps[j] if i==j else 0) for j,v in enumerate(start)] for i in range(n)]
    vals=[fn(p)[0] for p in simplex]
    for iteration in range(maxiter):
        ordered=sorted(zip(vals,simplex),key=lambda p:p[0]);vals=[p[0] for p in ordered];simplex=[p[1] for p in ordered]
        if math.isfinite(vals[0]) and max(abs(a-b) for p in simplex[1:] for a,b in zip(p,simplex[0]))<2e-7:break
        centroid=[sum(p[j] for p in simplex[:-1])/n for j in range(n)]
        reflect=[2*c-w for c,w in zip(centroid,simplex[-1])];fr=fn(reflect)[0]
        if fr<vals[0]:
            expand=[3*c-2*w for c,w in zip(centroid,simplex[-1])];fe=fn(expand)[0]
            simplex[-1],vals[-1]=(expand,fe) if fe<fr else (reflect,fr)
        elif fr<vals[-2]:simplex[-1],vals[-1]=reflect,fr
        else:
            outside=fr<vals[-1]
            contract=[c+0.5*(r-c) if outside else c+0.5*(w-c) for c,r,w in zip(centroid,reflect,simplex[-1])]
            fc=fn(contract)[0]
            if fc<(fr if outside else vals[-1]):simplex[-1],vals[-1]=contract,fc
            else:
                simplex=[simplex[0]]+[[b+0.5*(v-b) for b,v in zip(simplex[0],p)] for p in simplex[1:]]
                vals=[fn(p)[0] for p in simplex]
    best=min(zip(vals,simplex),key=lambda p:p[0])[1]
    return best,iteration

def projection(points,p,shoulder=False,fixed_width=None):
    t=p[0];w=fixed_width if fixed_width is not None else math.exp(p[1]) if -8<p[1]<2 else float('inf')
    k=p[-1] if shoulder else 0.
    if not (0.02<=w<=1. and points[0][0]-1<=t<=points[-1][0] and 0<=k<=1.):return float('inf'),None
    f=[]
    for x,_ in points:
        u=w*softplus((x-t)/w)
        f.append(u/(1+k*u))
    n=len(points);mf=sum(f)/n;my=sum(y for _,y in points)/n
    denom=sum((v-mf)**2 for v in f)
    if denom<1e-15:return float('inf'),None
    gamma=sum((v-mf)*(y-my) for v,(_,y) in zip(f,points))/denom
    base=my-gamma*mf
    # A fitted asymptotic base must not sit above the entire published left end.
    # This is constrained least squares, not clipping observations or a fit fallback.
    base_bound=min(y for _,y in points)
    unconstrained_base=base
    if base<0 or base>base_bound:
        base=max(0,min(base,base_bound))
        gamma=sum(v*(y-base) for v,(_,y) in zip(f,points))/sum(v*v for v in f)
    if not (0<=base<=1.5 and 0.1<=gamma<=2.):return float('inf'),None
    errors=[base+gamma*v-y for v,(_,y) in zip(f,points)]
    return sum(e*e for e in errors)/n,dict(base=base,gamma=gamma,toe_center=t,toe_width=w,shoulder=k,rmse=math.sqrt(sum(e*e for e in errors)/n),max_abs=max(map(abs,errors)),unconstrained_base=unconstrained_base,base_at_constraint=abs(base-base_bound)<1e-12)

def fit(points,shoulder=False,fixed_width=None):
    width_seeds=[0.08,0.16,0.25,0.4] if fixed_width is None else [fixed_width]
    best=(float('inf'),None,None)
    for t in [points[0][0]+0.6,points[0][0]+1.2]:
        for w in width_seeds:
            for k in ([0.,0.04,0.12] if shoulder else [0.]):
                p=[t] if fixed_width is not None else [t,math.log(w)]
                steps=[0.15] if fixed_width is not None else [0.15,0.15]
                if shoulder:p+=[k];steps+=[0.02]
                fn=lambda v:projection(points,v,shoulder,fixed_width)
                solution,iters=nelder_mead(fn,p,steps)
                loss,params=fn(solution)
                if loss<best[0]:best=loss,params,iters
    assert best[1] is not None
    return {**best[1],'iterations':best[2]}

def value(x,p):
    u=p['toe_width']*softplus((x-p['toe_center'])/p['toe_width'])
    return p['base']+p['gamma']*u/(1+p['shoulder']*u)

def slope(x,p):
    z=(x-p['toe_center'])/p['toe_width']
    sig=math.exp(-softplus(-z))
    u=p['toe_width']*softplus(z)
    return p['gamma']*sig/(1+p['shoulder']*u)**2

def inverse(d,p):
    gap=d-p['base'];den=p['gamma']-p['shoulder']*gap
    if gap<=0 or den<=0:raise ValueError('outside inverse model domain')
    u=gap/den;z=u/p['toe_width']
    return p['toe_center']+p['toe_width']*(z+math.log1p(-math.exp(-z)) if z>0.7 else math.log(math.expm1(z)))

results=[];predictions=[]
for stock in ['portra400','ektar100']:
    with (ROOT/f'{stock}-uniform.csv').open() as f:rows=list(csv.DictReader(f))
    for channel in 'BGR':
        pts=[(float(r['log10_exposure']),float(r['density'])) for r in rows if r['channel']==channel]
        current=fit(pts,fixed_width=1/math.log(10))
        toe=fit(pts)
        shoulder=fit(pts,shoulder=True)
        # Hold out the high-exposure end as a diagnostic, not a statistical CV score.
        core=fit(pts[:121])
        high_residuals=[y-value(x,core) for x,y in pts[121:]]
        top_slope=slope(pts[-1][0],shoulder)
        peak_slope=max(slope(pts[0][0]+(pts[-1][0]-pts[0][0])*i/1000,shoulder) for i in range(1001))
        # Direct trace slopes are independent of the chosen response model.
        def line_slope(items):
            mx=sum(x for x,_ in items)/len(items);my=sum(y for _,y in items)/len(items)
            return sum((x-mx)*(y-my) for x,y in items)/sum((x-mx)**2 for x,_ in items)
        middle_slope=line_slope(pts[72:113]);upper_slope=line_slope(pts[128:])
        max_roundtrip=0.
        for p in [current,toe,shoulder]:
            last=-float('inf')
            for i in range(1001):
                x=pts[0][0]+(pts[-1][0]-pts[0][0])*i/1000
                d=value(x,p)
                assert d>last;last=d
                if d-p['base']>1e-9:max_roundtrip=max(max_roundtrip,abs(inverse(d,p)-x))
        assert max_roundtrip<1e-7
        result=dict(stock=stock,channel=channel,current_width=current,toe_only=toe,toe_shoulder=shoulder,core_toe_fit=core,high_end_mean_density_residual=sum(high_residuals)/len(high_residuals),high_end_max_abs_density_residual=max(map(abs,high_residuals)),top_to_peak_slope_ratio=top_slope/peak_slope,trace_middle_slope=middle_slope,trace_upper_slope=upper_slope,trace_upper_middle_slope_ratio=upper_slope/middle_slope,max_forward_inverse_log_exposure_error=max_roundtrip)
        results.append(result)
        for x,y in pts:predictions.append((stock,channel,x,y,value(x,current),value(x,toe),value(x,shoulder),value(x,core)))
        print(f"{stock} {channel}: current-width RMSE={current['rmse']:.6f}; fitted-toe RMSE={toe['rmse']:.6f} width={toe['toe_width']:.6f} gamma={toe['gamma']:.6f}; shoulder RMSE={shoulder['rmse']:.6f} k={shoulder['shoulder']:.6f} width={shoulder['toe_width']:.6f} gamma={shoulder['gamma']:.6f}; top/peak slope={top_slope/peak_slope:.4f}; trace upper/middle slope={upper_slope/middle_slope:.4f}; high-end core residual={sum(high_residuals)/len(high_residuals):+.6f}")
(ROOT/'fit-results.json').write_text(json.dumps(results,indent=2)+'\n')
# A deliberately rounded, shared starting shape, not a film-population median.
shared_width=1/(2.5*math.log(10))
shared=[];shared_map={}
for r in results:
    stock,channel=r['stock'],r['channel']
    with (ROOT/f'{stock}-uniform.csv').open() as f:rows=list(csv.DictReader(f))
    pts=[(float(v['log10_exposure']),float(v['density'])) for v in rows if v['channel']==channel]
    p=fit(pts,shoulder=True,fixed_width=shared_width)
    shared.append(dict(stock=stock,channel=channel,**p));shared_map[(stock,channel)]=p
    print(f"shared_beta2.5 {stock} {channel}: RMSE={p['rmse']:.6f} maximum={p['max_abs']:.6f} k={p['shoulder']:.6f}")
(ROOT/'shared-toe-results.json').write_text(json.dumps(shared,indent=2)+'\n')
with (ROOT/'fit-predictions.csv').open('w',newline='') as f:
    w=csv.writer(f);w.writerow(['stock','channel','log10_exposure','density','current_width','toe_only','toe_shoulder','core_extrapolation','shared_beta2_5'])
    w.writerows([row+(value(row[2],shared_map[row[:2]]),) for row in predictions])
print('All individually fitted curves strictly monotonic; forward/inverse checks <1e-7 log10-exposure in sampled source intervals.')
