import sys, os, json, collections
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from survey import SETS, load
from odf import unq
SHIP = {"craft","freighter","constructionrig","wingman","repairship","cargo_ship","evolver","scavenger","fighter"}
STATION = {"starbase","shipyard","buildyard","research","mining","p_mining","sensor","colony","trading_station"}
OTHER = {"planet","planet_obj","moon","latinum","asteroid","scrap","background_obj","u_ball_obj"}
SIZE = ["collisionradius","shieldpad"]
INFO = ["scalesod","shieldscale","avoidanceclass","avoidme","footprintbuffer"]
out = {}
for s in SETS:
    o = load(s); rows = []
    for b in sorted(o.files):
        r = o.resolve(b)
        if not r: continue
        kv = r['kv']; cl = unq(kv.get('classlabel','')).lower()
        kind = 'ship' if cl in SHIP else 'station' if cl in STATION else 'other' if cl in OTHER else None
        if not kind: continue
        model = unq(kv['basename']) if 'basename' in kv else b[:-4]
        rows.append(dict(odf=b, kind=kind, cls=cl, race=unq(kv.get('race','')).lower(), layer=o.files[b][1],
            model=model, path=o.files[b][0], unitname=unq(kv.get('unitname','')),
            **{k: kv.get(k) for k in SIZE+INFO}, src={k: r['src'].get(k) for k in SIZE+['scalesod']}))
    out[s] = rows
json.dump(out, open('units.json','w'), indent=0)
print(f"{'set':30} {'kind':8} {'total':>6} {'shieldPad':>9} {'collRad':>7} {'noSizeKey':>9} {'%':>6} {'scaleSOD':>8}")
for s, rows in out.items():
    for kind in ['ship','station','other']:
        R = [x for x in rows if x['kind']==kind]
        if not R: continue
        sp = sum(1 for x in R if x['shieldpad'] is not None); cr = sum(1 for x in R if x['collisionradius'] is not None)
        none = [x for x in R if x['shieldpad'] is None and x['collisionradius'] is None]
        sc = sum(1 for x in R if x['scalesod'] is not None)
        print(f"{s:30} {kind:8} {len(R):6} {sp:9} {cr:7} {len(none):9} {100*len(none)/len(R):6.1f} {sc:8}")
