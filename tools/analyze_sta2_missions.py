#!/usr/bin/env python3
"""Inspect compiled Star Trek: Armada II mission DLL pairs.

Armada II's ``S.dsl`` and ``R.drl`` files are PE32 DLLs.  This tool reads their
metadata without loading or executing them and produces a clean-room research
manifest containing imports, exports, debug paths, and mission asset references.
It intentionally uses only the Python standard library.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import re
import shutil
import struct
import subprocess
import sys
from collections import defaultdict
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path
from typing import Iterable, Sequence


SCHEMA_VERSION = 3
PE_SIGNATURE = b"PE\0\0"
IMAGE_DIRECTORY_ENTRY_EXPORT = 0
IMAGE_DIRECTORY_ENTRY_IMPORT = 1
IMAGE_DIRECTORY_ENTRY_DEBUG = 6
IMAGE_DEBUG_TYPE_CODEVIEW = 2

MACHINE_NAMES = {
    0x014C: "i386",
    0x8664: "x86_64",
    0x01C0: "arm",
    0xAA64: "arm64",
}

REFERENCE_EXTENSIONS = {
    "aip",
    "avi",
    "bik",
    "bmp",
    "cfg",
    "dll",
    "drl",
    "dsl",
    "exe",
    "mp3",
    "odf",
    "pdb",
    "spr",
    "tga",
    "txt",
    "wav",
}
REFERENCE_RE = re.compile(
    rb"(?i)(?:[A-Z]:[\\/])?[A-Z0-9_~(). +\\/-]+\."
    + rb"(?:"
    + "|".join(sorted(REFERENCE_EXTENSIONS)).encode("ascii")
    + rb")"
)
ASCII_RE = re.compile(rb"[\x20-\x7e]{4,}")
WORLD_NAME_RE = re.compile(r"^WORLD_[A-Za-z0-9_]+$")
LOCALIZED_KEY_RE = re.compile(r"^MISSION_[A-Za-z0-9_]+$")
MANGLED_BASE_RE = re.compile(r"^\?([^@?]+)@@")
RULE_NAME_RE = re.compile(r"^[A-Za-z_][A-Za-z0-9_]*$")
MAP_SYMBOL_RE = re.compile(
    r"^\s*[0-9A-Fa-f]+:[0-9A-Fa-f]+\s+(\S+)\s+([0-9A-Fa-f]{8})\b"
)
SCRIPT_INTERFACE_VTABLE_SYMBOL = "??_7ScriptInterfaceImp@@6B@"
SCRIPT_INTERFACE_METHOD_RE = re.compile(r"^\?([^@]+)@ScriptInterfaceImp@@")

STATE_VTABLE_METHODS = {
    ("hot_int", 7): ("assign", "void assign(int)"),
    ("hot_int", 8): ("truthy", "int truthy(void)"),
    ("hot_float", 7): ("assign", "void assign(float)"),
    ("hot_float", 8): ("value", "float value(void)"),
    ("timer_int", 7): ("assign", "void assign(int)"),
    ("timer_int", 8): ("truthy", "int truthy(void)"),
    ("timer_int", 9): ("start", "void start(float)"),
    ("timer_int", 10): ("stop", "void stop(void)"),
    ("timer_int", 11): ("update", "void update(float)"),
    ("timer_int", 12): ("time", "float time(void)"),
}


class PeFormatError(ValueError):
    """Raised when a file is not a supported or structurally valid PE image."""


@dataclass(frozen=True)
class Section:
    name: str
    virtual_size: int
    virtual_address: int
    raw_size: int
    raw_offset: int
    characteristics: int


@dataclass(frozen=True)
class MissionPair:
    mission_id: str
    script: Path | None
    rules: Path | None

    @property
    def complete(self) -> bool:
        return self.script is not None and self.rules is not None


def _unpack_from(fmt: str, data: bytes, offset: int, context: str):
    size = struct.calcsize(fmt)
    if offset < 0 or offset + size > len(data):
        raise PeFormatError(f"truncated {context} at file offset 0x{offset:x}")
    return struct.unpack_from(fmt, data, offset)


def _decode_ascii(value: bytes) -> str:
    return value.decode("ascii", errors="replace")


def _timestamp_iso(value: int) -> str | None:
    if value == 0:
        return None
    try:
        return datetime.fromtimestamp(value, tz=timezone.utc).isoformat().replace("+00:00", "Z")
    except (OverflowError, OSError, ValueError):
        return None


class PeImage:
    """Minimal, bounds-checked reader for the PE metadata used by missions."""

    def __init__(self, data: bytes, source: str = "<memory>") -> None:
        self.data = data
        self.source = source
        self.machine = 0
        self.timestamp = 0
        self.characteristics = 0
        self.bits = 0
        self.image_base = 0
        self.subsystem = 0
        self.thunk_size = 0
        self.sections: list[Section] = []
        self.data_directories: list[tuple[int, int]] = []
        self._parse_headers()

    @classmethod
    def from_path(cls, path: Path) -> "PeImage":
        return cls(path.read_bytes(), str(path))

    def _parse_headers(self) -> None:
        if len(self.data) < 64 or self.data[:2] != b"MZ":
            raise PeFormatError(f"{self.source}: missing DOS MZ header")
        (pe_offset,) = _unpack_from("<I", self.data, 0x3C, "DOS e_lfanew")
        if self.data[pe_offset : pe_offset + 4] != PE_SIGNATURE:
            raise PeFormatError(f"{self.source}: missing PE signature")

        coff = pe_offset + 4
        (
            self.machine,
            section_count,
            self.timestamp,
            _symbol_table,
            _symbol_count,
            optional_size,
            self.characteristics,
        ) = _unpack_from("<HHIIIHH", self.data, coff, "COFF header")
        optional = coff + 20
        if optional + optional_size > len(self.data):
            raise PeFormatError(f"{self.source}: truncated optional header")
        (magic,) = _unpack_from("<H", self.data, optional, "optional header magic")
        if magic == 0x10B:
            self.bits = 32
            self.thunk_size = 4
            (self.image_base,) = _unpack_from("<I", self.data, optional + 28, "image base")
            (self.subsystem,) = _unpack_from("<H", self.data, optional + 68, "subsystem")
            (directory_count,) = _unpack_from("<I", self.data, optional + 92, "data directory count")
            directory_offset = optional + 96
        elif magic == 0x20B:
            self.bits = 64
            self.thunk_size = 8
            (self.image_base,) = _unpack_from("<Q", self.data, optional + 24, "image base")
            (self.subsystem,) = _unpack_from("<H", self.data, optional + 68, "subsystem")
            (directory_count,) = _unpack_from("<I", self.data, optional + 108, "data directory count")
            directory_offset = optional + 112
        else:
            raise PeFormatError(f"{self.source}: unsupported optional header 0x{magic:04x}")

        available_directories = max(0, (optional + optional_size - directory_offset) // 8)
        for index in range(min(directory_count, available_directories, 128)):
            self.data_directories.append(
                _unpack_from("<II", self.data, directory_offset + index * 8, f"data directory {index}")
            )

        section_offset = optional + optional_size
        for index in range(section_count):
            offset = section_offset + index * 40
            values = _unpack_from("<8sIIIIIIHHI", self.data, offset, f"section header {index}")
            name = _decode_ascii(values[0].split(b"\0", 1)[0])
            self.sections.append(
                Section(
                    name=name,
                    virtual_size=values[1],
                    virtual_address=values[2],
                    raw_size=values[3],
                    raw_offset=values[4],
                    characteristics=values[9],
                )
            )

    def directory(self, index: int) -> tuple[int, int]:
        if index >= len(self.data_directories):
            return 0, 0
        return self.data_directories[index]

    def rva_to_offset(self, rva: int, size: int = 1) -> int:
        for section in self.sections:
            span = max(section.virtual_size, section.raw_size)
            if section.virtual_address <= rva and rva + size <= section.virtual_address + span:
                relative = rva - section.virtual_address
                if relative + size > section.raw_size:
                    break
                offset = section.raw_offset + relative
                if offset + size <= len(self.data):
                    return offset
                break
        if 0 <= rva and rva + size <= len(self.data):
            return rva
        raise PeFormatError(f"{self.source}: unmapped RVA 0x{rva:x} (size {size})")

    def c_string_at_rva(self, rva: int, limit: int = 16_384) -> str:
        offset = self.rva_to_offset(rva)
        end_limit = min(len(self.data), offset + limit)
        end = self.data.find(b"\0", offset, end_limit)
        if end < 0:
            raise PeFormatError(f"{self.source}: unterminated string at RVA 0x{rva:x}")
        return _decode_ascii(self.data[offset:end])

    def exports(self) -> list[dict[str, object]]:
        directory_rva, directory_size = self.directory(IMAGE_DIRECTORY_ENTRY_EXPORT)
        if not directory_rva or directory_size < 40:
            return []
        offset = self.rva_to_offset(directory_rva, 40)
        values = _unpack_from("<IIHHIIIIIII", self.data, offset, "export directory")
        ordinal_base = values[5]
        function_count = values[6]
        name_count = values[7]
        function_table_rva = values[8]
        name_table_rva = values[9]
        ordinal_table_rva = values[10]
        if function_count > 1_000_000 or name_count > 1_000_000:
            raise PeFormatError(f"{self.source}: implausibly large export table")

        functions_offset = self.rva_to_offset(function_table_rva, function_count * 4)
        names_offset = self.rva_to_offset(name_table_rva, name_count * 4)
        ordinals_offset = self.rva_to_offset(ordinal_table_rva, name_count * 2)
        named_ordinals: set[int] = set()
        exports: list[dict[str, object]] = []
        for index in range(name_count):
            (name_rva,) = _unpack_from("<I", self.data, names_offset + index * 4, "export name RVA")
            (function_index,) = _unpack_from("<H", self.data, ordinals_offset + index * 2, "export ordinal")
            if function_index >= function_count:
                raise PeFormatError(f"{self.source}: export ordinal index is out of range")
            (function_rva,) = _unpack_from(
                "<I", self.data, functions_offset + function_index * 4, "export function RVA"
            )
            named_ordinals.add(function_index)
            item: dict[str, object] = {
                "name": self.c_string_at_rva(name_rva),
                "ordinal": ordinal_base + function_index,
                "rva": function_rva,
            }
            if directory_rva <= function_rva < directory_rva + directory_size:
                item["forwarder"] = self.c_string_at_rva(function_rva)
            exports.append(item)

        for function_index in range(function_count):
            if function_index in named_ordinals:
                continue
            (function_rva,) = _unpack_from(
                "<I", self.data, functions_offset + function_index * 4, "unnamed export RVA"
            )
            if function_rva:
                exports.append(
                    {"name": None, "ordinal": ordinal_base + function_index, "rva": function_rva}
                )
        return sorted(exports, key=lambda item: (item["ordinal"], str(item["name"])))

    def imports(self) -> list[dict[str, object]]:
        directory_rva, _directory_size = self.directory(IMAGE_DIRECTORY_ENTRY_IMPORT)
        if not directory_rva:
            return []
        descriptor_offset = self.rva_to_offset(directory_rva, 20)
        imports: list[dict[str, object]] = []
        ordinal_mask = 1 << (self.thunk_size * 8 - 1)
        thunk_format = "<I" if self.thunk_size == 4 else "<Q"

        for descriptor_index in range(4_096):
            offset = descriptor_offset + descriptor_index * 20
            values = _unpack_from("<IIIII", self.data, offset, "import descriptor")
            if values == (0, 0, 0, 0, 0):
                break
            original_thunk, _timestamp, _forwarder_chain, name_rva, first_thunk = values
            dll_name = self.c_string_at_rva(name_rva)
            thunk_rva = original_thunk or first_thunk
            symbols: list[dict[str, object]] = []
            if thunk_rva:
                thunk_offset = self.rva_to_offset(thunk_rva, self.thunk_size)
                for thunk_index in range(100_000):
                    (thunk_value,) = _unpack_from(
                        thunk_format,
                        self.data,
                        thunk_offset + thunk_index * self.thunk_size,
                        "import thunk",
                    )
                    if thunk_value == 0:
                        break
                    if thunk_value & ordinal_mask:
                        symbols.append(
                            {
                                "ordinal": thunk_value & 0xFFFF,
                                "lookup_rva": thunk_rva + thunk_index * self.thunk_size,
                                "iat_rva": first_thunk + thunk_index * self.thunk_size,
                            }
                        )
                    else:
                        name_offset = self.rva_to_offset(thunk_value, 3)
                        (hint,) = _unpack_from("<H", self.data, name_offset, "import hint")
                        name_end = self.data.find(b"\0", name_offset + 2, min(len(self.data), name_offset + 16_386))
                        if name_end < 0:
                            raise PeFormatError(f"{self.source}: unterminated import name")
                        symbols.append(
                            {
                                "name": _decode_ascii(self.data[name_offset + 2 : name_end]),
                                "hint": hint,
                                "lookup_rva": thunk_rva + thunk_index * self.thunk_size,
                                "iat_rva": first_thunk + thunk_index * self.thunk_size,
                            }
                        )
            imports.append({"library": dll_name, "symbols": symbols})
        else:
            raise PeFormatError(f"{self.source}: import descriptor limit exceeded")
        return imports

    def pdb_paths(self) -> list[str]:
        directory_rva, directory_size = self.directory(IMAGE_DIRECTORY_ENTRY_DEBUG)
        if not directory_rva or directory_size < 28:
            return []
        offset = self.rva_to_offset(directory_rva, directory_size)
        paths: list[str] = []
        for index in range(directory_size // 28):
            values = _unpack_from("<IIHHIIII", self.data, offset + index * 28, "debug directory")
            debug_type = values[4]
            data_size = values[5]
            raw_offset = values[7]
            if debug_type != IMAGE_DEBUG_TYPE_CODEVIEW or data_size < 16:
                continue
            if raw_offset + data_size > len(self.data):
                continue
            block = self.data[raw_offset : raw_offset + data_size]
            if block.startswith(b"RSDS") and len(block) >= 24:
                path_bytes = block[24:].split(b"\0", 1)[0]
            elif block.startswith(b"NB10") and len(block) >= 16:
                path_bytes = block[16:].split(b"\0", 1)[0]
            else:
                continue
            path = _decode_ascii(path_bytes)
            if path and path not in paths:
                paths.append(path)
        return paths

    def follow_relative_jump(self, rva: int) -> int:
        """Follow the single x86 near jump used by MSVC export thunks."""
        offset = self.rva_to_offset(rva, 5)
        if self.data[offset] != 0xE9:
            return rva
        (displacement,) = _unpack_from("<i", self.data, offset + 1, "relative jump")
        return rva + 5 + displacement

    def script_interface(self, exports: Sequence[dict[str, object]]) -> dict[str, int] | None:
        """Recognise Armada II's ScriptExtendInterface version check.

        Stock A2 scripts compare their second argument (``[ebp+0xc]``) with an
        8-bit interface version.  Keeping this deliberately narrow avoids
        pretending to be a general x86 disassembler.
        """
        export = next(
            (item for item in exports if item.get("name") == "ScriptExtendInterface"),
            None,
        )
        if export is None:
            return None
        export_rva = int(export["rva"])
        implementation_rva = self.follow_relative_jump(export_rva)
        offset = self.rva_to_offset(implementation_rva, 64)
        block = self.data[offset : offset + 96]
        match = re.search(rb"\x83\x7d\x0c(.)", block, flags=re.DOTALL)
        if not match:
            return {
                "export_rva": export_rva,
                "implementation_rva": implementation_rva,
            }
        return {
            "version": match.group(1)[0],
            "export_rva": export_rva,
            "implementation_rva": implementation_rva,
        }


def extract_ascii_strings(data: bytes, minimum_length: int = 4) -> list[str]:
    if minimum_length < 4:
        pattern = re.compile(rb"[\x20-\x7e]{%d,}" % minimum_length)
    else:
        pattern = ASCII_RE
    return [_decode_ascii(match.group()) for match in pattern.finditer(data)]


def extract_references(data: bytes) -> dict[str, list[str]]:
    by_extension: dict[str, set[str]] = defaultdict(set)
    for match in REFERENCE_RE.finditer(data):
        value = _decode_ascii(match.group()).strip()
        extension = value.rsplit(".", 1)[-1].casefold()
        if value and extension in REFERENCE_EXTENSIONS:
            by_extension[extension].add(value)
    return {
        extension: sorted(values, key=str.casefold)
        for extension, values in sorted(by_extension.items())
    }


def _unique_matching(strings: Iterable[str], pattern: re.Pattern[str]) -> list[str]:
    return sorted({value for value in strings if pattern.fullmatch(value)}, key=str.casefold)


def demangle_symbols(symbols: Iterable[str], enabled: bool = True) -> dict[str, str]:
    mangled = sorted({symbol for symbol in symbols if symbol.startswith("?")})
    executable = shutil.which("llvm-undname") if enabled else None
    if not executable or not mangled:
        return {}

    demangled: dict[str, str] = {}
    for start in range(0, len(mangled), 256):
        chunk = mangled[start : start + 256]
        result = subprocess.run(
            [executable, *chunk],
            check=False,
            capture_output=True,
            text=True,
        )
        if result.returncode != 0:
            continue
        blocks = re.split(r"\n\s*\n", result.stdout.strip())
        for block in blocks:
            lines = [line.strip() for line in block.splitlines() if line.strip()]
            if len(lines) >= 2 and lines[0].startswith("?"):
                demangled[lines[0]] = lines[1]
    return demangled


def parse_map_symbols(text: str) -> tuple[dict[int, list[str]], dict[str, int]]:
    by_address: dict[int, list[str]] = defaultdict(list)
    by_name: dict[str, int] = {}
    for line in text.splitlines():
        match = MAP_SYMBOL_RE.match(line)
        if not match:
            continue
        name = match.group(1)
        virtual_address = int(match.group(2), 16)
        by_address[virtual_address].append(name)
        by_name.setdefault(name, virtual_address)
    return dict(by_address), by_name


def extract_script_interface(
    executable_path: Path,
    map_path: Path,
    *,
    demangle: bool = True,
) -> dict[str, object]:
    """Resolve ScriptInterfaceImp's vtable using a matching engine map file."""
    executable_data = executable_path.read_bytes()
    pe = PeImage(executable_data, str(executable_path))
    map_data = map_path.read_bytes()
    map_text = map_data.decode("utf-8", errors="ignore")
    symbols_by_address, addresses_by_name = parse_map_symbols(map_text)
    vtable_va = addresses_by_name.get(SCRIPT_INTERFACE_VTABLE_SYMBOL)
    if vtable_va is None:
        raise ValueError(f"{map_path}: ScriptInterfaceImp vtable symbol was not found")
    vtable_offset = pe.rva_to_offset(vtable_va - pe.image_base, 4)

    entries: list[dict[str, object]] = []
    decorated_names: list[str] = []
    for slot in range(4_096):
        (function_va,) = _unpack_from(
            "<I", executable_data, vtable_offset + slot * 4, "ScriptInterface vtable"
        )
        if _code_section_for_va(pe, function_va) is None:
            break
        candidates = symbols_by_address.get(function_va, [])
        decorated = next(
            (name for name in candidates if SCRIPT_INTERFACE_METHOD_RE.match(name)),
            candidates[0] if candidates else "",
        )
        method_match = SCRIPT_INTERFACE_METHOD_RE.match(decorated)
        method_name = method_match.group(1) if method_match else decorated or f"slot_{slot}"
        entry: dict[str, object] = {
            "slot": slot,
            "offset": slot * 4,
            "rva": function_va - pe.image_base,
            "name": method_name,
        }
        if decorated:
            entry["decorated"] = decorated
            decorated_names.append(decorated)
        entries.append(entry)
    if not entries:
        raise ValueError(f"{executable_path}: ScriptInterfaceImp vtable contains no code entries")

    demangled = demangle_symbols(decorated_names, enabled=demangle)
    for entry in entries:
        decorated = entry.get("decorated")
        if isinstance(decorated, str) and decorated in demangled:
            entry["prototype"] = demangled[decorated]
    return {
        "vtable_symbol": SCRIPT_INTERFACE_VTABLE_SYMBOL,
        "vtable_rva": vtable_va - pe.image_base,
        "method_count": len(entries),
        "engine_executable": executable_path.name,
        "engine_executable_sha256": hashlib.sha256(executable_data).hexdigest(),
        "engine_map": map_path.name,
        "engine_map_sha256": hashlib.sha256(map_data).hexdigest(),
        "methods": entries,
    }


