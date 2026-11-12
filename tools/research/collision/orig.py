import sys, os, json, math
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import sodbound
cache = {}
def orig(path):
    """A2 InitShieldMatrices approximation: whole-hierarchy mesh-vertex AABB; ellipsoid semi-axes half-extent*sqrt3
    centred on box centre, rescaled to enclose all mesh vertices; returns max semi-axis (scale 1, pad 0)."""
    if path in cache: return cache[path]
    n = sodbound.parse(open(path, 'rb').read()); pts = []
    for i, x in enumerate(n):
        if x['t'] == 1 and x['pos']:
            m = sodbound.model_m(n, i); pts += [sodbound.tp(m, p) for p in x['pos']]
    if not pts: cache[path] = None; return None
    lo = [min(p[k] for p in pts) for k in range(3)]; hi = [max(p[k] for p in pts) for k in range(3)]
    c = [(lo[k]+hi[k])/2 for k in range(3)]; a = [max((hi[k]-lo[k])/2*math.sqrt(3), 1e-6) for k in range(3)]
    s = math.sqrt(max(sum(((p[k]-c[k])/a[k])**2 for k in range(3)) for p in pts))
    cache[path] = (max(a)*s, math.hypot(c[0], c[2]))
    return cache[path]
d = json.load(open('radii.json'))
def q(v):
    v = sorted(v); n = len(v); return f"n={n} min={v[0]:.2f} p10={v[n//10]:.2f} med={v[n//2]:.2f} p90={v[9*n//10]:.2f} max={v[-1]:.2f}"
for s in ['A1', 'A2 1.1 (GOG)', 'FO 3.x', 'Roots base', 'DW']:
    rat = []; off = []; neg = 0; ex = []
    for x in d[s]:
        if not x['sod'] or not isinstance(x['circle'], float) or x['circle'] <= 0: continue
        o = orig(x['sod'])
        if not o: continue
        R = o[0]*x['scale'] + (x['pad'] or 0)
        if R <= 0: neg += 1; continue
        rat.append(x['sta64_radius']/R); off.append(o[1]*x['scale'])
        ex.append((x['odf'], round(x['sta64_radius'],1), round(R,1), round(o[1]*x['scale'],1)))
    print(s, 'sta64/original', q(rat), '| box-centre offset from origin (XZ)', q(off), '| orig<=0:', neg)
    ex.sort(key=lambda e: e[1]/e[2]); print('   extremes', ex[:3], ex[-3:])
