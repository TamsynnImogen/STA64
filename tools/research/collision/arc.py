"""Standalone FPQ (v1/v2) and ZFS extractor, ported from STA64 crates/sta_fpq and sta_vfs/zfs.
usage: arc.py <archive> <outdir> [ext,ext...]"""
import os, struct, sys

def lzo(src, expected, require_eof):
    out = bytearray(); pos = 0; n_src = len(src)
    def byte():
        nonlocal pos
        b = src[pos]; pos += 1; return b
    def ext(base):
        n = base
        while True:
            b = byte()
            if b == 0: n += 255
            else: return n + b
    def lit(n):
        nonlocal pos
        out.extend(src[pos:pos+n]);
        if pos + n > n_src: raise ValueError("overrun")
        pos += n
    def cm(dist, ln):
        if dist == 0 or dist > len(out): raise ValueError("lookbehind")
        s = len(out) - dist
        for i in range(ln): out.append(out[s+i])
    first = src[0]
    if first > 17:
        pos = 1; n = first - 17; lit(n); state = n if n < 4 else 4
    else:
        state = 0
    while True:
        if not require_eof and pos == n_src:
            assert len(out) == expected; return bytes(out)
        t = byte()
        if t < 16:
            if state == 0:
                n = (ext(15) if t == 0 else t) + 3; lit(n); state = 4; continue
            elif state != 4:
                nx = t & 3; dist = 1 + (t >> 2) + (byte() << 2); ln = 2
            else:
                nx = t & 3; dist = 1 + 0x800 + (t >> 2) + (byte() << 2); ln = 3
        elif t >= 64:
            nx = t & 3; dist = 1 + ((t >> 2) & 7) + (byte() << 3); ln = (t >> 5) + 1
        elif t >= 32:
            l = t & 31; ln = (ext(31) if l == 0 else l) + 2
            d = byte() | (byte() << 8); dist = 1 + (d >> 2); nx = d & 3
        else:
            l = t & 7; ln = (ext(7) if l == 0 else l) + 2
            d = byte() | (byte() << 8); far = ((t & 8) << 11) + (d >> 2)
            if far == 0:
                assert len(out) == expected, (len(out), expected); return bytes(out)
            dist = far + 0x4000; nx = d & 3
        cm(dist, ln); lit(nx); state = nx

def v1prefix(b):
    v = 0; sh = 0; at = 0
    while True:
        w = b[at] | (b[at+1] << 8); at += 2
        v |= (w & 0x7fff) << sh; sh += 15
        if not w & 0x8000: return v, at

def fpq(data):
    u = lambda o: struct.unpack_from("<I", data, o)[0]
    ver, nfo, nfi, nh, dlen, nlen = [u(4 + 4*i) for i in range(6)]
    fo_at = 28 + nh*12; fi_at = fo_at + nfo*12; names = fi_at + nfi*25; dat = names + nlen
    name = lambda o, l: data[names+o:names+o+l].decode('latin1')
    folders = [(struct.unpack_from("<i", data, fo_at+i*12)[0], u(fo_at+i*12+4), u(fo_at+i*12+8)) for i in range(nfo)]
    for i in range(nfi):
        o = fi_at + i*25
        doff, size, stored, no, nl = [u(o+4*k) for k in range(5)]
        comp = data[o+20]; folder = struct.unpack_from("<i", data, o+21)[0]
        parts = [name(no, nl)]
        while folder != -1:
            p, a, b = folders[folder]; parts.append(name(a, b)); folder = p
        path = "/".join(reversed(parts))
        def read(doff=doff, size=size, stored=stored, comp=comp):
            s = data[dat+doff:dat+doff+stored]
            if comp == 0: return s
            if ver == 1:
                sz, used = v1prefix(s); return lzo(s[used:], size, False)
            return lzo(s, size, True)
        yield path, read

def zfs(data):
    u = lambda o: struct.unpack_from("<I", data, o)[0]
    maxn, per, count, key, block = u(8), u(12), u(16), data[20:24], u(24)
    slot = maxn + 20; n = 0
    while block and n < count:
        nxt = u(block)
        for i in range(per):
            at = block + 4 + i*slot
            raw = data[at:at+maxn]; end = raw.find(b"\0")
            if end == 0: continue
            nm = raw[:end if end >= 0 else maxn].decode('latin1')
            off, size, flags = u(at+maxn), u(at+maxn+8), u(at+maxn+16)
            def read(off=off, size=size, flags=flags, nm=nm):
                assert flags == 0, (nm, flags)
                r = data[off:off+size]
                if key == b"\0\0\0\0": return r
                w = len(r) & ~3
                return bytes(c ^ key[i & 3] for i, c in enumerate(r[:w])) + r[w:]
            n += 1
            yield nm, read
        block = nxt

def main():
    arc, out = sys.argv[1], sys.argv[2]
    exts = tuple("." + e.lower() for e in sys.argv[3].split(",")) if len(sys.argv) > 3 else None
    data = open(arc, "rb").read()
    it = fpq(data) if data[:4] == b"FPQ\0" else zfs(data)
    n = 0
    for path, read in it:
        if exts and not path.lower().endswith(exts): continue
        dst = os.path.join(out, path.replace("\\", "/").lstrip("/"))
        os.makedirs(os.path.dirname(dst) or out, exist_ok=True)
        open(dst, "wb").write(read()); n += 1
    print(arc, n)

main()
