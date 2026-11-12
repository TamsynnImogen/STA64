import sys, os, json, collections, statistics
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import sodbound
G = os.environ.get('STA_GAMES', '.'); R = G + '/Star Trek Armada II Fleet Operations Roots/Data'
SODS = {
 'A1': [G + '/Star Trek Armada/SOD'],
 'A2 1.1 (GOG)': [G + '/Star Trek Armada II/SOD'],
 'A2 (Stock)': [G + '/Star Trek Armada II (Stock)/SOD'],
 'FO 3.x': [G + '/Star Trek Armada II Fleet Operations/Data/sod'],
 'Roots base': [R + '/sod'],
 'Roots/Fleet Ops 4.0': [R + '/Mods/Fleet Ops 4.0/sod', R + '/sod'],
 'Roots/STA2 Classic': [R + '/Mods/STA2 Classic Mod/SOD', R + '/sod'],
 'Roots/STA2 Classic Enhanced': [R + '/Mods/STA2 Classic Mod/SOD', R + '/sod'],
 'DW': [G + '/Star Trek Armada II Dominion War/Data/sod'],
}
def index(dirs):
    m = {}
    for d in reversed(dirs):
        for root, _, fs in os.walk(d):
            for f in fs:
                if f.lower().endswith('.sod'): m[f[:-4].lower()] = os.path.join(root, f)
    return m
cache = {}
def bound(path):
    if path not in cache:
        try:
            n = sodbound.parse(open(path, 'rb').read())
            cache[path] = (sodbound.circle(n), sodbound.tight_xz(n))
        except Exception as e:
            cache[path] = ('ERR ' + type(e).__name__, None)
    return cache[path]
def num(v):
    try: return float(str(v).split()[0].rstrip('fF;'))
    except Exception: return None
units = json.load(open('units.json'))
out = {}
for s, rows in units.items():
    idx = index(SODS[s]); res = []
    for x in rows:
        if x['kind'] not in ('ship', 'station'): continue
        p = idx.get(x['model'].lower().removesuffix('.sod'))
        c, t = bound(p) if p else ('MISSING', None)
        sc = num(x['scalesod']) or 1.0
        x2 = dict(x); x2.update(sod=p, circle=c, tight=t, scale=sc,
            sta64_radius=(c*sc if isinstance(c, float) else None), pad=num(x['shieldpad']))
        res.append(x2)
    out[s] = res
    miss = sum(1 for r in res if r['circle'] == 'MISSING'); err = sum(1 for r in res if str(r['circle']).startswith('ERR'))
    ok = [r for r in res if isinstance(r['circle'], float)]
    print(f"{s:30} units {len(res)} model-missing {miss} parse-err {err} ok {len(ok)}")
json.dump(out, open('radii.json', 'w'), indent=0, default=str)
