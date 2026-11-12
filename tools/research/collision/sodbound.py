import struct, math
class R:
    def __init__(s, d, p): s.d, s.p = d, p
    def u8(s): v = s.d[s.p]; s.p += 1; return v
    def u16(s): v = struct.unpack_from('<H', s.d, s.p)[0]; s.p += 2; return v
    def u32(s): v = struct.unpack_from('<I', s.d, s.p)[0]; s.p += 4; return v
    def f(s, n=1): v = struct.unpack_from('<%df' % n, s.d, s.p); s.p += 4*n; return v
    def st(s): n = s.u16(); b = s.d[s.p:s.p+n]; s.p += n; return b.split(b'\0')[0].decode('latin1')
def f32(x): return struct.unpack('<f', struct.pack('<f', x))[0]
def parse(data):
    assert data[:10] in (b'Storm3D_SW', b'StarTrekDB')
    r = R(data, 10); v = r.f()[0]
    V = lambda x: f32(x)
    if v < V(1.7):
        for _ in range(r.u16()): r.st(); r.st(); r.u32(); r.u8(); r.u8(); r.u8()
    for _ in range(r.u16()):
        r.st(); r.f(10); r.u8()
        if v >= V(1.9): r.u8()
    nodes = []
    for _ in range(r.u16()):
        t = r.u16(); name = r.st(); parent = r.st(); m = r.f(12); pos = None
        if t == 1:
            r.st()
            if v >= V(1.93):
                r.u32()
                for _ in range(r.u32()): r.st(); r.u32()
            elif v >= V(1.7): r.st()
            if v >= V(1.92): r.st()
            if v >= V(1.91): r.u16()
            nv = r.u16(); pos = []
            if nv:
                nuv = r.u16()
                if nuv:
                    ng = r.u16()
                    pos = [r.f(3) for _ in range(nv)]; r.p += 8*nuv
                    for _ in range(ng):
                        nf = r.u16(); r.st(); r.p += 12*nf
                    r.u8(); n = r.u16(); r.p += 2*n
        elif t == 2: r.u8(); r.f(5)
        elif t == 12: r.st()
        nodes.append(dict(t=t, name=name, parent=parent, m=m, pos=pos))
    low = {}
    for i, n in enumerate(nodes): low.setdefault(n['name'].lower(), i)
    for i, n in enumerate(nodes):
        p = low.get(n['parent'].lower()) if n['parent'] else None
        n['pi'] = p if p != i else None
    return nodes
def tp(m, p):
    return (p[0]*m[0]+p[1]*m[3]+p[2]*m[6]+m[9], p[0]*m[1]+p[1]*m[4]+p[2]*m[7]+m[10], p[0]*m[2]+p[1]*m[5]+p[2]*m[8]+m[11])
def then(c, p):
    out = []
    for r_ in range(3):
        a = c[r_*3:r_*3+3]
        out += [a[0]*p[0]+a[1]*p[3]+a[2]*p[6], a[0]*p[1]+a[1]*p[4]+a[2]*p[7], a[0]*p[2]+a[1]*p[5]+a[2]*p[8]]
    return out + list(tp(p, c[9:12]))
def model_m(nodes, i):
    m = list(nodes[i]['m']); at = nodes[i]['pi']; g = 0
    while at is not None and g <= len(nodes):
        m = then(m, nodes[at]['m']); at = nodes[at]['pi']; g += 1
    return m
def bounds(pos):
    lo = [min(p[k] for p in pos) for k in range(3)]; hi = [max(p[k] for p in pos) for k in range(3)]
    c = [(lo[k]+hi[k])/2 for k in range(3)]
    return c, math.sqrt(max(sum((p[k]-c[k])**2 for k in range(3)) for p in pos))
def circle(nodes, filt=None):
    """STA64 launch.rs bounding_circle: max over mesh nodes of |XZ(centre)| + 3D sphere r."""
    best = 0.0
    for i, n in enumerate(nodes):
        if n['t'] != 1 or not n['pos']: continue
        if filt and not filt(nodes, i): continue
        c, r = bounds(n['pos']); p = tp(model_m(nodes, i), c)
        best = max(best, math.hypot(p[0], p[2]) + r)
    return best
def tight_xz(nodes, filt=None):
    """Exact max XZ distance of any transformed vertex from origin."""
    best = 0.0
    for i, n in enumerate(nodes):
        if n['t'] != 1 or not n['pos']: continue
        if filt and not filt(nodes, i): continue
        m = model_m(nodes, i)
        for p in n['pos']:
            q = tp(m, p); best = max(best, math.hypot(q[0], q[2]))
    return best
