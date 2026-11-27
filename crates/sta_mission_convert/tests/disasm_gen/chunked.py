"""Helper of the table generators: runs GNU objdump 2.47 on small chunks of authored bytes.

Authored bytes only: nothing here reads a game file. Nothing is kept between chunks.
"""
try:
    import resource
    resource.setrlimit(resource.RLIMIT_AS, (2 * 1024**3, 2 * 1024**3))
except (ImportError, ValueError, OSError):
    pass
import os
import re
import subprocess
import tempfile

LINE = re.compile(r'^\s*([0-9a-f]+):\t(.*)$')


def check_objdump():
    out = subprocess.run(['objdump', '--version'], capture_output=True, text=True).stdout
    if ' 2.47' not in out.split('\n')[0]:
        raise SystemExit('objdump 2.47 is needed')


def objdump_chunk(slots, base=0x10000000):
    """slots: 32-byte slots (a few thousand at most). Returns, per slot, (text, length) of the
    first instruction at the slot start."""
    with tempfile.TemporaryDirectory() as d:
        path = os.path.join(d, 'chunk.bin')
        with open(path, 'wb') as f:
            for s in slots:
                f.write(s)
        p = subprocess.Popen(['objdump', '-D', '-b', 'binary', '-m', 'i386', '-M', 'intel',
                              '--no-show-raw-insn', '--adjust-vma=%#x' % base, path],
                             stdout=subprocess.PIPE, text=True, encoding='latin1')
        first = {}
        prev = None
        for line in p.stdout:
            m = LINE.match(line)
            if not m:
                continue
            a, t = int(m.group(1), 16), m.group(2).strip()
            if t.startswith('...'):
                continue
            if prev is not None and prev[0] in first and first[prev[0]][1] is None:
                first[prev[0]][1] = a - prev[0]
            if (a - base) % 32 == 0:
                first[a] = [t, None]
            prev = (a, t)
        p.wait()
    out = []
    for i in range(len(slots)):
        t, n = first.get(base + 32 * i, (None, None))
        out.append((t, n if n is not None else 32))
    return out
