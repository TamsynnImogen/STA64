# ADR 0008: Stock HUD on windows wider or narrower than its layout sheet

- **Status:** proposed
- **Date:** 2026-11-14
- **Deciders:** Cedric decided H1 to H3 on 2026-11-14 (the stock HUD,
  `crates/sta64/src/ui/hud/`, has been worked by Cedric's sessions since
  Tammy handed it off on 2026-11-13). H4 to H6 add mod-facing files and need
  both Tammy and Cedric. Accepted when both approve the PR that merges this
  file; not binding before.
- **Relates to:** [ADR 0004](0004-modding-v1.md) D1, D3, D6, D7, D11; the UI
  text ADR (`0001-ui-text-and-icons.md`, to be renumbered 0007) open
  question 1; WORKPLAN UI-18, UI-29, UI-33

## Context

- The stock HUD ("stock" layout, as opposed to STA64's own HUD layout) draws
  a game's or mod's panels from its `gui_*.cfg` sheets. Each sheet places
  rects on a canvas (`screenWidth` x `screenHeight`; Armada I states none and
  its pieces tile 640 x 480). Evidence: read from data.
- Armada I, Armada II and STA2 Classic author one canvas, 4:3. Fleet
  Operations 3.2.7 and Roots author four sheets (`gui_4x3`, `gui_5x4`,
  `gui_16x10`, `gui_16x9`), the widest 16:9. No install has a sheet wider
  than 16:9. Evidence: read from data.
- STA64 picks the sheet whose canvas aspect is closest to the window's and
  then scales width and height separately, so the HUD is stretched whenever
  the window's aspect differs from the sheet's: Armada II at 16:9 and wider,
  everything at 21:9 and 32:9. Round things become ovals, the minimap is not
  square, text slots widen. The original engines stretch the same way;
  evidence: read from the Refit notes on the original (static), not measured
  here.
- Panel art is authored as groups. Pieces abut, and some carry lighting baked
  in from a neighbour (in Fleet Operations the selection panel lights the
  minimap frame). Moving panels apart, or filling the space between them with
  repeated art, changes how the HUD looks. Evidence: stated by Cedric from
  the art; which seams continue is not measured.
- No sheet in any install has an anchor, align or dock key. Evidence: read
  from data.

The forces:

- The stock layout exists to look like the game or mod being played.
- Players use 21:9 and 32:9 windows, and the 3D view should use the whole
  window.
- A mod author must be able to state a wide layout, and an adapted mod must
  still run on its original engine (D3).
- Mods that ship nothing new must keep working (D1).

## Decision

### Default behaviour

**H1. The stock HUD's appearance is not changed.**
STA64 scales a stock HUD by one factor for both axes. It never stretches one
axis, never cuts, redraws, repeats or fills panel art, and never moves panels
of one sheet relative to each other on its own judgment. Panels that are
grouped in the sheet stay grouped.

**H2. Closest sheet, clamped to its own aspect, centred.**
STA64 picks the sheet whose canvas aspect is closest to the window's (the
earliest candidate wins a tie, so a base sheet wins over a suffixed sheet
that declares the same canvas). It scales that canvas uniformly to the
window's height, or to the window's width when the window is narrower than
the sheet, and centres it. The HUD occupies only that centred area. A window
of the sheet's own aspect looks exactly as today.

**H3. The 3D view fills the whole window.**
The world is drawn behind and beside the HUD area. Only HUD art takes
pointer input; the margins beside the HUD area select, order and scroll as
the open view does. The minimap and every other consumer of HUD rects use the
one mapping of H2.

### What a mod can state (proposed, not in the first package)

**H4. A mod may ship a sheet for a wider aspect.**
A sheet named as the Fleet Operations sheets are (`gui_21x9.cfg`,
`gui_32x9.cfg`), with its own canvas, is a candidate under H2 like any other
and is resolved through the mod chain (D6, D11). The author places every
panel and may supply art made for that width. This is the way to change the
HUD's look on wide windows: it is the author's statement, not STA64's.

