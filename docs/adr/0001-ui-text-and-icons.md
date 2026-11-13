# ADR 0001: UI text, markup and icon formats

- **Status:** proposed
- **Date:** 2026-11-13
- **Deciders:** Tammy and Cedric (accepted when both approve the PR that
  merges this file; not binding before)
- **Relates to:** [MODDING_DECISIONS.md](../MODDING_DECISIONS.md) D1, D3 and
  its "Not in v1" list; WORKPLAN UI-03, UI-12, UI-16, UI-27

## Context

STA64 draws two kinds of text and has two ways of drawing it.

- Game and mod text is drawn with the mod's own SPR fonts, resolved through
  the mod chain; each font's byte → glyph table is read, not assumed (UI-12,
  UI-16, `sta64/src/ui/sprite_font.rs`).
- Roots writes its tooltips in its fonts' own codes: figures, resource symbols
  and unit classes are single bytes the font draws as pictures (for example
  0x85 for dilithium). Evidence: measured on Roots with the Refit bench. Mods
  rely on this, so the sprite fonts cannot be dropped.
- STA64's own screens have a baked SDF font (Noto Sans Condensed, OFL, made by
  `sta_fontbake`). The loader and layout exist; the SDF material that draws
  the quads does not, so widgets still use Bevy's text. Refit's icon atlas
  (SDF primitives written in code) is not ported.
- No markup exists, and no decision covers fonts, icons or markup.

The forces:

- Sprite fonts are single-byte and made for one pixel size. They cannot show
  player names or translations outside their table and do not stay sharp at
  large UI scales.
- Supporting two font systems must not mean writing layout, wrapping and every
  widget twice.
- An adapted mod must still run on its original engine (D3), and string-table
  overlays are excluded from modding v1.
- Giving mods modern formats beside the legacy ones is a goal of the project.
- Game assets and franchise symbols are never committed or released
  (AGENTS.md, "Research and game files").

## Decision

### Fonts

**T1. Two font systems, split by whose text it is.**
Text that comes from a game or a mod (tooltips, unit names, HUD and stock menu
labels) is drawn with that mod's sprite fonts, as it is on the original. Text
of STA64's own screens (options, lobby, tools, notices we write) is drawn with
the baked SDF font. No third system, and no setting that swaps one for the
other in v1.

**T2. One text-run model under both.**
A line of text is a list of spans: text in a font with a colour, an inline
picture, a tab or a line break. Measuring, wrapping, aligning and drawing are
written once, on spans. The glyph source is an interface behind that model,
with the sprite fonts and the SDF font as its first two implementations.

**T3. Legacy strings parse into spans through the mod's glyph table.**
A byte the font draws as a letter becomes text; a byte it draws as a picture
(resource symbols, coloured figures, class words) becomes an inline picture
span that is not tinted. The table and the names of those codes come from the
mod's data and UI-16's glyph → word mapping, never from a list in code.

**T4. A character the sprite font lacks falls back to the SDF font.**
When a string holds a character outside the sprite font's table (a player
name, a translated string), that span is drawn with the SDF font at the same
line height, not as `?`.

### Markup

**T5. Markup is read only in strings STA64 owns.**
That is: STA64's own UI strings, and text passed by mission Lua through a call
that asks for markup. The legacy string tables
(`Dynamic_Localized_Strings.h`, ODF tooltip keys) are never parsed for it.

**T6. The syntax is a small set of bracket tags.**

| Tag | Meaning |
|---|---|
| `[b]…[/b]` | bold face |
| `[color=<role or #rrggbb>]…[/color]` | a theme colour role (`good`, `warn`, `bad`, `accent`, `text_dim`) or a literal colour |
| `[icon=<name>]` | an inline picture (T8) |
| `[br]` | line break |
| `[[` | a literal `[` |

An unknown or unclosed tag is drawn as written and logged once.

### Icons

**T7. Two kinds of icon, two formats.**
- *Flat, single-colour icons of our own design* (command, status and
  navigation symbols of our own screens): authored as SVG in the repository,
  baked at build time by `sta_fontbake` into an SDF atlas, tinted by the
  theme.
- *The games' icons* (build buttons, resource icons, portraits, race
  emblems): stay raster, read from the install's SPR tables and textures
  through the mod chain (UI-27). A mod replaces them the way it replaces any
  texture, at whatever resolution it ships.

SVG is a source format only; STA64 does not load or rasterise SVG at run time
in v1. Race emblems and every other franchise symbol are never committed or
released, in any form: a redrawn SVG of an emblem is still the rights
holder's mark. Where our own screens show a race, the emblem comes from the
player's install at run time, with the race's name as the fallback.

