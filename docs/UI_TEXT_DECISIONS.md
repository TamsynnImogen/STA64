# UI text and icons: decisions for sign-off

Status: **proposed** (2026-11-13, from the font and icon discussion between
Tammy and Cedric). Not binding until both approve the PR that adds this file.
Covers which fonts draw which text, how text and pictures mix in one line,
the markup for new strings and the formats of icons. It builds on UI-03
(baked SDF font), UI-12 and UI-16 (the games' sprite fonts) and UI-27 (GUI
sprite artwork), and stays inside [MODDING_DECISIONS.md](MODDING_DECISIONS.md)
(D1, D3 and its "Not in v1" list).

## Where things stand

- Game and mod text is drawn with the mod's own SPR fonts, resolved through
  the mod chain; each font's byte → glyph table is read, not assumed (UI-12,
  UI-16, `sta64/src/ui/sprite_font.rs`).
- Roots writes its tooltips in its fonts' own codes: figures, resource symbols
  and unit classes are single bytes the font draws as pictures (for example
  0x85 for dilithium). Evidence: measured on Roots with the Refit bench.
- STA64's own screens have a baked SDF font (Noto Sans Condensed, OFL, made by
  `sta_fontbake`). The loader and layout exist; the SDF material that draws
  the quads does not, so widgets still use Bevy's text. Refit's icon atlas
  (SDF primitives written in code) is not ported.
- No markup exists. No decision covers fonts, icons or markup today.

## Fonts

**T1. Two font systems, split by whose text it is.**
Text that comes from a game or a mod (tooltips, unit names, HUD and stock menu
labels) is drawn with that mod's sprite fonts, as it is on the original. Text
of STA64's own screens (options, lobby, tools, notices we write) is drawn with
the baked SDF font. No third system, and no setting that swaps one for the
other in v1.

**T2. One text-run model under both.**
A line of text is a list of spans: text in a font with a colour, an inline
picture, a tab or a line break. Measuring, wrapping, aligning and drawing are
written once, on spans. Sprite fonts and the SDF font are two glyph sources
behind it. This is what keeps "two fonts" from becoming every widget
implemented twice. The glyph source is an interface, not a pair of cases: a
font a mod supplies as TTF/OTF (see "Planned after v1") is a third source
behind the same model, with no change to layout or widgets.

**T3. Legacy strings parse into spans through the mod's glyph table.**
A byte the font draws as a letter becomes text; a byte it draws as a picture
(resource symbols, coloured figures, class words) becomes an inline picture
span that is not tinted. The table and the names of those codes come from the
mod's data and UI-16's glyph → word mapping, never from a list in code.
Legacy strings are never read as markup (T5).

**T4. A character the sprite font lacks falls back to the SDF font.**
Sprite fonts are single-byte. When a string holds a character outside the
font's table (a player name, a translated string), that span is drawn with
the SDF font at the same line height, not as `?`.

## Markup

**T5. Markup is read only in strings STA64 owns.**
That is: STA64's own UI strings, and text passed by mission Lua through a call
that asks for markup. The legacy string tables
(`Dynamic_Localized_Strings.h`, ODF tooltip keys) are never parsed for it, for
two reasons: an adapted mod must still display correctly on its original
engine (D3), where tags would show as literal text, and existing strings that
contain brackets must not change meaning.

Consequence: a mod cannot put markup in its tooltips in v1, because that
needs a STA64-only strings file beside the legacy one, which is a
string-table overlay and on the "Not in v1" list. This proposal leaves that
list unchanged; mod tooltips keep the glyph codes they use today, which T3
already draws.

**T6. The syntax is a small set of bracket tags, not Markdown.**

| Tag | Meaning |
|---|---|
| `[b]…[/b]` | bold face |
| `[color=<role or #rrggbb>]…[/color]` | a theme colour role (`good`, `warn`, `bad`, `accent`, `text_dim`) or a literal colour |
| `[icon=<name>]` | an inline picture (T8) |
| `[br]` | line break |
| `[[` | a literal `[` |

An unknown or unclosed tag is drawn as written and logged once. Markdown was
considered and rejected for game text: it has no colour and no inline picture,
its `*`, `_` and `#` occur in ordinary strings, and its block structure
(headings, lists, links) is not something a tooltip or notice needs. Tags are
explicit and the set can grow by one row at a time.

