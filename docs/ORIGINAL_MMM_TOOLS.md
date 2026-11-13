# Original mission analysis and MMM conversion tools

These are the original Python tools from the July 2026 Armada mission-porting
workflow. The two scripts and their existing synthetic tests are preserved
without source changes from the earlier `sta_viewer` project.

| Tool | Purpose |
|---|---|
| `tools/analyze_sta2_missions.py` | Read compiled DSL/DRL pairs as data and produce a structural manifest. `--include-non-a2` includes Armada I, with reduced semantic recovery. |
| `tools/convert_sta2_missions_to_mmm.py` | Read that manifest and generate Armada II Megadroid Mission Mod Lua scaffolds. Unresolved conditions remain disabled; `--pilot` enables only the UI tutorial pilot. |

The A1 workflow uses the analyzer's non-A2 mode as a worksheet for manual Lua
implementation. This import does not add a separate automatic A1-to-MMM
generator or establish complete campaign fidelity.

These tools target the earlier MMM workflow. STA64's `a1_to_lua.py`,
`dsl_to_lua.py` and `drl_to_lua.py` are separate converters targeting STA64's
mission API. Importing the original tools does not change the runtime or its
restricted Lua environment (modding decisions D2 and D13).

## Usage

Run from the repository root with Python 3.10 or newer. Supply your own game
install, and keep generated manifests, research reports and Lua outside the
repository and installed game directories. The tools read the mission DLLs;
they do not load or execute them.

Analyze A2 missions, optionally resolving engine method names with a matching
executable and linker map:

```sh
python3 tools/analyze_sta2_missions.py /path/to/Armada2/missions \
  --engine-exe /path/to/Armada2/Armada2.exe \
  --engine-map /path/to/Armada2/armada2.map \
  --output-dir /path/to/private/a2-analysis
```

The two engine options are optional and must be supplied together.

Analyze A1 missions:

```sh
python3 tools/analyze_sta2_missions.py /path/to/Armada1/missions \
  --include-non-a2 --output-dir /path/to/private/a1-analysis
```

Generate A2 MMM review scaffolds:

```sh
python3 tools/convert_sta2_missions_to_mmm.py \
  /path/to/private/a2-analysis/sta2-missions.json \
  --bzn-dir /path/to/Armada2/bzn \
  --output-dir /path/to/private/a2-mmm-review
```

`--mission a2_fed01` limits generation to one mission. Existing output files
are protected unless `--force` is supplied. See each script's `--help` for
additional options. Optional `llvm-undname` improves demangled names; otherwise
the tools use Python's standard library.

## Synthetic checks

The preserved tests use authored fixtures and require no game install:

```sh
PYTHONDONTWRITEBYTECODE=1 python3 -m unittest \
  tools.test_analyze_sta2_missions tools.test_convert_sta2_missions_to_mmm
```
