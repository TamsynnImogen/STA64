"""Generate original, public-domain CON-09 GLB fixture using only Python stdlib."""
import json
import struct
import zlib
from pathlib import Path


def png(width, height, pixels):
    def chunk(kind, data):
        return struct.pack(">I", len(data)) + kind + data + struct.pack(">I", zlib.crc32(kind + data))
    rows = b"".join(b"\0" + pixels[y * width * 4:(y + 1) * width * 4] for y in range(height))
    return (b"\x89PNG\r\n\x1a\n" + chunk(b"IHDR", struct.pack(">IIBBBBB", width, height, 8, 6, 0, 0, 0))
            + chunk(b"IDAT", zlib.compress(rows)) + chunk(b"IEND", b""))


# CCW quads viewed from outside; tangent follows increasing U on each face.
faces = [
    ((0, 0, 1), (1, 0, 0), [(-1, -1, 1), (1, -1, 1), (1, 1, 1), (-1, 1, 1)]),
    ((0, 0, -1), (-1, 0, 0), [(1, -1, -1), (-1, -1, -1), (-1, 1, -1), (1, 1, -1)]),
    ((1, 0, 0), (0, 0, -1), [(1, -1, 1), (1, -1, -1), (1, 1, -1), (1, 1, 1)]),
    ((-1, 0, 0), (0, 0, 1), [(-1, -1, -1), (-1, -1, 1), (-1, 1, 1), (-1, 1, -1)]),
    ((0, 1, 0), (1, 0, 0), [(-1, 1, 1), (1, 1, 1), (1, 1, -1), (-1, 1, -1)]),
    ((0, -1, 0), (1, 0, 0), [(-1, -1, -1), (1, -1, -1), (1, -1, 1), (-1, -1, 1)]),
]
positions, normals, tangents, uv0, uv1, colors, indices = [], [], [], [], [], [], []
for normal, tangent, corners in faces:
    start = len(positions)
    positions.extend(corners)
    normals.extend([normal] * 4)
    tangents.extend([(*tangent, 1)] * 4)
    uv0.extend([(0, 1), (1, 1), (1, 0), (0, 0)])
    uv1.extend([(0, 1), (1, 1), (1, 0), (0, 0)])
    colors.extend([(1, 1, 1, 1)] * 4)
    indices.extend(start + i for i in (0, 1, 2, 0, 2, 3))

binary = bytearray()
views, accessors = [], []


def add_view(data):
    while len(binary) % 4:
        binary.append(0)
    offset = len(binary)
    binary.extend(data)
    views.append({"buffer": 0, "byteOffset": offset, "byteLength": len(data)})
    return len(views) - 1


def attribute(values, components, kind, component_type=5126):
    flat = [x for row in values for x in row]
    fmt = "f" if component_type == 5126 else "H"
    view = add_view(struct.pack("<" + fmt * len(flat), *flat))
    accessor = {"bufferView": view, "componentType": component_type, "count": len(values), "type": kind}
    if kind == "VEC3" and values is positions:
        accessor["min"] = [-1, -1, -1]
        accessor["max"] = [1, 1, 1]
    accessors.append(accessor)
    return len(accessors) - 1


attrs = {
    "POSITION": attribute(positions, 3, "VEC3"),
    "NORMAL": attribute(normals, 3, "VEC3"),
    "TANGENT": attribute(tangents, 4, "VEC4"),
    "TEXCOORD_0": attribute(uv0, 2, "VEC2"),
    "TEXCOORD_1": attribute(uv1, 2, "VEC2"),
    "COLOR_0": attribute(colors, 4, "VEC4"),
}
index_accessor = attribute([(i,) for i in indices], 1, "SCALAR", 5123)
checker = bytearray()
for y in range(16):
    for x in range(16):
        checker.extend((225, 35, 25, 255) if (x // 4 + y // 4) % 2 == 0 else (25, 80, 230, 255))
color_view = add_view(png(16, 16, checker))
normal_view = add_view(png(1, 1, bytes((128, 128, 255, 255))))
document = {
    "asset": {"version": "2.0", "generator": "STA64 original synthetic fixture"},
    "scene": 0, "scenes": [{"nodes": [0]}],
    "nodes": [{"name": "fixture_cube", "mesh": 0}],
    "meshes": [{"primitives": [{"attributes": attrs, "indices": index_accessor, "material": 0, "mode": 4}]}],
    "materials": [{"name": "checker_mask", "pbrMetallicRoughness": {
        "baseColorFactor": [1, 1, 1, 0.9], "metallicFactor": 0.2, "roughnessFactor": 0.7,
        "baseColorTexture": {"index": 0, "texCoord": 1}},
        "normalTexture": {"index": 1, "scale": 0.5}, "alphaMode": "MASK", "alphaCutoff": 0.5,
        "doubleSided": True}],
    "textures": [{"source": 0, "sampler": 0}, {"source": 1, "sampler": 0}],
    "samplers": [{"magFilter": 9728, "minFilter": 9728, "wrapS": 33071, "wrapT": 33071}],
    "images": [{"bufferView": color_view, "mimeType": "image/png"},
               {"bufferView": normal_view, "mimeType": "image/png"}],
    "accessors": accessors, "bufferViews": views, "buffers": [{"byteLength": len(binary)}],
}
payload = json.dumps(document, separators=(",", ":")).encode()
payload += b" " * ((-len(payload)) % 4)
binary += b"\0" * ((-len(binary)) % 4)
total = 12 + 8 + len(payload) + 8 + len(binary)
glb = (b"glTF" + struct.pack("<II", 2, total) + struct.pack("<I4s", len(payload), b"JSON") + payload
       + struct.pack("<I4s", len(binary), b"BIN\0") + binary)
Path(__file__).with_name("fx_cruiser.glb").write_bytes(glb)