def base_symbol(name: str) -> str:
    match = MANGLED_BASE_RE.match(name)
    if match:
        return match.group(1)
    return name


def classify_exports(exports: Sequence[dict[str, object]]) -> dict[str, list[str]]:
    actions: list[str] = []
    state: list[str] = []
    bootstrap: list[str] = []
    other: list[str] = []
    for export in exports:
        raw_name = export.get("name")
        if not isinstance(raw_name, str):
            continue
        name = base_symbol(raw_name)
        demangled = export.get("demangled")
        is_function = (
            isinstance(demangled, str) and "(" in demangled
        ) or "@@YA" in raw_name or name in {"ScriptExtendInterface", "ScriptInit"}
        if name in {"ScriptExtendInterface", "ScriptInit"}:
            bootstrap.append(name)
        elif is_function:
            actions.append(name)
        elif name.startswith("p_") or name == "g_pSI":
            state.append(name)
        else:
            other.append(name)
    return {
        "bootstrap": sorted(set(bootstrap), key=str.casefold),
        "actions": sorted(set(actions), key=str.casefold),
        "state": sorted(set(state), key=str.casefold),
        "other": sorted(set(other), key=str.casefold),
    }


def _code_section_for_va(pe: PeImage, virtual_address: int) -> Section | None:
    for section in pe.sections:
        start = pe.image_base + section.virtual_address
        if start <= virtual_address < start + max(section.virtual_size, section.raw_size):
            if section.name == ".text" or section.characteristics & 0x20:
                return section
    return None


