import sys, os, re, json, collections
sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
from odf import OdfSet, unq
G = os.environ.get('STA_GAMES', '.')      # folder holding the game installs
X = os.environ.get('STA_EXTRACT', 'extract')  # archive contents extracted with arc.py
R = G + '/Star Trek Armada II Fleet Operations Roots/Data'
SETS = {
 'A1': [('a1', X + '/a1_assets'), ('a1addon', G + '/Star Trek Armada/addon')],
 'A2 1.1 (GOG)': [('a2', G + '/Star Trek Armada II/odf')],
 'A2 (Stock)': [('a2s', G + '/Star Trek Armada II (Stock)/odf')],
 'FO 3.x': [('fo_addon', G + '/Star Trek Armada II Fleet Operations/Data/addon'), ('fo', X + '/fo')],
 'Roots base': [('roots_addon', R + '/addon'), ('roots', X + '/roots'), ('roots_sys', X + '/roots_system')],
 'Roots/Fleet Ops 4.0': [('fo40_addon', R + '/Mods/Fleet Ops 4.0/addon'), ('fo40', X + '/fo40'), ('fo40_sys', X + '/fo40_system')],
 'Roots/STA2 Classic': [('cl_addon', R + '/Mods/STA2 Classic Mod/addon'), ('classic', X + '/classic')],
 'Roots/STA2 Classic Enhanced': [('enh', R + '/Mods/STA2 Classic Mod Enhanced Gameplay/odf'), ('cl_addon', R + '/Mods/STA2 Classic Mod/addon'), ('classic', X + '/classic')],
 'DW': [('dw_addon', G + '/Star Trek Armada II Dominion War/Data/addon'), ('dw', X + '/dw')],
}
def load(name):
    return OdfSet([(n, d) for n, d in SETS[name] if os.path.isdir(d)])
if __name__ == '__main__':
    pat = re.compile(r'radius|collis|avoid|scale|bound|size|extent|physic|repul|sphere|width|length|height|clear|spacing|footprint|box', re.I)
    keys = collections.defaultdict(lambda: collections.Counter())
    for s in SETS:
        o = load(s)
        for b in o.files:
            try: incs, kv = o.own(b)
            except Exception as e: continue
            for k, (orig, v) in kv:
                if pat.search(k): keys[k][s] += 1
    for k in sorted(keys):
        print(k, dict(keys[k]))