**T8. `[icon=<name>]` resolves through data.**
First the name of a picture glyph of the active mod's font (the words of
UI-16's glyph mapping, so `[icon=dilithium]` draws the same symbol a Roots
tooltip does), then the name of an icon in STA64's baked atlas. A name that
resolves to neither is drawn as the word.

## Alternatives considered

- **Sprite fonts only.** Keeps one system and full fidelity, but leaves
  STA64's own screens without Unicode, kerning or sharp text at large UI
  scales. Rejected.
- **A real font everywhere, sprite fonts converted or dropped.** Breaks every
  tooltip written in a font's picture codes and makes mods look unlike
  themselves (against D1). Rejected.
- **Markdown as the markup.** It has no colour and no inline picture, its
  `*`, `_` and `#` occur in ordinary strings, and its block structure
  (headings, lists, links) is not what a tooltip or notice needs. Rejected for
  game text.
- **Markup inside the legacy string tables.** Tags would show as literal text
  on the original engines (against D3), and existing strings containing
  brackets would change meaning. Rejected.
- **SVG loaded at run time.** Adds a rasteriser dependency for what a
  build-time bake already gives our own icons. Postponed to the mod-supplied
  case below, not rejected.
- **Vectorising the games' painted icons.** They are painted art that does
  not survive tracing, and the result would still be game content. Rejected.

## Consequences

**Gained**

- Mods look as they do on the original, glyph codes included, while STA64's
  own screens get a scalable Unicode font.
- Layout, wrapping and widgets are written once (T2).
- New strings can carry colour and inline symbols without inventing font
  codes.
- The repository holds only icons of our own design.

**Costs and limits**

- Two glyph sources to maintain, and the SDF text material still has to be
  built before T1 is true for our own screens.
- A mod cannot put markup in its tooltips in v1: that needs a STA64-only
  strings file beside the legacy one, which is a string-table overlay and on
  MODDING_DECISIONS' "Not in v1" list. Mod tooltips keep the glyph codes they
  use today, which T3 draws.
- Sharpness of sprite text at large UI scales is not promised (open
  question 1).
- Our own screens depend on the install for race emblems and fall back to
  names without one.

**Not in v1**

Markup in legacy string tables or in mod tooltips; a STA64-only strings file
for mods; Markdown; tags for font size, links or images by path; loading SVG
at run time; vectorising or redrawing the games' icons and emblems;
converting a mod's sprite fonts or icons to another format.

**Planned after v1**

Postponed for order of work only; v1 must not be built in a way that blocks
them.

- **Fonts supplied by a mod as TTF/OTF.** A mod names a font file in a
  STA64-only key that the original engines ignore (D3) and keeps its sprite
  fonts for them. STA64 bakes the face into an SDF atlas on the player's
  machine and uses it as the glyph source for that mod's text (T2), with the
  mod's picture glyphs still drawn from its sprite font (T3). What v1 owes
  this: the glyph source interface of T2, and `sta_fontbake` usable as a
  library, not only as a build tool.
- **Markup in mod strings**, once a STA64-only strings file exists (open
  question 2).
- **Vector icons supplied by a mod**, baked the same way as a mod's font.

**Work packages**

Rows are added to `docs/WORKPLAN.md` with the next free UI numbers once this
ADR is accepted; none is claimed before that.

| Delivers | Decision | Builds on |
|---|---|---|
| SDF text material: the baked font's quads drawn, widgets moved off Bevy's text | T1 | UI-03 |
| Text-run model: spans, measure, wrap and draw over both glyph sources; legacy string parser with picture glyphs; SDF fallback | T2, T3, T4 | UI-16 |
| Markup parser and the Lua text call that opts in | T5, T6, T8 | the two above, LUA-09 |
| Icon bake from SVG sources into the SDF atlas; first icon set | T7 | UI-03 |

Done when: a Roots tooltip, a string with a character outside the sprite
font and a marked-up notice with a colour and `[icon=dilithium]` each lay out
through the same code path in a headless test; the widget gallery draws its
labels and icons from the SDF atlases at UI scales 1.0, 1.25 and 2.0.

## Open questions

1. **Sprite fonts at large UI scales.** How the originals scale their fonts
   with resolution is not measured. Until it is, sprite text is scaled as the
   stock HUD is (UI-18).
2. **Lifting the string-table overlay exclusion.** Markup in mod strings
   needs a STA64-only strings file beside the legacy one. If that is wanted
   for the first release, it is a change to MODDING_DECISIONS' "Not in v1"
   list and goes through its own PR.

## Changing this ADR

Once accepted, the decisions change only through a new ADR that supersedes
this one (or the affected T numbers), approved by both Tammy and Cedric. This
file is then marked "Superseded by ADR NNNN" and kept.