def _function_region(pe: PeImage, thunk_va: int) -> tuple[int, bytes]:
    rva = thunk_va - pe.image_base
    implementation_rva = pe.follow_relative_jump(rva)
    offset = pe.rva_to_offset(implementation_rva)
    section = _code_section_for_va(pe, pe.image_base + implementation_rva)
    if section is None:
        raise PeFormatError(f"{pe.source}: rule thunk points outside executable code")
    section_end = section.raw_offset + section.raw_size
    hard_end = min(len(pe.data), section_end, offset + 8_192)
    padding = pe.data.find(b"\xcc" * 8, offset, hard_end)
    end = padding if padding >= 0 else hard_end
    return implementation_rva, pe.data[offset:end]


def _import_references(
    code: bytes,
    imports: Sequence[dict[str, object]],
    image_base: int,
) -> list[dict[str, object]]:
    occurrences: list[tuple[int, dict[str, object]]] = []
    for library in imports:
        for symbol in library["symbols"]:
            iat_rva = symbol.get("iat_rva")
            if not isinstance(iat_rva, int):
                continue
            needle = struct.pack("<I", image_base + iat_rva)
            start = 0
            while True:
                position = code.find(needle, start)
                if position < 0:
                    break
                reference: dict[str, object] = {
                    "library": library["library"],
                    "iat_rva": iat_rva,
                }
                if "name" in symbol:
                    reference["name"] = symbol["name"]
                    reference["symbol"] = base_symbol(str(symbol["name"]))
                else:
                    reference["ordinal"] = symbol["ordinal"]
                    reference["symbol"] = f"ordinal_{symbol['ordinal']}"
                occurrences.append((position, reference))
                start = position + 1

    unique: list[dict[str, object]] = []
    seen: set[tuple[object, object]] = set()
    for _position, reference in sorted(occurrences, key=lambda item: item[0]):
        key = (reference["library"], reference["symbol"])
        if key not in seen:
            seen.add(key)
            unique.append(reference)
    return unique


def _direct_import_calls(
    code: bytes,
    imports: Sequence[dict[str, object]],
    image_base: int,
) -> list[dict[str, object]]:
    """Recover direct/loaded-register calls through a PE import table.

    Armada I mission callbacks call exported ``Armada.exe`` functions directly,
    unlike Armada II's ScriptInterface vtable.  The retail A1 compiler emits
    either ``call dword ptr [iat]`` or loads an IAT entry into a register and
    calls that register one or more times.  This recogniser intentionally keeps
    to those compiler-emitted forms.
    """
    imports_by_va: dict[int, dict[str, object]] = {}
    for library in imports:
        for symbol in library.get("symbols", []):
            iat_rva = symbol.get("iat_rva")
            if not isinstance(iat_rva, int):
                continue
            record: dict[str, object] = {
                "library": library["library"],
                "iat_rva": iat_rva,
            }
            if isinstance(symbol.get("name"), str):
                record["name"] = symbol["name"]
                record["symbol"] = base_symbol(str(symbol["name"]))
            else:
                record["ordinal"] = symbol.get("ordinal")
                record["symbol"] = f"ordinal_{symbol.get('ordinal')}"
            if isinstance(symbol.get("demangled"), str):
                record["prototype"] = symbol["demangled"]
            imports_by_va[image_base + iat_rva] = record

    register_imports: dict[int, dict[str, object]] = {}
    calls: list[dict[str, object]] = []
    position = 0
    while position < len(code):
        if position + 6 <= len(code) and code[position : position + 2] == b"\xff\x15":
            (address,) = struct.unpack_from("<I", code, position + 2)
            imported = imports_by_va.get(address)
            if imported:
                calls.append({**imported, "code_offset": position, "form": "iat"})
            position += 6
            continue

        if position + 6 <= len(code) and code[position] == 0x8B:
            modrm = code[position + 1]
            if modrm in {0x05, 0x0D, 0x15, 0x1D, 0x25, 0x2D, 0x35, 0x3D}:
                (address,) = struct.unpack_from("<I", code, position + 2)
                imported = imports_by_va.get(address)
                if imported:
                    register_imports[(modrm >> 3) & 7] = imported
                position += 6
                continue

        if position + 5 <= len(code) and code[position] == 0xA1:
            (address,) = struct.unpack_from("<I", code, position + 1)
            imported = imports_by_va.get(address)
            if imported:
                register_imports[0] = imported
            position += 5
            continue

        if position + 2 <= len(code) and code[position] == 0xFF:
            modrm = code[position + 1]
            if 0xD0 <= modrm <= 0xD7:
                imported = register_imports.get(modrm - 0xD0)
                if imported:
                    calls.append(
                        {**imported, "code_offset": position, "form": "register"}
                    )
                position += 2
                continue
        position += 1
    return calls


