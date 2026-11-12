"""ODF loading with #include resolution over layered directories.
Layer list: highest priority first. Files indexed by lower-case basename (A2/FO look up
ODFs by name, not path)."""
import os, re

INC = re.compile(r'^\s*#include\s+"([^"]+)"', re.I)
KV = re.compile(r'^\s*([A-Za-z_][\w\.\[\]]*)\s*=\s*(.*)$')

def strip_comment(s):
    out = []; q = False; i = 0
    while i < len(s):
        c = s[i]
        if c == '"': q = not q
        if not q and s.startswith('//', i): break
        out.append(c); i += 1
    return ''.join(out).strip()

def parse(text):
    incs, kv = [], []
    for line in text.splitlines():
        m = INC.match(line)
        if m: incs.append(m.group(1)); continue
        line = strip_comment(line)
        m = KV.match(line)
        if m: kv.append((m.group(1).lower(), (m.group(1), m.group(2).strip())))
    return incs, kv

class OdfSet:
    def __init__(self, layers):
        self.files = {}   # basename lower -> (path, layer)
        for name, d in reversed(layers):  # low priority first, overwritten by higher
            for root, _, fs in os.walk(d):
                for f in fs:
                    if f.lower().endswith('.odf'):
                        self.files[f.lower()] = (os.path.join(root, f), name)
        self.cache = {}; self.raw = {}
    def own(self, b):
        if b not in self.raw:
            p = self.files[b][0]
            self.raw[b] = parse(open(p, 'rb').read().decode('latin1'))
        return self.raw[b]
    def resolve(self, b, stack=()):
        b = b.lower()
        if b in self.cache: return self.cache[b]
        if b not in self.files or b in stack: return None
        incs, kv = self.own(b)
        d = {}; src = {}; chain = [b]
        for inc in incs:
            r = self.resolve(os.path.basename(inc.replace('\\', '/')), stack + (b,))
            if r:
                d.update(r['kv']); src.update(r['src']); chain += r['chain']
        for k, v in kv:
            d[k] = v[1]; src[k] = b
        res = {'kv': d, 'src': src, 'chain': chain, 'incs': incs}
        self.cache[b] = res
        return res

def unq(v):
    v = v.strip()
    m = re.match(r'"([^"]*)"', v)
    if m: return m.group(1)
    return v.split()[0] if v.split() else ''