## Icons

**T7. Two kinds of icon, two formats.**
- *Flat, single-colour icons of our own design* (command, status and
  navigation symbols of our own screens): authored as SVG in the repository,
  baked at build time by `sta_fontbake` into an SDF atlas, tinted by the
  theme, sharp at every UI scale.
- *The games' icons* (build buttons, resource icons, portraits, race
  emblems): stay raster, read from the install's SPR tables and textures
  through the mod chain (UI-27). They are not vectorised. A mod replaces them
  the way it replaces any texture, at whatever resolution it ships.

Race emblems and every other franchise symbol are never committed or
released, in any form: a redrawn SVG of an emblem is still the rights
holder's mark, not our work. Where our own screens show a race (lobby, race
picker), the emblem comes from the player's install at run time, and the
race's name is the fallback when the install has none.

SVG is a source format only. STA64 does not load or rasterise SVG at run time
in v1.

**T8. `[icon=<name>]` resolves through data.**
First the name of a picture glyph of the active mod's font (the words of
UI-16's glyph mapping, so `[icon=dilithium]` draws the same symbol a Roots
tooltip does), then the name of an icon in STA64's baked atlas. A name that
resolves to neither is drawn as the word.

## Not in v1

Markup in legacy string tables or in mod tooltips (see T5); a STA64-only
strings file for mods (string-table overlays stay excluded by
MODDING_DECISIONS); Markdown; tags for font size, links or images by path;
loading SVG at run time; vectorising or redrawing the games' icons and
emblems; converting a mod's sprite fonts or icons to another format. Items
that are postponed, not rejected, are under "Planned after v1".

## Planned after v1

Giving mods modern formats beside the legacy ones is a goal of the project,
not an exception to it. These are left out of v1 for order of work only, and
v1 must not be built in a way that blocks them:

- **Fonts supplied by a mod as TTF/OTF.** A mod names a font file in a
  STA64-only key that the original engines ignore (D3) and keeps its sprite
  fonts for them. STA64 bakes the face into an SDF atlas on the player's
  machine and uses it as the glyph source for that mod's text (T2), with
  the mod's picture glyphs still drawn from its sprite font (T3). What v1
  owes this: the glyph source interface of T2, and `sta_fontbake` usable as a
  library, not only as a build tool.
- **Markup in mod strings**, once a STA64-only strings file exists (open
  question 2).
- **Vector icons supplied by a mod**, baked the same way as a mod's font.

## Open questions for sign-off

1. **Sprite fonts at large UI scales.** How the originals scale their fonts
   with resolution is not measured. Until it is, sprite text is scaled as the
   stock HUD is (UI-18) and nothing is promised about sharpness at 4K.
2. **Lifting the string-table overlay exclusion.** Markup in mod strings
   needs a STA64-only strings file beside the legacy one. If that is wanted
   for the first release, that is a change to MODDING_DECISIONS'
   "Not in v1" list and goes through its own PR.

## Work packages

Rows are added to `docs/WORKPLAN.md` with the next free UI numbers once this
is accepted; none is claimed before that.

| Delivers | Decision | Builds on |
|---|---|---|
| SDF text material: the baked font's quads drawn, widgets moved off Bevy's text | T1 | UI-03 |
| Text-run model: spans, measure, wrap and draw over both glyph sources; legacy string parser with picture glyphs; SDF fallback | T2, T3, T4 | UI-16 |
| Markup parser and the Lua text call that opts in | T5, T6, T8 | the two above, LUA-09 |
| Icon bake from SVG sources into the SDF atlas; first icon set | T7 | UI-03 |

Accepted when: a Roots tooltip, a string with a character outside the sprite
font and a marked-up notice with a colour and `[icon=dilithium]` each lay out
through the same code path in a headless test; the widget gallery draws its
labels and icons from the SDF atlases at UI scales 1.0, 1.25 and 2.0.

## Changing these decisions

As in MODDING_DECISIONS: through a PR that edits this file, states which
decision and why, and is approved by both Tammy and Cedric. Numbers are never
reused.