def _vtable_calls(
    code: bytes,
    imports: Sequence[dict[str, object]],
    image_base: int,
) -> list[dict[str, object]]:
    import_positions: list[tuple[int, str]] = []
    for library in imports:
        for symbol in library["symbols"]:
            iat_rva = symbol.get("iat_rva")
            if not isinstance(iat_rva, int):
                continue
            needle = struct.pack("<I", image_base + iat_rva)
            start = 0
            while True:
                position = code.find(needle, start)
                if position < 0:
                    break
                raw_name = symbol.get("name", f"ordinal_{symbol.get('ordinal')}")
                import_positions.append((position, base_symbol(str(raw_name))))
                start = position + 1
    import_positions.sort()

    calls: list[dict[str, object]] = []
    for position in range(max(0, len(code) - 6)):
        if code[position] != 0xFF:
            continue
        modrm = code[position + 1]
        mode = modrm >> 6
        operation = (modrm >> 3) & 7
        register = modrm & 7
        if operation != 2:
            continue
        if mode == 0 and register != 5:
            offset = 0
        elif mode == 1:
            offset = struct.unpack_from("<b", code, position + 2)[0]
        elif mode == 2:
            offset = struct.unpack_from("<i", code, position + 2)[0]
        else:
            continue
        preceding = [item for item in import_positions if 0 < position - item[0] <= 96]
        owner = preceding[-1][1] if preceding else None
        call: dict[str, object] = {"offset": offset, "code_offset": position}
        if offset >= 0 and offset % 4 == 0:
            call["slot"] = offset // 4
        if owner:
            call["owner"] = owner
        if call not in calls:
            calls.append(call)
    return calls


def _pushed_strings(pe: PeImage, code: bytes) -> list[str]:
    values: set[str] = set()
    for match in re.finditer(rb"\x68(.{4})", code, flags=re.DOTALL):
        (virtual_address,) = struct.unpack("<I", match.group(1))
        try:
            value = pe.c_string_at_rva(virtual_address - pe.image_base, limit=512)
        except PeFormatError:
            continue
        if 0 < len(value) <= 256 and value.isascii() and value.isprintable():
            values.add(value)
    return sorted(values, key=str.casefold)


def _pushed_code_regions(
    pe: PeImage, code: bytes, excluded_rvas: set[int]
) -> list[tuple[int, bytes]]:
    """Find callback functions passed as immediate pointers by a wrapper."""
    regions: list[tuple[int, bytes]] = []
    seen = set(excluded_rvas)
    for match in re.finditer(rb"\x68(.{4})", code, flags=re.DOTALL):
        (virtual_address,) = struct.unpack("<I", match.group(1))
        if _code_section_for_va(pe, virtual_address) is None:
            continue
        candidate_rva = virtual_address - pe.image_base
        try:
            candidate_offset = pe.rva_to_offset(candidate_rva, 3)
        except PeFormatError:
            continue
        prefix = pe.data[candidate_offset : candidate_offset + 3]
        if prefix[:1] != b"\xe9" and prefix != b"\x55\x8b\xec":
            continue
        implementation_rva, callback_code = _function_region(pe, virtual_address)
        if implementation_rva in seen:
            continue
        seen.add(implementation_rva)
        regions.append((implementation_rva, callback_code))
    return regions


def _called_code_regions(
    pe: PeImage,
    code_rva: int,
    code: bytes,
    excluded_rvas: set[int],
    reference_libraries: Sequence[dict[str, object]],
) -> list[tuple[int, bytes]]:
    """Find directly called mission helpers reached by a small export wrapper."""
    regions: list[tuple[int, bytes]] = []
    seen = set(excluded_rvas)
    for match in re.finditer(rb"\xe8(.{4})", code, flags=re.DOTALL):
        (displacement,) = struct.unpack("<i", match.group(1))
        target_rva = code_rva + match.start() + 5 + displacement
        if target_rva < 0 or _code_section_for_va(pe, pe.image_base + target_rva) is None:
            continue
        implementation_rva, helper_code = _function_region(
            pe, pe.image_base + target_rva
        )
        if implementation_rva in seen:
            continue
        references = _import_references(
            helper_code, reference_libraries, pe.image_base
        )
        if not references:
            continue
        seen.add(implementation_rva)
        regions.append((implementation_rva, helper_code))
    return regions


def _export_reference_library(
    exports: Sequence[dict[str, object]], source_name: str
) -> list[dict[str, object]]:
    """Represent exported addresses like import slots for code-reference scans."""
    symbols = [
        {"name": export["name"], "iat_rva": export["rva"]}
        for export in exports
        if isinstance(export.get("name"), str) and isinstance(export.get("rva"), int)
    ]
    return [{"library": source_name, "symbols": symbols}]


def _resolved_interface_calls(
    vtable_calls: Sequence[dict[str, object]],
    interface_methods: dict[int, dict[str, object]] | None,
) -> tuple[list[int], list[dict[str, object]]]:
    slots = sorted(
        {
            int(call["slot"])
            for call in vtable_calls
            if call.get("owner") == "g_pSI" and "slot" in call
        }
    )
    resolved: list[dict[str, object]] = []
    seen_slots: set[int] = set()
    for vtable_call in vtable_calls:
        if vtable_call.get("owner") != "g_pSI" or "slot" not in vtable_call:
            continue
        slot = int(vtable_call["slot"])
        if slot in seen_slots:
            continue
        seen_slots.add(slot)
        call: dict[str, object] = {"slot": slot, "offset": slot * 4}
        if interface_methods and slot in interface_methods:
            method = interface_methods[slot]
            call["name"] = method["name"]
            if "prototype" in method:
                call["prototype"] = method["prototype"]
        resolved.append(call)
    return slots, resolved


def _prototype_parameters(prototype: object) -> list[str]:
    if not isinstance(prototype, str):
        return []
    start = prototype.rfind("(")
    end = prototype.rfind(")")
    if start < 0 or end <= start:
        return []
    value = prototype[start + 1 : end].strip()
    if not value or value == "void":
        return []
    return [part.strip() for part in value.split(",")]


def _symbols_by_va(
    reference_libraries: Sequence[dict[str, object]], image_base: int
) -> dict[int, str]:
    result: dict[int, str] = {}
    for library in reference_libraries:
        for symbol in library["symbols"]:
            rva = symbol.get("iat_rva")
            name = symbol.get("name")
            if isinstance(rva, int) and isinstance(name, str):
                result[image_base + rva] = base_symbol(name)
    return result


def _immediate_value(
    pe: PeImage, value: int, symbols: dict[int, str]
) -> dict[str, object]:
    if value in symbols:
        return {"kind": "symbol_address", "symbol": symbols[value], "raw": value}
    try:
        string = pe.c_string_at_rva(value - pe.image_base, limit=512)
    except PeFormatError:
        string = ""
    if string and string.isascii() and string.isprintable():
        return {"kind": "string", "value": string, "raw": value}
    return {"kind": "immediate", "value": value}


def _typed_argument(value: dict[str, object], parameter_type: str) -> dict[str, object]:
    result = dict(value)
    result["parameter_type"] = parameter_type
    if value.get("kind") != "immediate":
        return result
    raw = int(value["value"])
    if "float" in parameter_type:
        result["kind"] = "float"
        result["value"] = struct.unpack("<f", struct.pack("<I", raw & 0xFFFFFFFF))[0]
    elif "bool" in parameter_type:
        result["kind"] = "boolean"
        result["value"] = bool(raw)
    elif raw & 0x80000000:
        result["value"] = raw - 0x1_0000_0000
    return result