**H5. A mod may ship a hints file instead.**
`misc/gui_anchors.cfg`, in the `key = value` syntax of the gui sheets, names
panel-level keys and says where each goes when the window is wider or taller
than the chosen sheet: an edge or the centre per axis, or `attach <key>` to
keep a panel with another one. Panels attached to each other move as one
group. No original cfg includes this file. An optional
`misc/gui_<race>_anchors.cfg` overrides per race. Keys inside the existing
gui sheets are not used for this, because whether the original engines
ignore unknown keys there is not measured.

**H6. Order.**
The sheet is chosen by H2 among all candidates, H4's included. If the window
still differs from the chosen sheet's aspect and a hints file exists, panels
are placed by it across the whole window. Otherwise H2's centred area
applies. STA64 does not derive anchors from differences between a mod's
sheets or from where rects lie.

Done when (H1 to H3): layout tests pin, per profile, that every stock HUD
rect keeps its sheet aspect at 4:3, 16:10, 16:9, 21:9 and 32:9, that the HUD
area is centred, and that a window of the sheet's aspect gives the rects it
gives today; a pointer test pins that the margins reach the world.

## Alternatives considered

- **Keep the per-axis stretch.** Matches the original engines and needs no
  work, but distorts every HUD on the windows people use. Rejected as the
  default; see open question 1.
- **Anchor panels to the window's corners and edges, world between them.**
  Uses the whole width, but separates art that is authored as one piece
  (Armada I's bottom row, the Fleet Operations minimap and selection panel)
  and so changes the look (against H1). Kept only as something a mod or
  profile states (H5).
- **Fill the gaps by repeating slices of panel art.** No data says which
  slices tile, and it draws art the author did not make. Rejected.
- **Derive anchors from the differences between a mod's aspect sheets.** It
  reproduces the Fleet Operations sheets, but extrapolates an author's intent
  past the widest layout they made. Rejected in favour of H4 and H5.
- **Anchor keys inside the gui sheets.** Simplest for authors; depends on
  unmeasured behaviour of the original engines (D3). Rejected unless
  measured safe.

## Consequences

**Gained**

- No stock HUD is distorted at any window aspect, and none looks different
  from its sheet.
- The mapping is one scale and one offset, the same for every profile; no
  per-game cases.
- Mods that ship nothing keep working; authors who want a wide layout have
  two ways to state one, both invisible to the original engines.

**Accepted**

- On a window much wider than the closest sheet the HUD sits in the middle,
  with the minimap away from the corner: a 4:3 HUD on 21:9 covers a little
  over half the width.
- Wide layouts for the stock games exist only if someone authors them (open
  question 2).
- H4 and H5 need `check-pack` to list the new files and flag unknown keys
  (D7), which is CON-lane work.

**Work**

A UI-lane package for H1 to H3, added to WORKPLAN by its own PR. H4 costs
only the candidate list. H5 is a later package, after open question 3.

## Open questions

1. **Roll-out.** Whether the centred layout becomes the default at once and
   whether "stretched, as the original" stays as a setting.
2. **A wide layout for stock Armada II.** Cedric considers this defensible:
   minimap in the bottom-left corner, selection panel with the command bar
   in the bottom centre, video frame in the bottom-right corner. Whether
   STA64 offers it as an opt-in (a setting, or a hints file of key names
   shipped for the profile) is not decided.
3. **Do the originals ignore the new files?** That Fleet Operations 3.2.7
   and Roots ignore a sheet they do not name, and that every lineage ignores
   an extra file in `misc/`, is expected but not measured. H4 and H5 are not
   promised to authors before it is.
4. **HUD scale setting.** Range, and whether it is in the settings screen
   with the first package. It scales panels and their text together (UI text
   ADR, open question 1).

## Changing this ADR

Once accepted, H1 to H3 change through a PR approved by Cedric; H4 to H6
through one approved by both Tammy and Cedric, with a dated amendment note
under the decision.
