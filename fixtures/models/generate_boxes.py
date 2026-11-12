"""Generate the CON-04 box models for fixtures/content (Python stdlib only).

One untextured, flat-shaded GLB box per unit of `fake::db()` except the
cruiser (fx_cruiser.glb is CON-09's textured cube). Sizes are half-extents
in model units; the nodes `hp01`, `hp02` and `root` exist so hardpoint names
in fixtures/content resolve. Original, public domain; deterministic output.

    python3 fixtures/models/generate_boxes.py
"""
import json
import struct
from pathlib import Path

BOXES = {
    "fx_frigate": (0.5, 0.25, 1.5),
    "fx_station": (2.0, 1.0, 2.0),
    "fx_yard": (2.5, 0.75, 2.5),
    "fx_miner": (0.5, 0.4, 1.0),
    "fx_moon": (3.0, 3.0, 3.0),
}

# (normal, four CCW corners as sign triples) per face.
FACES = [
    ((0, 0, 1), [(-1, -1, 1), (1, -1, 1), (1, 1, 1), (-1, 1, 1)]),
    ((0, 0, -1), [(1, -1, -1), (-1, -1, -1), (-1, 1, -1), (1, 1, -1)]),
    ((1, 0, 0), [(1, -1, 1), (1, -1, -1), (1, 1, -1), (1, 1, 1)]),
    ((-1, 0, 0), [(-1, -1, -1), (-1, -1, 1), (-1, 1, 1), (-1, 1, -1)]),
    ((0, 1, 0), [(-1, 1, 1), (1, 1, 1), (1, 1, -1), (-1, 1, -1)]),
    ((0, -1, 0), [(-1, -1, -1), (1, -1, -1), (1, -1, 1), (-1, -1, 1)]),
]


def box_glb(name, half):
    positions, normals, indices = [], [], []
    for normal, corners in FACES:
        base = len(positions)
        positions += [tuple(c * h for c, h in zip(corner, half)) for corner in corners]
        normals += [normal] * 4
        indices += [base + i for i in (0, 1, 2, 0, 2, 3)]
    binary = bytearray()
    views, accessors = [], []

    def add(data, target, count, ctype, kind, extra=None):
        while len(binary) % 4:
            binary.append(0)
        views.append({"buffer": 0, "byteOffset": len(binary), "byteLength": len(data), "target": target})
        binary.extend(data)
        acc = {"bufferView": len(views) - 1, "componentType": ctype, "count": count, "type": kind}
        acc.update(extra or {})
        accessors.append(acc)
        return len(accessors) - 1

    flat = lambda rows: [x for row in rows for x in row]
    pos = add(struct.pack("<%df" % (3 * len(positions)), *flat(positions)), 34962, len(positions), 5126, "VEC3",
              {"min": [-h for h in half], "max": list(half)})
    nrm = add(struct.pack("<%df" % (3 * len(normals)), *flat(normals)), 34962, len(normals), 5126, "VEC3")
    idx = add(struct.pack("<%dH" % len(indices), *indices), 34963, len(indices), 5123, "SCALAR")
    document = {
        "asset": {"version": "2.0", "generator": "STA64 original synthetic fixture"},
        "scene": 0,
        "scenes": [{"nodes": [0]}],
        "nodes": [
            {"name": name, "mesh": 0, "children": [1, 2, 3]},
            {"name": "root"},
            {"name": "hp01", "translation": [0.0, half[1], 0.0]},
            {"name": "hp02", "translation": [0.0, half[1], half[2] / 2]},
        ],
        "meshes": [{"primitives": [{"attributes": {"POSITION": pos, "NORMAL": nrm}, "indices": idx, "material": 0}]}],
        "materials": [{"name": "grey", "pbrMetallicRoughness": {"baseColorFactor": [0.6, 0.62, 0.66, 1.0],
                                                                "metallicFactor": 0.1, "roughnessFactor": 0.8}}],
        "buffers": [{"byteLength": len(binary)}],
        "bufferViews": views,
        "accessors": accessors,
    }
    js = json.dumps(document, separators=(",", ":")).encode()
    js += b" " * (-len(js) % 4)
    bin_chunk = bytes(binary) + b"\0" * (-len(binary) % 4)
    total = 12 + 8 + len(js) + 8 + len(bin_chunk)
    return (struct.pack("<4sII", b"glTF", 2, total) + struct.pack("<I4s", len(js), b"JSON") + js
            + struct.pack("<I4s", len(bin_chunk), b"BIN\0") + bin_chunk)


if __name__ == "__main__":
    out = Path(__file__).resolve().parent
    for name, half in BOXES.items():
        (out / f"{name}.glb").write_bytes(box_glb(name, half))
        print("wrote", out / f"{name}.glb")
