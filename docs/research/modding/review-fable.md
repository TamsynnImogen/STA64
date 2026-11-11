# Second-round review of the modding plan (2026-11-11)

Scope: `docs/MODDING-IDEAS.md` with `review-opus.md` applied, plus the two
owner decisions (shader tier first-class; Refit as first-party reference
pack). Read-only: installs under `/home/cedric/Games/Heroic/` inspected with
`find`/`grep`/`md5sum`/`diff`/`strings`, code by reading; nothing built or
run. FPQ contents are compressed, so statements about ODFs inside `ODF.fpq`
are from name tables only.

Verdict: the first review fixed the model (two axes, closed rules, agreement
between peers) and is mostly right. Both documents still design the
*container* in detail while the stated goal, "existing mods keep working", is
decided elsewhere: by how many ODF/cfg keys the engine understands (104 of
893 FO keys today), by whether an adapted mod must still run on the original
engine, and by files that are not ODFs. v1 should be much smaller than either
document proposes and should be measured against one third-party mod.

## 1. The first review, judged

| Finding | Verdict | Evidence |
|---|---|---|
| F1 FO 4.0 / STA2 Classic are `standalone=1`, only Classic Enhanced has `ParentMod` | Right | `Roots/Data/Mods/*/info.ini`: both `standalone=1`, `ignoreParentMaps=1`; Enhanced `ParentMod="STA2 Classic Mod"`, `standalone=0`. Base `Data/info.ini` is itself `standalone=1`. |
| F2 `.tt`/`.xml` follows the data pack | Right, and worse than stated | Base 10 `.tt` + 24 `.xml`, FO 4.0 0 + 3, Classic 37 + 0, Enhanced 8 + 1. FO 4.0's `MonitorTutorial.bzn` names `tech1.tt` although FO 4.0 ships only `tech1.xml`: references are by stem, so the lookup must ignore the written extension. |
| F5 FO 4.0 ships own cfg and "engine shader programs" | Half right | `RTS_CFG.h` (94 differing lines) and `ART_CFG.h` (70) differ. The 4 `Shaders/*.nvv` are md5-identical to base: redundant copies, not per-pack shaders. FO 4.0 re-ships about 3,487 of its 5,035 files at base paths. |
| F11 / C1 "the sim reads values the hash does not govern" | Fact right, consequence overstated | 6 call sites, 7 keys, one file (`sta_sim/src/specials/mod.rs:78-114`). Every one has a typed, hashed twin parsed from the same key (`def.rs:277`, hash blocks `0xe1`/`0xe2`). No desync hole exists today. It is a hygiene and speed problem: the lookups lower-case a string and parse text per unit per tick (`specials/mod.rs:322-347`). Still ban `.raw` in `sta_sim`; it is a one-day fix, not an architectural finding. |
| C1 "`LoadOrder::identity()` is most of it already" | Wrong for the target corpus | `order.rs:70-83` hashes id, version text and `.sta` hashes. A loose folder contributes id and version only (module doc says so), and every existing mod is loose files plus FPQ. Editing an ODF in Classic Enhanced does not change identity. It also has no non-test caller. |
| C1 lock carries gameplay-script hashes and per-layer byte hashes | Over-specified | Under D3 scripts and AI run on the host and only commands enter the log; peers and replays never run them. See finding 3. |
| C2 two axes (engine lineage, data chain) | Right on substance, wrong key | `manifest.toml` already has `engine`: a semver requirement on STA64 itself (`manifest.rs`, `STA_ARCHIVE.md:159`). `engine = "roots5"` fails with `BadRequirement`. The manifest also already has `profile` (parsed, unused) and `kind = addon|standalone`. See finding 4. |
| repo-seams "~50 `mount_install` call sites" (basis for MOD-02's wrapper and freeze worry) | Wrong | 5 non-test call sites (`lib.rs:34`, `audio/mod.rs:85`, `bench/mod.rs:203`, `game/flow.rs:365`, `frontend/scene.rs:1459`); the rest are tests and examples. |
| H6 / repo-seams "recipes are `include_str!`, make them VFS-overridable" | Mostly already done | Nebula (`render/nebula.rs:41-46`) and surfaces (`render/surfaces.rs:57-63`) read `nebulae/<key>.ini` / `surfaces/<key>.ini` from the VFS first. Only ordnance (`fx/recipe.rs:163-166`) is embedded-only. The Refit-modded A2 install already carries `Surfaces/` and `Nebulae/` (9 ini each) that STA64 reads as-is. |
| H5 "audit the Lua sandbox" | Right, understated | `sta_mission/src/runtime.rs:254` and `script.rs:112` use `Lua::new()`; no `set_memory_limit`, hook or interrupt; no global removed. Per mlua's documentation `Lua::new()` loads the memory-safe standard set, which includes `io`, `os` and `package` (confirm against mlua 0.12). If so, the 82 installed mission scripts, and any downloaded map with a script, can already touch files. This is a present problem, not a MOD-11 precondition. |
| H1 closed registry, H2 no oracle, H3 coverage not tiers, H4 layer order before extension order, M4 hot reload dev-only | Right | Keep. H3 is the most valuable recommendation in the review (finding 1 below sharpens it). |
| M1 "STA64 owns its formats, Refit is a consumer" | Late | The recipe ini format is already shared in code (row above). Freeze it as it is; do not fork it. |
| Revised MVP includes FO 3.2.7 loading "on the right engine" | Over-scoped | There is no FO 3 lineage in `Profile` (`Native, Fo4, Sta2Classic, Sta1Classic`). Adding one is sim and content work across lanes, not pack work. |

Also noted: repo-seams says `info.ini` `[modules]` lists are "Roots hook module
lists"; no installed mod has a `[modules]` section.

## 2. Findings both documents missed (ranked)

### 1. Critical: compatibility is key coverage, and nothing in the plan moves it

Evidence: `docs/data/README.md`: STA64 reads 104 of 893 distinct FO ODF keys,
107 of 970 for A2, 75 of 572 for A1; 2 of 474 cfg keys. `RTS_CFG.h` has 195
tunables (112 float, 83 int); the engine consumes about 12, all camera, audio
and star rendering, none in `sta_sim` or `sta_content`, none hashed.
STA2 Classic Enhanced differs from its parent by 8 changed lines of
`RTS_CFG.h`, 8 lines of `tech1.tt`, 178 ODFs and 17 maps: today STA64 cannot
tell the two mods' rules apart, because it reads none of those 8 cfg lines.

Consequence: a perfect pack layer would not make one more existing mod play
correctly. The plan spends 13 packages on the container and none on the
measure.

Recommendation: the first deliverable is the coverage report (H3), run over
the installed corpus and over maps, with numbers per mod: keys read / keys
present weighted by use, cfg keys the mod changes that the sim ignores, ODFs
referenced by maps that do not resolve. Prioritise SIM/CON key work from that
report. The quirk IDs proposed in fo-hooks-asi §5.2 are mostly not quirks: a
Roots sentinel (`avoidanceclass = 151`, `hitChanceMod = 3`) is how that
lineage's reader fills a canonical field (`fires_while_cloaked`), done once
at load in `sta_content`. Reserve named, documented, tested rule switches for
behaviour a pack may legitimately choose (queue policy, cloak rules); expect
tens, not hundreds.

### 2. Critical: nobody asked whether an adapted mod must still run on the original engine

Mod authors (the Roots author is in contact) will not maintain two trees. If
"minimal adaptation" produces a folder the original engine can no longer
load, the adaptation will not be made. This constrains every format choice:
`manifest.toml` beside `info.ini` is harmless; a `[patch]` fragment replacing
a whole-file ODF is STA64-only; a self-including ODF may or may not load on
stock A2 or Roots (untested); extra `info.ini` sections and `STA64_*` ODF
keys are presumably ignored (untested).

Recommendation: decide it now (owner decision 1). If yes: legacy `info.ini`
stays a first-class manifest (it already carries `[Mod] Profile=` and
`[Features]`, `profile.rs:365-404`), every STA64 addition lives in files or
keys the old engine ignores, and two bench checks on the original engines
settle "unknown `info.ini` section" and "unknown ODF key". The "tiny ini" the
goal asks for then already exists; do not introduce a second mandatory file.

### 3. High: the lock should gate on resolved rules, not on the pack set

The first review's session manifest compares ordered pack lists with
per-layer byte hashes. That is both too strict and too expensive: a peer with
a texture pack, a Refit recipe folder or a different menu scene simulates the
same game and would be refused; and hashing 2.4 GB of loose Dominion War
files at every lobby join is not acceptable. Refit already draws this line:
`layers.toml` marks rule layers `scope = 'match'`.

Recommendation, exactly what is in the lock:
- Must match: `SEAM_REV` / build protocol id; lineage id; one digest of the
  resolved sim-visible content (all defs, per-def rule values, tunables the
  sim reads, tech tree, resource model, map). Values, not names: per-ODF
  `Feature_*` overrides mean a preset name does not determine the rules.
- Informational, carried in lobby, replay and save headers: pack ids and
  versions in order, so a replay can find its content and a mismatch can be
  explained. On mismatch only, compute a per-def digest list and show the
  first differing ODF/key; per-file hashes are never needed.
- Not in the lock: presentation packs, shaders, host-only scripts.

Fix the digest before relying on it: derive it from one serialisation
(first review, agreed); today the tech tree is folded through
`format!("{entries:?}{caps:?}")` (`db.rs` near 546), which changes with any
`Debug` derive, and `ResourceModel` is not folded at all.

### 4. High: the axes exist already; one is misnamed, one is unimplemented

- Lineage: use the existing `profile` key (manifest and `info.ini`), meaning
  the closed, code-defined engine lineage. Keep `engine` as the STA64 semver
  requirement. Do not add a third key. Rename the enum variants by lineage as
  the first review says (`Sta2Classic` means stock A2 while "STA2 Classic" is
  a Roots data root).
- Data chain: `kind = standalone` / `ParentMod` exist as data, but
  `Vfs::mount_install` ignores them: `mod_chain` (`lib.rs:327-344`) always
  appends base `Data`, and `ModInfo.standalone` / `ignore_parent_maps` are
  parsed and never read. So FO 4.0 is mounted over Roots base today; since
  ODFs resolve by bare name across the whole chain (`index.rs:1-31`), base
  Roots ODFs, 21 extra tech-tree xml and 192 base maps leak into FO 4.0. This
  is a correctness bug independent of the RFC.
- These two axes are sufficient. "Profile as a rules pack" should go: a
  lineage is code; a data root supplies numbers through the files it already
  has (`RTS_CFG.h`, ODFs). No separate `rules` layer format is needed for
  legacy content.

### 5. High: the conflict problem is not in ODFs

Evidence: stock A2 has 707 `#include` lines, none a self-include; Classic
Enhanced 139, none; Dominion War's 623 addon ODFs none. ODF references are
always bare names (`weapon1 = "ganti_cloak"`), and 167 of Enhanced's 178 ODFs
are whole-file overrides of small files. Whole-file ODF override is the
community's working idiom and clobbers little. The files every mod must
copy in full are the monoliths: `tech1.tt` (484 lines, 8 changed),
`RTS_CFG.h` (553 lines, 8 changed), `Dynamic_Localized_Strings.h` (2,444
lines stock, 61,091 in FO 4.0), `gui_global.spr` (2,389), `events.dat`
(3,651), `Input.map` (3,774), `races.odf`.

Recommendation: drop the TOML `[patch]` format for ODFs. One rule for all
line-oriented text files instead: *a file that includes its own name inherits
the same-named file from the layer below*. `sta_odf::resolve_layered` already
implements it for ODF; `.h` files already use `#include`; `.tt` and the
string table need a keyed merge (first column / string key). Patches stay in
the syntax the modder already writes, and values pass through the one
text-to-`Fixed` path (finding 7). Provenance (`sta64 why`) must cover these
files, not only ODFs. Do `.h` and `.tt` first: they are where the installed
corpus copies whole files for single-digit line changes.

### 6. High: classic mods and maps have no folder boundary

Dominion War is a full install with files overwritten in place (8,738 files
vs 5,430 in FO; no `Mods/`, no `info.ini`); A1 and A2 mods use `addon/` or
overwrite. `Mods/` with `info.ini` exists only on Roots. Maps are the most
numerous community content (206 `.bzn` in Dominion War, 192 in Roots base)
and name ODFs and the tech tree by bare name, so a map is only valid against
one data root.

Recommendation: "T0" for pre-Roots mods is *point STA64 at the modded
install as a root*; say so, and support several registered roots. An
extract-over-install zip unpacked into a mod folder mounts as a layer because
it mirrors the install layout. Maps get first-class treatment in the coverage
report (unresolved ODF names per map). Multi-addon stacking, version solving
and a load-order UI have no instance in the corpus (one two-deep chain); the
code exists in `order.rs`, leave the UI out.

### 7. Medium: number parsing must have one path

ODF numbers parse as `f32`, widen to `f64`, then `Fixed::from_f64` (Q32.32)
(`sta_odf/src/lib.rs:138`, `sta_content/src/odf.rs:141-157`). That is
deterministic across platforms. But the same text gives a different `Fixed`
if another source parses it as `f64` (TOML does): `0.1` in an ODF and `0.1`
in a TOML patch would not be the same rule value. Rule: every rule value
enters through one `text -> Fixed` function; pack values are kept as text
until then. Also convert at load, never per tick, and enforce the float rule
with a lint (it is doc-only today; no `clippy.toml` float restriction).

### 8. Medium: a keyed quirk map is the wrong shape for the sim

MOD-03 proposes `BTreeMap<QuirkId, QuirkValue>` and "profile as data" as an L
seam PR with a freeze. The sim has 3 profile branches (`kinematics.rs:88`,
`boundary/state.rs:133,145`); content has about 8. A typed `Rules` struct
(enums, `Fixed`, integers; serde names double as ids; digest derived; docs
generated) gives the same result with compile-time checking and no string
lookups in `step`. Since the rule set is closed, nothing is lost. Do it
additively, one field per branch removed, without a freeze.

### 9. Medium: ordering risk

A new MOD lane with seam freezes competes with INT-M3..M5 for the same seam
files (`db.rs`, `features.rs`, replay format) while NET-01 is unwritten and
`sta_net` is a stub. Every replay/snapshot format bump and golden re-record
costs all lanes. Batch: one format bump (pack names in headers, derived
digest) scheduled with NET-01, nothing else from this plan touching seams
before INT-M3.

### 10. Low

- `.drl`/`.dsl` mission DLLs (36 each in A2 and STA2 Classic) are never
  loaded: stock campaigns are outside "runs as-is". State it in the matrix.
- Case-only path differences across layers are common (109 between STA2
  Classic and Roots base); the VFS normalises, keep a test on real data.
- Stock A2 has one duplicate ODF basename (`ep_stand.odf` in `SOD/` and
  `odf/other/`); the index's warning path is exercised by stock data.

## 3. Push-back on owner decisions

**Shader tier first-class.** The sequencing is right and step one is nearly
shipped (only ordnance recipes are not yet VFS-first). Push-back on the rest
of the ladder for now: the people who will write WESL against STA64 in the
next year are, realistically, the owner. A versioned input contract, shims,
`new-effect`, auto-sliders and perception-filtered input structs are product
work for an audience of one. The cheapest form that honours the decision: a
developer option that lets a VFS file shadow any embedded `.wesl` by path,
labelled unstable, no contract. Promote to a real slot when a second author
asks. On fairness: in an open-source lockstep engine every client holds full
state and can be rebuilt, so filtered shader inputs are not a security
boundary; draw from the perception-filtered view because that is what the
renderer should consume anyway, and do not design further around it.

**Refit as the reference pack.** Right for the render tier; wrong as the
reference for the primary goal. Refit is render, UI and QOL written by the
engine's author: it cannot show whether a third party's data mod works with
minimal adaptation. Use two references: Refit for recipes and shaders, and
STA2 Classic Enhanced (215 files, a parent, changed `RTS_CFG.h`, tech trees,
ODFs, maps, no binaries) as the acceptance pack for compatibility. Have its
author try the result.

## Recommended v1

Six pieces, none larger than M, one format bump.

1. **Mount fix** (S): `standalone` and `ignoreParentMaps` honoured; lineage
   taken from manifest `profile`, then `info.ini`, then heuristics; one entry
   point behind the 5 call sites; several install roots.
2. **Coverage report and `why`** (M): `sta64 check-pack <root|mod>`: key
   coverage weighted by use, changed-but-ignored cfg keys, unresolved map
   references, which layer and line won a file or key. Runs over the corpus
   in an install-gated job.
3. **Rules and digest** (M, with NET-01): typed `Rules` struct fed from
   `RTS_CFG.h`, starting with the keys Classic Enhanced changes; remove the 6
   raw reads; derived digest incl. tech tree and resource model; replay and
   snapshot headers gain lineage and pack names (single format bump).
4. **Self-include inheritance for `.h` and `.tt`** (M): finding 5, with
   provenance.
5. **Render** (S): ordnance recipes VFS-first; dev-only `.wesl` shadowing.
6. **Lua restriction** (S): explicit std-lib set, memory and instruction
   limits. Do this regardless of the RFC.

Done when: Classic Enhanced loads over its parent and FO 4.0 loads without
base Roots leaking in; the report prints coverage numbers for every installed
mod; two in-process peers on Classic and Classic Enhanced refuse to start and
name the differing key, while a peer that differs only by a texture or recipe
folder is accepted; a replay names its packs; a recipe ini dropped in a mod
folder changes a headless frame.

Non-goals for v1: exe fingerprinting (data fingerprints suffice), FO 3.2.7 /
Dominion War / A1 lineages, bug-emulation quirks, TOML `[patch]` for ODF,
trigger/timeline DSL, third-party pack Lua, WASM, shader contracts and
shims, pack UI and load-order editor, dependency fetching, signing policy,
string-table overlay, hot reload beyond what the dev shadowing gives,
`.asi`/hook importer, per-match rule toggles.

## Owner decisions

Decide now:
1. **Dual-run:** must a mod adapted for STA64 still load on its original
   engine? (Recommended: yes; fixes the manifest and patch choices.)
2. **Axes:** `profile` = closed lineage, `engine` stays the STA64 version
   requirement, data chain from `kind`/`ParentMod`; which lineages v1 covers
   (recommended: stock A2 and Roots only).
3. **What must match:** resolved-rules digest only, pack list informational
   (recommended), or exact pack-set equality.
4. **Rules shape and carrier:** typed struct fed by the legacy files
   (`RTS_CFG.h`, ODF keys) versus a keyed quirk map and a new rules file.
5. **Acceptance target:** Classic Enhanced as the compatibility reference
   beside Refit as the render reference, and the "done when" above.

Can wait: script tiers and Lua versus anything else for packs; shader slot
contract and its versioning; pack UI and load order; distribution, signing
and trust policy; FO 3 and A1 lineages; localisation overlay; hot reload
policy; whether Refit's recipe format ever diverges from STA64's.