def _call_arguments(
    pe: PeImage,
    code: bytes,
    call_offset: int,
    prototype: object,
    symbols: dict[int, str],
) -> tuple[list[dict[str, object]], bool]:
    """Recover simple MSVC debug-build push arguments before a virtual call."""
    parameters = _prototype_parameters(prototype)
    marker = code.rfind(b"\x8b\xf4", max(0, call_offset - 384), call_offset)
    if marker < 0:
        marker = code.rfind(b"\x8b\xfc", max(0, call_offset - 384), call_offset)
    if marker < 0:
        return [], not parameters

    pushes: list[dict[str, object]] = []
    registers: dict[int, dict[str, object]] = {}
    position = marker + 2
    while position < call_offset:
        opcode = code[position]
        if opcode == 0x6A and position + 1 < call_offset:
            raw = struct.unpack_from("<b", code, position + 1)[0] & 0xFFFFFFFF
            pushes.append({"kind": "immediate", "value": raw})
            position += 2
            continue
        if opcode == 0x68 and position + 4 < call_offset:
            (raw,) = struct.unpack_from("<I", code, position + 1)
            pushes.append(_immediate_value(pe, raw, symbols))
            position += 5
            continue
        if opcode == 0xA1 and position + 4 < call_offset:
            (address,) = struct.unpack_from("<I", code, position + 1)
            registers[0] = {
                "kind": "symbol_value",
                "symbol": symbols.get(address, f"address_0x{address:08x}"),
                "raw": address,
            }
            position += 5
            continue
        if opcode == 0x8B and position + 2 < call_offset:
            modrm = code[position + 1]
            if modrm == 0x45:
                displacement = struct.unpack_from("<b", code, position + 2)[0]
                registers[0] = {
                    "kind": "function_parameter",
                    "index": max(0, (displacement - 8) // 4),
                    "stack_offset": displacement,
                }
                position += 3
                continue
            if modrm in {0x05, 0x0D, 0x15, 0x1D, 0x25, 0x2D, 0x35, 0x3D} and position + 5 < call_offset:
                register = (modrm >> 3) & 7
                (address,) = struct.unpack_from("<I", code, position + 2)
                registers[register] = {
                    "kind": "symbol_value",
                    "symbol": symbols.get(address, f"address_0x{address:08x}"),
                    "raw": address,
                }
                position += 6
                continue
        if (
            opcode == 0x51
            and position + 3 < call_offset
            and code[position + 1 : position + 4] == b"\xd9\x1c\x24"
        ):
            pushes.append({"kind": "x87_result"})
            position += 4
            continue
        if 0x50 <= opcode <= 0x57:
            register = opcode - 0x50
            pushes.append(registers.get(register, {"kind": "register", "register": register}))
            position += 1
            continue
        if opcode == 0xFF and position + 1 < call_offset:
            modrm = code[position + 1]
            if modrm == 0x35 and position + 5 < call_offset:
                (address,) = struct.unpack_from("<I", code, position + 2)
                pushes.append(
                    {
                        "kind": "symbol_value",
                        "symbol": symbols.get(address, f"address_0x{address:08x}"),
                        "raw": address,
                    }
                )
                position += 6
                continue
        position += 1

    if len(pushes) < len(parameters):
        return list(reversed(pushes)), False
    if not parameters:
        return [], True
    selected = list(reversed(pushes[-len(parameters) :]))
    arguments = [
        _typed_argument(value, parameter_type)
        for value, parameter_type in zip(selected, parameters)
    ]
    exact = len(pushes) == len(parameters) and all(
        argument.get("kind") not in {"register"} for argument in arguments
    )
    return arguments, exact


def _interface_sequence(
    pe: PeImage,
    code: bytes,
    vtable_calls: Sequence[dict[str, object]],
    reference_libraries: Sequence[dict[str, object]],
    interface_methods: dict[int, dict[str, object]] | None,
) -> list[dict[str, object]]:
    symbols = _symbols_by_va(reference_libraries, pe.image_base)
    sequence: list[dict[str, object]] = []
    previous_result: str | None = None
    for vtable_call in vtable_calls:
        if vtable_call.get("owner") != "g_pSI" or "slot" not in vtable_call:
            continue
        slot = int(vtable_call["slot"])
        call: dict[str, object] = {
            "slot": slot,
            "offset": slot * 4,
            "code_offset": int(vtable_call["code_offset"]),
        }
        method = interface_methods.get(slot) if interface_methods else None
        if method:
            call["name"] = method["name"]
            prototype = method.get("prototype")
            if prototype:
                call["prototype"] = prototype
            arguments, exact = _call_arguments(
                pe, code, int(vtable_call["code_offset"]), prototype, symbols
            )
            for argument in arguments:
                if argument.get("kind") == "x87_result":
                    if previous_result:
                        argument["kind"] = "call_result"
                        argument["call"] = previous_result
                    else:
                        exact = False
            call["arguments"] = arguments
            call["arguments_exact"] = exact
            if isinstance(prototype, str) and " virtual void __thiscall " not in prototype:
                call_offset = int(vtable_call["code_offset"])
                store_match = re.search(
                    rb"\xa3(.{4})", code[call_offset + 2 : call_offset + 40]
                )
                if store_match:
                    (store_address,) = struct.unpack("<I", store_match.group(1))
                    result_symbol = symbols.get(store_address)
                    if result_symbol:
                        call["result_symbol"] = result_symbol
            if isinstance(prototype, str) and " float __thiscall " in prototype:
                previous_result = str(method["name"])
        sequence.append(call)
    return sequence


def _export_symbol_types(
    exports: Sequence[dict[str, object]],
) -> dict[str, str]:
    result: dict[str, str] = {}
    for export in exports:
        raw_name = export.get("name")
        demangled = export.get("demangled")
        if not isinstance(raw_name, str) or not isinstance(demangled, str):
            continue
        match = re.search(r"class ([A-Za-z_][A-Za-z0-9_]*) \*", demangled)
        if match:
            result[base_symbol(raw_name)] = match.group(1)
    return result


def _state_sequence(
    pe: PeImage,
    code: bytes,
    vtable_calls: Sequence[dict[str, object]],
    reference_libraries: Sequence[dict[str, object]],
    symbol_types: dict[str, str],
) -> list[dict[str, object]]:
    symbols = _symbols_by_va(reference_libraries, pe.image_base)
    sequence: list[dict[str, object]] = []
    for vtable_call in vtable_calls:
        owner = vtable_call.get("owner")
        slot_value = vtable_call.get("slot")
        if not isinstance(owner, str) or not isinstance(slot_value, int):
            continue
        symbol_type = symbol_types.get(owner)
        method = STATE_VTABLE_METHODS.get((str(symbol_type), slot_value))
        if method is None:
            continue
        name, prototype = method
        arguments, exact = _call_arguments(
            pe,
            code,
            int(vtable_call["code_offset"]),
            prototype,
            symbols,
        )
        sequence.append(
            {
                "owner": owner,
                "owner_type": symbol_type,
                "slot": slot_value,
                "name": name,
                "prototype": prototype,
                "code_offset": int(vtable_call["code_offset"]),
                "arguments": arguments,
                "arguments_exact": exact,
            }
        )
    return sequence


def extract_action_topology(
    pe: PeImage,
    exports: Sequence[dict[str, object]],
    script_classification: dict[str, list[str]],
    interface_methods: dict[int, dict[str, object]] | None = None,
    direct_imports: Sequence[dict[str, object]] | None = None,
    *,
    follow_direct_helpers: bool = False,
) -> list[dict[str, object]]:
    """Inventory observable structure in exported DSL action bodies."""
    action_names = set(script_classification["actions"])
    category_by_symbol = {
        symbol: category
        for category in ("bootstrap", "actions", "state", "other")
        for symbol in script_classification[category]
    }
    reference_library = _export_reference_library(exports, Path(pe.source).name)
    symbol_types = _export_symbol_types(exports)
    topology: list[dict[str, object]] = []
    seen: set[tuple[str, int]] = set()

    for export in exports:
        raw_name = export.get("name")
        export_rva = export.get("rva")
        if not isinstance(raw_name, str) or not isinstance(export_rva, int):
            continue
        name = base_symbol(raw_name)
        if name not in action_names or (name, export_rva) in seen:
            continue
        if _code_section_for_va(pe, pe.image_base + export_rva) is None:
            continue

        implementation_rva, code = _function_region(pe, pe.image_base + export_rva)
        regions = [(implementation_rva, code)]
        regions.extend(_pushed_code_regions(pe, code, {implementation_rva}))
        helper_regions: list[tuple[int, bytes]] = []
        if follow_direct_helpers:
            helper_regions = _called_code_regions(
                pe,
                implementation_rva,
                code,
                {region_rva for region_rva, _ in regions},
                reference_library,
            )
            regions.extend(helper_regions)
        references_by_symbol: dict[tuple[object, object], dict[str, object]] = {}
        vtable_calls: list[dict[str, object]] = []
        strings: set[str] = set()
        callback_records: list[dict[str, object]] = []
        for region_index, (region_rva, region_code) in enumerate(regions):
            region_references = _import_references(
                region_code, reference_library, pe.image_base
            )
            for reference in region_references:
                key = (reference["library"], reference["symbol"])
                references_by_symbol.setdefault(key, reference)
            region_vtable_calls = _vtable_calls(
                region_code, reference_library, pe.image_base
            )
            for call in region_vtable_calls:
                if call not in vtable_calls:
                    vtable_calls.append(call)
            region_strings = _pushed_strings(pe, region_code)
            strings.update(region_strings)
            if region_index:
                callback_records.append(
                    {
                        "rva": region_rva,
                        "referenced_symbols": [
                            str(reference["symbol"])
                            for reference in region_references
                        ],
                        "strings": region_strings,
                        "script_interface_sequence": _interface_sequence(
                            pe,
                            region_code,
                            region_vtable_calls,
                            reference_library,
                            interface_methods,
                        ),
                        "state_sequence": _state_sequence(
                            pe,
                            region_code,
                            region_vtable_calls,
                            reference_library,
                            symbol_types,
                        ),
                        "direct_import_calls": _direct_import_calls(
                            region_code,
                            direct_imports or [],
                            pe.image_base,
                        ),
                    }
                )
        references = list(references_by_symbol.values())
        symbols = [str(reference["symbol"]) for reference in references]
        symbol_categories: dict[str, list[str]] = defaultdict(list)
        for symbol in symbols:
            symbol_categories[category_by_symbol.get(symbol, "unclassified")].append(
                symbol
            )
        interface_slots, interface_calls = _resolved_interface_calls(
            vtable_calls, interface_methods
        )
        topology.append(
            {
                "name": name,
                "export_rva": export_rva,
                "implementation_rva": implementation_rva,
                "callback_rvas": [
                    region_rva for region_rva, _region_code in regions[1:]
                ],
                "helper_rvas": [region_rva for region_rva, _ in helper_regions],
                "callbacks": callback_records,
                "referenced_symbols": symbols,
                "referenced_actions": symbol_categories["actions"],
                "referenced_state": symbol_categories["state"],
                "referenced_objects": symbol_categories["other"],
                "unclassified_symbols": symbol_categories["unclassified"],
                "vtable_calls": vtable_calls,
                "script_interface_slots": interface_slots,
                "script_interface_calls": interface_calls,
                "strings": sorted(strings, key=str.casefold),
            }
        )
        seen.add((name, export_rva))

    return sorted(topology, key=lambda action: int(action["export_rva"]))


def extract_rule_topology(
    pe: PeImage,
    imports: Sequence[dict[str, object]],
    script_name: str,
    script_classification: dict[str, list[str]],
    interface_methods: dict[int, dict[str, object]] | None = None,
) -> list[dict[str, object]]:
    """Recover compiler-emitted DRL rule registrations.

    Debug-built Armada II DRLs register each rule using three adjacent
    ``push imm32`` instructions: rule-name string, action thunk, then condition
    thunk. Candidates are accepted only when the name is an identifier, both
    thunks point into executable code, and the action references the matching
    DSL. This is intentionally a narrow structural recogniser.
    """
    category_by_symbol = {
        symbol: category
        for category in ("bootstrap", "actions", "state", "other")
        for symbol in script_classification[category]
    }
    symbol_types: dict[str, str] = {}
    for library in imports:
        if str(library["library"]).casefold() != script_name.casefold():
            continue
        for imported_symbol in library["symbols"]:
            raw_name = imported_symbol.get("name")
            demangled = imported_symbol.get("demangled")
            if not isinstance(raw_name, str) or not isinstance(demangled, str):
                continue
            match = re.search(r"class ([A-Za-z_][A-Za-z0-9_]*) \*", demangled)
            if match:
                symbol_types[base_symbol(raw_name)] = match.group(1)
    topology: list[dict[str, object]] = []
    seen_registrations: set[int] = set()

    for section in pe.sections:
        if section.name != ".text" and not section.characteristics & 0x20:
            continue
        blob = pe.data[section.raw_offset : section.raw_offset + section.raw_size]
        for match in re.finditer(rb"\x68(.{4})\x68(.{4})\x68(.{4})", blob, flags=re.DOTALL):
            name_va, action_va, condition_va = struct.unpack(
                "<III", match.group(1) + match.group(2) + match.group(3)
            )
            if _code_section_for_va(pe, action_va) is None or _code_section_for_va(pe, condition_va) is None:
                continue
            try:
                name = pe.c_string_at_rva(name_va - pe.image_base, limit=256)
            except PeFormatError:
                continue
            if not RULE_NAME_RE.fullmatch(name):
                continue

            registration_rva = section.virtual_address + match.start()
            if registration_rva in seen_registrations:
                continue
            condition_rva, condition_code = _function_region(pe, condition_va)
            action_rva, action_code = _function_region(pe, action_va)
            condition_refs = _import_references(condition_code, imports, pe.image_base)
            action_refs = _import_references(action_code, imports, pe.image_base)
            script_condition_refs = [
                reference
                for reference in condition_refs
                if str(reference["library"]).casefold() == script_name.casefold()
            ]
            script_action_refs = [
                reference
                for reference in action_refs
                if str(reference["library"]).casefold() == script_name.casefold()
            ]
            action_symbols = [
                str(reference["symbol"])
                for reference in script_action_refs
                if category_by_symbol.get(str(reference["symbol"])) == "actions"
            ]
            if not action_symbols:
                continue

            condition_symbols = [str(reference["symbol"]) for reference in script_condition_refs]
            condition_categories: dict[str, list[str]] = defaultdict(list)
            for symbol in condition_symbols:
                condition_categories[category_by_symbol.get(symbol, "unclassified")].append(symbol)
            vtable_calls = _vtable_calls(condition_code, imports, pe.image_base)
            interface_slots, interface_calls = _resolved_interface_calls(
                vtable_calls, interface_methods
            )
            condition_ir: dict[str, object] | None = None
            if len(condition_symbols) == 1:
                condition_symbol = condition_symbols[0]
                truth_calls = [
                    call
                    for call in vtable_calls
                    if call.get("owner") == condition_symbol and call.get("slot") == 8
                ]
                owned_calls = [
                    call for call in vtable_calls if call.get("owner") == condition_symbol
                ]
                if len(truth_calls) == 1 and len(owned_calls) == 1:
                    symbol_type = symbol_types.get(condition_symbol)
                    if symbol_type in {"hot_int", "timer_int"}:
                        condition_ir = {
                            "kind": (
                                "timer_expired"
                                if symbol_type == "timer_int"
                                else "state_truthy"
                            ),
                            "symbol": condition_symbol,
                            "symbol_type": symbol_type,
                            "confidence": "high",
                        }
            topology.append(
                {
                    "name": name,
                    "registration_rva": registration_rva,
                    "condition_thunk_rva": condition_va - pe.image_base,
                    "condition_rva": condition_rva,
                    "action_thunk_rva": action_va - pe.image_base,
                    "action_rva": action_rva,
                    "actions": action_symbols,
                    "condition_symbols": condition_symbols,
                    "condition_actions": condition_categories["actions"],
                    "condition_state": condition_categories["state"],
                    "condition_objects": condition_categories["other"],
                    "condition_unclassified": condition_categories["unclassified"],
                    "condition_vtable_calls": vtable_calls,
                    "script_interface_slots": interface_slots,
                    "script_interface_calls": interface_calls,
                    "condition_strings": _pushed_strings(pe, condition_code),
                    "condition_ir": condition_ir,
                    "confidence": "high" if len(action_symbols) == 1 else "medium",
                }
            )
            seen_registrations.add(registration_rva)
    return sorted(topology, key=lambda rule: int(rule["registration_rva"]))


def correlate_rule_bindings(
    script: dict[str, object], rules: dict[str, object]
) -> dict[str, object]:
    """Match a DRL's DSL imports to the script's exported mission symbols."""
    script_name = str(script["name"])
    script_imports: list[dict[str, object]] = []
    for library in rules["imports"]:
        if str(library["library"]).casefold() == script_name.casefold():
            script_imports.extend(library["symbols"])

    imported_names = [
        str(symbol["name"])
        for symbol in script_imports
        if isinstance(symbol.get("name"), str)
    ]
    export_names = {
        str(export["name"])
        for export in script["exports"]
        if isinstance(export.get("name"), str)
    }
    imported_set = set(imported_names)
    classification = script["classification"]
    category_by_base = {
        name: category
        for category in ("bootstrap", "actions", "state", "other")
        for name in classification[category]
    }
    categories: dict[str, list[str]] = defaultdict(list)
    for raw_name in imported_names:
        categories[category_by_base.get(base_symbol(raw_name), "unclassified")].append(
            base_symbol(raw_name)
        )

    return {
        "library": script_name,
        "imported_symbols": len(imported_names),
        "matched_exports": len(imported_set & export_names),
        "unresolved_imports": sorted(imported_set - export_names),
        "unreferenced_exports": sorted(export_names - imported_set),
        "bootstrap": sorted(set(categories["bootstrap"]), key=str.casefold),
        "actions": sorted(set(categories["actions"]), key=str.casefold),
        "state": sorted(set(categories["state"]), key=str.casefold),
        "other": sorted(set(categories["other"]), key=str.casefold),
        "unclassified": sorted(set(categories["unclassified"]), key=str.casefold),
    }


def pair_missions(root: Path) -> tuple[list[MissionPair], list[str]]:
    slots: dict[str, dict[str, Path]] = defaultdict(dict)
    display_ids: dict[str, str] = {}
    warnings: list[str] = []
    for path in sorted(root.iterdir(), key=lambda item: item.name.casefold()):
        if not path.is_file() or path.suffix.casefold() not in {".dsl", ".drl"}:
            continue
        stem = path.stem
        kind = stem[-1:].casefold()
        expected_extension = ".dsl" if kind == "s" else ".drl" if kind == "r" else ""
        if not expected_extension or path.suffix.casefold() != expected_extension:
            warnings.append(f"unrecognised mission filename: {path.name}")
            continue
        mission_id = stem[:-1]
        key = mission_id.casefold()
        if kind in slots[key]:
            warnings.append(f"duplicate {kind.upper()} file for {mission_id}: {path.name}")
            continue
        slots[key][kind] = path
        display_ids.setdefault(key, mission_id)

    pairs = [
        MissionPair(display_ids[key], values.get("s"), values.get("r"))
        for key, values in slots.items()
    ]
    pairs.sort(key=lambda pair: natural_key(pair.mission_id))
    return pairs, warnings


def natural_key(value: str) -> list[object]:
    return [int(part) if part.isdigit() else part.casefold() for part in re.split(r"(\d+)", value)]


def _named_symbols(imports: Sequence[dict[str, object]]) -> Iterable[str]:
    for library in imports:
        for symbol in library.get("symbols", []):
            name = symbol.get("name")
            if isinstance(name, str):
                yield name


def inspect_file(path: Path, demangle: bool, include_strings: bool) -> dict[str, object]:
    data = path.read_bytes()
    pe = PeImage(data, str(path))
    exports = pe.exports()
    imports = pe.imports()
    symbol_names = [item["name"] for item in exports if isinstance(item.get("name"), str)]
    symbol_names.extend(_named_symbols(imports))
    demangled = demangle_symbols(symbol_names, enabled=demangle)
    for export in exports:
        name = export.get("name")
        if isinstance(name, str) and name in demangled:
            export["demangled"] = demangled[name]
    for library in imports:
        for symbol in library["symbols"]:
            name = symbol.get("name")
            if isinstance(name, str) and name in demangled:
                symbol["demangled"] = demangled[name]

    strings = extract_ascii_strings(data)
    result: dict[str, object] = {
        "name": path.name,
        "size": len(data),
        "sha256": hashlib.sha256(data).hexdigest(),
        "pe": {
            "machine": MACHINE_NAMES.get(pe.machine, f"0x{pe.machine:04x}"),
            "bits": pe.bits,
            "timestamp": _timestamp_iso(pe.timestamp),
            "image_base": pe.image_base,
            "subsystem": pe.subsystem,
            "characteristics": pe.characteristics,
            "sections": [
                {
                    "name": section.name,
                    "virtual_address": section.virtual_address,
                    "virtual_size": section.virtual_size,
                    "raw_offset": section.raw_offset,
                    "raw_size": section.raw_size,
                    "characteristics": section.characteristics,
                }
                for section in pe.sections
            ],
            "pdb_paths": pe.pdb_paths(),
        },
        "imports": imports,
        "exports": exports,
        "references": extract_references(data),
        "world_names": _unique_matching(strings, WORLD_NAME_RE),
        "localized_keys": _unique_matching(strings, LOCALIZED_KEY_RE),
    }
    script_interface = pe.script_interface(exports)
    if script_interface is not None:
        result["script_interface"] = script_interface
    if include_strings:
        result["ascii_strings"] = strings
    return result


def analyze_directory(
    root: Path,
    *,
    demangle: bool = True,
    include_strings: bool = False,
    script_interface: dict[str, object] | None = None,
    include_non_a2: bool = False,
) -> dict[str, object]:
    if not root.is_dir():
        raise ValueError(f"mission directory does not exist: {root}")
    pairs, warnings = pair_missions(root)
    non_a2_pairs = [pair for pair in pairs if not pair.mission_id.casefold().startswith("a2_")]
    if not include_non_a2:
        for pair in non_a2_pairs:
            warnings.append(f"skipped non-Armada-II pair: {pair.mission_id}")
        pairs = [pair for pair in pairs if pair.mission_id.casefold().startswith("a2_")]
    if not pairs:
        expected = "compiled mission pairs" if include_non_a2 else "Armada II mission pairs"
        raise ValueError(f"no {expected} found in {root}")

    missions: list[dict[str, object]] = []
    interface_methods = {
        int(method["slot"]): method for method in script_interface.get("methods", [])
    } if script_interface else None
    for pair in pairs:
        mission: dict[str, object] = {"id": pair.mission_id, "complete": pair.complete}
        if pair.script:
            script = inspect_file(pair.script, demangle, include_strings)
            script["classification"] = classify_exports(script["exports"])
            mission["script"] = script
            mission["action_topology"] = extract_action_topology(
                PeImage.from_path(pair.script),
                script["exports"],
                script["classification"],
                interface_methods,
                script["imports"],
            )
            bootstrap_classification = {
                key: list(values)
                for key, values in script["classification"].items()
            }
            bootstrap_classification["actions"] = list(
                script["classification"]["bootstrap"]
            )
            mission["bootstrap_topology"] = extract_action_topology(
                PeImage.from_path(pair.script),
                script["exports"],
                bootstrap_classification,
                interface_methods,
                script["imports"],
                follow_direct_helpers=True,
            )
        if pair.rules:
            mission["rules"] = inspect_file(pair.rules, demangle, include_strings)
        if pair.script and pair.rules:
            mission["rule_bindings"] = correlate_rule_bindings(
                mission["script"], mission["rules"]
            )
            mission["rule_topology"] = extract_rule_topology(
                PeImage.from_path(pair.rules),
                mission["rules"]["imports"],
                str(mission["script"]["name"]),
                mission["script"]["classification"],
                interface_methods,
            )
            expected_actions = set(mission["rule_bindings"]["actions"])
            recovered_actions = {
                action
                for rule in mission["rule_topology"]
                for action in rule["actions"]
            }
            missing_actions = sorted(
                expected_actions - recovered_actions, key=str.casefold
            )
            mission["rule_topology_coverage"] = {
                "expected_actions": len(expected_actions),
                "recovered_actions": len(expected_actions & recovered_actions),
                "missing_actions": missing_actions,
            }
            if missing_actions:
                warnings.append(
                    f"{pair.mission_id}: rule topology did not recover actions: "
                    + ", ".join(missing_actions)
                )
        if not pair.complete:
            missing = "script DSL" if pair.script is None else "rule DRL"
            warnings.append(f"{pair.mission_id}: missing {missing}")
        missions.append(mission)

    interface_versions = sorted(
        {
            mission["script"]["script_interface"]["version"]
            for mission in missions
            if "script_interface" in mission.get("script", {})
            and "version" in mission["script"]["script_interface"]
        }
    )
    condition_interface_slots = sorted(
        {
            slot
            for mission in missions
            for rule in mission.get("rule_topology", [])
            for slot in rule["script_interface_slots"]
        }
    )
    action_interface_slots = sorted(
        {
            slot
            for mission in missions
            for action in mission.get("action_topology", [])
            for slot in action["script_interface_slots"]
        }
    )
    result: dict[str, object] = {
        "schema_version": SCHEMA_VERSION,
        "format": "sta2-compiled-mission-manifest",
        "source_root": str(root.resolve()),
        "summary": {
            "missions": len(missions),
            "complete_pairs": sum(1 for mission in missions if mission["complete"]),
            "script_files": sum("script" in mission for mission in missions),
            "rule_files": sum("rules" in mission for mission in missions),
            "script_interface_versions": interface_versions,
            "compiled_rules": sum(len(mission.get("rule_topology", [])) for mission in missions),
            "exact_simple_conditions": sum(
                1
                for mission in missions
                for rule in mission.get("rule_topology", [])
                if rule.get("condition_ir") is not None
            ),
            "action_bodies_inspected": sum(
                len(mission.get("action_topology", [])) for mission in missions
            ),
            "action_callbacks": sum(
                len(action.get("callbacks", []))
                for mission in missions
                for action in mission.get("action_topology", [])
            ),
            "action_script_interface_calls": sum(
                len(callback.get("script_interface_sequence", []))
                for mission in missions
                for action in mission.get("action_topology", [])
                for callback in action.get("callbacks", [])
            ),
            "exact_action_call_arguments": sum(
                1
                for mission in missions
                for action in mission.get("action_topology", [])
                for callback in action.get("callbacks", [])
                for call in callback.get("script_interface_sequence", [])
                if call.get("arguments_exact")
            ),
            "bootstrap_helpers": sum(
                len(entry.get("helper_rvas", []))
                for mission in missions
                for entry in mission.get("bootstrap_topology", [])
            ),
            "bootstrap_state_operations": sum(
                len(callback.get("state_sequence", []))
                for mission in missions
                for entry in mission.get("bootstrap_topology", [])
                for callback in entry.get("callbacks", [])
            ),
            "missions_with_complete_action_topology": sum(
                1
                for mission in missions
                if "rule_topology_coverage" in mission
                and not mission["rule_topology_coverage"]["missing_actions"]
            ),
            "condition_script_interface_slots": condition_interface_slots,
            "action_script_interface_slots": action_interface_slots,
        },
        "warnings": warnings,
        "missions": missions,
    }
    if script_interface:
        result["script_interface"] = script_interface
    return result


def _reference_count(file_data: dict[str, object]) -> int:
    references = file_data.get("references", {})
    return sum(len(values) for values in references.values())


def _markdown_list(values: Sequence[str], empty: str = "None found") -> str:
    if not values:
        return empty
    return ", ".join(f"`{value}`" for value in values)


def render_markdown(manifest: dict[str, object]) -> str:
    summary = manifest["summary"]
    lines = [
        "# Armada II compiled mission analysis",
        "",
        "> Clean-room research output. The source DLLs were inspected as data and were not executed.",
        "",
        f"- Missions: {summary['missions']}",
        f"- Complete S/R pairs: {summary['complete_pairs']}",
        f"- Script DSL files: {summary['script_files']}",
        f"- Rule DRL files: {summary['rule_files']}",
        f"- ScriptInterface versions: {_markdown_list([str(value) for value in summary['script_interface_versions']])}",
        f"- Compiled rules recovered: {summary['compiled_rules']}",
        f"- Exact simple conditions: {summary['exact_simple_conditions']}",
        f"- DSL action bodies inspected: {summary['action_bodies_inspected']}",
        f"- Action callbacks recovered: {summary['action_callbacks']}",
        "- Exact action call arguments: "
        f"{summary['exact_action_call_arguments']}/{summary['action_script_interface_calls']}",
        f"- Bootstrap helpers followed: {summary['bootstrap_helpers']}",
        f"- Bootstrap state operations: {summary['bootstrap_state_operations']}",
        "- Missions with complete action topology: "
        f"{summary['missions_with_complete_action_topology']}/{summary['missions']}",
        f"- ScriptInterface condition slots observed: {len(summary['condition_script_interface_slots'])}",
        f"- ScriptInterface action slots observed: {len(summary['action_script_interface_slots'])}",
        "",
    ]
    if "script_interface" in manifest:
        lines.insert(-1, f"- ScriptInterface methods resolved: {manifest['script_interface']['method_count']}")
    warnings = manifest.get("warnings", [])
    if warnings:
        lines.extend(["## Warnings", ""])
        lines.extend(f"- {warning}" for warning in warnings)
        lines.append("")

    lines.extend(
        [
            "## Mission inventory",
            "",
            "| Mission | Script exports | Actions | State | Rules | Rule bindings | References |",
            "|---|---:|---:|---:|---:|---:|---:|",
        ]
    )
    for mission in manifest["missions"]:
        script = mission.get("script", {})
        rules = mission.get("rules", {})
        classification = script.get("classification", {})
        rule_binding_count = mission.get("rule_bindings", {}).get("imported_symbols", 0)
        reference_count = _reference_count(script) + _reference_count(rules)
        lines.append(
            f"| `{mission['id']}` | {len(script.get('exports', []))} | "
            f"{len(classification.get('actions', []))} | {len(classification.get('state', []))} | "
            f"{len(mission.get('rule_topology', []))} | {rule_binding_count} | {reference_count} |"
        )
    lines.append("")

    lines.extend(["## Mission details", ""])
    for mission in manifest["missions"]:
        lines.extend([f"### {mission['id']}", ""])
        script = mission.get("script")
        rules = mission.get("rules")
        if script:
            classification = script["classification"]
            lines.extend(
                [
                    f"- Script: `{script['name']}` ({script['size']} bytes)",
                    f"- Bootstrap: {_markdown_list(classification['bootstrap'])}",
                    f"- Actions/functions: {_markdown_list(classification['actions'])}",
                    f"- State pointers: {_markdown_list(classification['state'])}",
                    f"- WORLD steps: {_markdown_list(script['world_names'])}",
                ]
            )
            pdb_paths = script["pe"]["pdb_paths"]
            if pdb_paths:
                lines.append(f"- Script PDB path: `{pdb_paths[0]}`")
        if rules:
            rule_imports = sum(len(item["symbols"]) for item in rules["imports"])
            lines.append(f"- Rules: `{rules['name']}` ({rule_imports} imported symbols)")
            pdb_paths = rules["pe"]["pdb_paths"]
            if pdb_paths:
                lines.append(f"- Rules PDB path: `{pdb_paths[0]}`")
        bindings = mission.get("rule_bindings")
        if bindings:
            lines.append(
                f"- Rule-to-script bindings: {bindings['matched_exports']}/{bindings['imported_symbols']} resolved"
            )
            lines.append(f"- Rule-bound actions: {_markdown_list(bindings['actions'])}")
        coverage = mission.get("rule_topology_coverage")
        if coverage:
            lines.append(
                "- Rule topology action coverage: "
                f"{coverage['recovered_actions']}/{coverage['expected_actions']}"
            )
        action_calls = sorted(
            {
                str(call.get("name", f"slot_{call['slot']}"))
                for action in mission.get("action_topology", [])
                for call in action["script_interface_calls"]
            },
            key=str.casefold,
        )
        if action_calls:
            lines.append(
                f"- Action-side ScriptInterface calls: {_markdown_list(action_calls)}"
            )
        bootstrap_helpers = sum(
            len(entry.get("helper_rvas", []))
            for entry in mission.get("bootstrap_topology", [])
        )
        bootstrap_state_operations = sum(
            len(callback.get("state_sequence", []))
            for entry in mission.get("bootstrap_topology", [])
            for callback in entry.get("callbacks", [])
        )
        if bootstrap_helpers:
            lines.append(f"- Bootstrap helpers followed: {bootstrap_helpers}")
            lines.append(
                f"- Bootstrap state operations: {bootstrap_state_operations}"
            )

        topology = mission.get("rule_topology", [])
        if topology:
            lines.extend(
                [
                    "",
                    "Recovered rule topology:",
                    "",
                    "| Rule | Condition symbols | Proven condition IR | ScriptInterface slots | Action |",
                    "|---|---|---|---|---|",
                ]
            )
            for rule in topology:
                condition_symbols = [
                    symbol for symbol in rule["condition_symbols"] if symbol != "g_pSI"
                ]
                conditions = "<br>".join(f"`{value}`" for value in condition_symbols) or "—"
                slots = "<br>".join(
                    f"`#{call['slot']} {call.get('name', '')}`".rstrip()
                    for call in rule["script_interface_calls"]
                ) or "—"
                actions = "<br>".join(f"`{value}`" for value in rule["actions"]) or "—"
                condition_ir = rule.get("condition_ir")
                ir_text = (
                    f"`{condition_ir['kind']}({condition_ir['symbol']})`"
                    if condition_ir
                    else "—"
                )
                lines.append(
                    f"| `{rule['name']}` | {conditions} | {ir_text} | {slots} | {actions} |"
                )

        combined_references: dict[str, set[str]] = defaultdict(set)
        for file_data in (script, rules):
            if not file_data:
                continue
            for extension, values in file_data["references"].items():
                combined_references[extension].update(values)
        if combined_references:
            lines.extend(["", "Asset/file references:", ""])
            for extension in sorted(combined_references):
                lines.append(
                    f"- `.{extension}`: {_markdown_list(sorted(combined_references[extension], key=str.casefold))}"
                )
        lines.append("")
    return "\n".join(lines).rstrip() + "\n"


def write_json(path: Path, manifest: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(json.dumps(manifest, indent=2, sort_keys=True) + "\n", encoding="utf-8")


def write_markdown(path: Path, manifest: dict[str, object]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    path.write_text(render_markdown(manifest), encoding="utf-8")


def build_parser() -> argparse.ArgumentParser:
    parser = argparse.ArgumentParser(
        description="Inspect Armada II S.dsl/R.drl mission DLLs without executing them."
    )
    parser.add_argument("mission_dir", type=Path, help="directory containing Armada II mission pairs")
    parser.add_argument(
        "--output-dir",
        type=Path,
        help="write sta2-missions.json and sta2-missions.md here; otherwise print Markdown",
    )
    parser.add_argument(
        "--engine-exe",
        type=Path,
        help="matching Armada2.exe used to resolve ScriptInterface vtable slots",
    )
    parser.add_argument(
        "--engine-map",
        type=Path,
        help="matching armada2.map used to name ScriptInterface vtable slots",
    )
    parser.add_argument(
        "--include-strings",
        action="store_true",
        help="include every printable ASCII string in JSON (can make the report very large)",
    )
    parser.add_argument(
        "--include-non-a2",
        action="store_true",
        help=(
            "analyse every S.dsl/R.drl pair, including Armada I missions; "
            "A1 uses a different rule/action ABI and may have reduced semantic recovery"
        ),
    )
    parser.add_argument(
        "--no-demangle",
        action="store_true",
        help="do not use llvm-undname even if it is installed",
    )
    return parser


def main(argv: Sequence[str] | None = None) -> int:
    args = build_parser().parse_args(argv)
    try:
        if bool(args.engine_exe) != bool(args.engine_map):
            raise ValueError("--engine-exe and --engine-map must be supplied together")
        script_interface = (
            extract_script_interface(
                args.engine_exe,
                args.engine_map,
                demangle=not args.no_demangle,
            )
            if args.engine_exe
            else None
        )
        manifest = analyze_directory(
            args.mission_dir,
            demangle=not args.no_demangle,
            include_strings=args.include_strings,
            script_interface=script_interface,
            include_non_a2=args.include_non_a2,
        )
    except (OSError, PeFormatError, ValueError) as error:
        print(f"error: {error}", file=sys.stderr)
        return 2

    if args.output_dir:
        json_path = args.output_dir / "sta2-missions.json"
        markdown_path = args.output_dir / "sta2-missions.md"
        write_json(json_path, manifest)
        write_markdown(markdown_path, manifest)
        summary = manifest["summary"]
        print(
            f"Analysed {summary['missions']} missions ({summary['complete_pairs']} complete pairs).\n"
            f"JSON: {json_path}\nMarkdown: {markdown_path}"
        )
    else:
        print(render_markdown(manifest), end="")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
